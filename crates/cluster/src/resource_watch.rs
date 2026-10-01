use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt::Debug;
use std::future;
use std::pin::Pin;
use std::time::Duration;

use futures::future::Either;
use futures::stream::{self, BoxStream, SelectAll, select_all};
use futures::{Stream, StreamExt};
use k8s_openapi::serde::de::DeserializeOwned;
use kube::ResourceExt;
use kube::runtime::WatchStreamExt;
use kube::runtime::watcher::{self, Event};
use tokio::time::Instant;

use crate::connection::{ClusterConnection, ClusterError, ScopedApi, classify_error};

/// Changes inside one window are merged into a single snapshot.
pub(crate) const BATCH_WINDOW: Duration = Duration::from_millis(100);

/// One item of a resource watch stream.
#[derive(Debug)]
pub enum WatchUpdate<T> {
    /// The complete current set, ordered by (namespace, name). Cluster-scoped kinds
    /// use the name only, which matches the `list_*` order.
    Snapshot(Vec<T>),
    /// The watch failed and is retrying with backoff. The last snapshot stays the
    /// best-known state. Never sent for an expected 410 relist.
    Failed(ClusterError),
}

/// Keeps a watch store to the `max_items` most recent summaries. `None` recency is oldest.
pub(crate) struct StoreLimit<T> {
    pub(crate) max_items: usize,
    pub(crate) recency: fn(&T) -> Option<jiff::Timestamp>,
}

// Manual: the derives would demand `T: Clone` and `T: Copy`, but only a `fn` pointer is held.
impl<T> Clone for StoreLimit<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for StoreLimit<T> {}

/// Watches `apis` and streams batched snapshots of `summarize`d objects. One api streams
/// as is; several are merged into one snapshot stream (see `merge_snapshots`). Nothing
/// happens until the stream is polled, and dropping it drops every HTTP watch.
pub(crate) fn summary_watch<K, T>(
    connection: &ClusterConnection,
    apis: Vec<ScopedApi<K>>,
    action: &'static str,
    summarize: fn(&K) -> T,
) -> impl Stream<Item = WatchUpdate<T>> + Send + 'static
where
    K: kube::Resource + Clone + DeserializeOwned + Debug + Send + 'static,
    T: Clone + PartialEq + Send + 'static,
{
    watch_apis(
        connection,
        apis,
        watcher::Config::default(),
        action,
        summarize,
        None,
    )
}

/// Like `summary_watch`, with a server-side `config` and a store that keeps only the
/// `limit.max_items` most recent summaries.
pub(crate) fn limited_summary_watch<K, T>(
    connection: &ClusterConnection,
    apis: Vec<ScopedApi<K>>,
    config: watcher::Config,
    action: &'static str,
    summarize: fn(&K) -> T,
    limit: StoreLimit<T>,
) -> impl Stream<Item = WatchUpdate<T>> + Send + 'static
where
    K: kube::Resource + Clone + DeserializeOwned + Debug + Send + 'static,
    T: Clone + PartialEq + Send + 'static,
{
    watch_apis(connection, apis, config, action, summarize, Some(limit))
}

fn watch_apis<K, T>(
    connection: &ClusterConnection,
    apis: Vec<ScopedApi<K>>,
    config: watcher::Config,
    action: &'static str,
    summarize: fn(&K) -> T,
    limit: Option<StoreLimit<T>>,
) -> impl Stream<Item = WatchUpdate<T>> + Send + 'static
where
    K: kube::Resource + Clone + DeserializeOwned + Debug + Send + 'static,
    T: Clone + PartialEq + Send + 'static,
{
    let mut watches: Vec<_> = apis
        .into_iter()
        .map(|(namespace, api)| {
            let events = watcher::watcher(api, config.clone()).default_backoff();
            let updates = batch_updates(
                events,
                connection.context().to_owned(),
                action,
                summarize,
                limit,
            );
            (namespace.unwrap_or_default(), updates.boxed())
        })
        .collect();
    if watches.len() == 1 {
        let (_, only) = watches.remove(0);
        return Either::Left(only);
    }
    Either::Right(merge_snapshots(watches, limit))
}

/// The testable core: any source of watcher events works, including fakes.
fn batch_updates<K, T, S>(
    events: S,
    context: String,
    action: &'static str,
    summarize: fn(&K) -> T,
    limit: Option<StoreLimit<T>>,
) -> impl Stream<Item = WatchUpdate<T>> + Send + 'static
where
    S: Stream<Item = Result<Event<K>, watcher::Error>> + Send + 'static,
    K: kube::Resource + Send + 'static,
    T: Clone + PartialEq + Send + 'static,
{
    let batcher = Batcher {
        events: Box::pin(events),
        context,
        action,
        summarize,
        store: SummaryStore::new(limit),
        is_synced: false,
        is_dirty: false,
        is_recovering: false,
        is_source_ended: false,
        deadline: None,
        outbox: VecDeque::new(),
    };
    futures::stream::unfold(batcher, |mut batcher| async move {
        let update = batcher.next_update().await?;
        Some((update, batcher))
    })
}

struct Batcher<K, T, S> {
    events: Pin<Box<S>>,
    context: String,
    action: &'static str,
    summarize: fn(&K) -> T,
    store: SummaryStore<T>,
    /// The first `InitDone` was seen. Nothing is emitted before it.
    is_synced: bool,
    /// The store changed since the last emitted snapshot.
    is_dirty: bool,
    /// A `Failed` was sent and no event has arrived since.
    is_recovering: bool,
    is_source_ended: bool,
    deadline: Option<Instant>,
    /// Updates produced by one step, such as a flush followed by `Failed`.
    outbox: VecDeque<WatchUpdate<T>>,
}

impl<K, T, S> Batcher<K, T, S>
where
    S: Stream<Item = Result<Event<K>, watcher::Error>>,
    K: kube::Resource,
    T: Clone + PartialEq,
{
    async fn next_update(&mut self) -> Option<WatchUpdate<T>> {
        loop {
            if let Some(update) = self.outbox.pop_front() {
                return Some(update);
            }
            if self.is_source_ended {
                return None;
            }
            // Cancel-safe: the deadline lives in `self`, and `next()` loses nothing when
            // its future is dropped. The timer is polled first so a busy source cannot
            // push a snapshot past its window.
            tokio::select! {
                biased;
                () = wait_until(self.deadline) => self.flush_snapshot(),
                item = self.events.next() => match item {
                    Some(Ok(event)) => self.handle_event(event),
                    Some(Err(error)) => self.handle_error(error),
                    None => self.handle_source_end(),
                },
            }
        }
    }

    fn handle_event(&mut self, event: Event<K>) {
        let has_changed = match event {
            // The watcher emits `Init` before every list attempt, including retries of a
            // failing one, so a list in progress must not clear the recovery state or
            // re-emit the old items. `InitDone` reports the change.
            Event::Init => {
                self.store.begin_init();
                return;
            }
            Event::InitApply(object) => {
                self.store
                    .init_apply(object_key(&object), (self.summarize)(&object));
                return;
            }
            Event::InitDone => {
                self.store.finish_init();
                self.is_synced = true;
                true
            }
            Event::Apply(object) => self
                .store
                .apply(object_key(&object), (self.summarize)(&object)),
            Event::Delete(object) => self.store.delete(&object_key(&object)),
        };
        if has_changed || self.is_recovering {
            self.is_dirty = true;
            self.is_recovering = false;
        }
        if self.is_synced && self.is_dirty && self.deadline.is_none() {
            self.deadline = Some(Instant::now() + BATCH_WINDOW);
        }
    }

    fn handle_error(&mut self, error: watcher::Error) {
        let Some(error) = watch_error(&self.context, self.action, error) else {
            return;
        };
        self.flush_pending_snapshot();
        self.outbox.push_back(WatchUpdate::Failed(error));
        self.is_recovering = true;
    }

    fn handle_source_end(&mut self) {
        self.flush_pending_snapshot();
        self.is_source_ended = true;
    }

    /// Emits the pending changes now. A partial initial list is never emitted.
    fn flush_pending_snapshot(&mut self) {
        if self.is_synced && self.is_dirty {
            self.flush_snapshot();
        }
        self.deadline = None;
    }

    fn flush_snapshot(&mut self) {
        self.outbox
            .push_back(WatchUpdate::Snapshot(self.store.snapshot()));
        self.is_dirty = false;
        self.deadline = None;
    }
}

/// Merges one snapshot stream per namespace into one, concatenating the latest snapshot of
/// each input in input order. `inputs` pair a namespace with its stream, in namespace order.
/// Coalesces like the Batcher: at most one merged snapshot per `BATCH_WINDOW`, and none until
/// every input has settled (a snapshot or a failure). A failing namespace never hides the
/// others' data: its failure is flushed after the pending merged snapshot and names it, and
/// is announced again after every later merged snapshot until its next snapshot clears it.
fn merge_snapshots<T: Clone + Send + 'static>(
    inputs: Vec<(String, BoxStream<'static, WatchUpdate<T>>)>,
    limit: Option<StoreLimit<T>>,
) -> impl Stream<Item = WatchUpdate<T>> + Send + 'static {
    let mut merge_inputs = Vec::new();
    let mut tagged = Vec::new();
    for (index, (namespace, updates)) in inputs.into_iter().enumerate() {
        merge_inputs.push(MergeInput {
            namespace,
            state: InputState::Waiting,
            failure: None,
        });
        // The end marker lets an input that ends while `Waiting` settle instead of stalling
        // the merge.
        let events = updates
            .map(MergeEvent::Update)
            .chain(stream::once(future::ready(MergeEvent::Ended)));
        tagged.push(events.map(move |event| (index, event)).boxed());
    }
    let merger = Merger {
        events: select_all(tagged),
        inputs: merge_inputs,
        limit,
        is_dirty: false,
        is_ended: false,
        deadline: None,
        outbox: VecDeque::new(),
    };
    futures::stream::unfold(merger, |mut merger| async move {
        let update = merger.next_update().await?;
        Some((update, merger))
    })
}

enum MergeEvent<T> {
    Update(WatchUpdate<T>),
    /// The input stream finished.
    Ended,
}

enum InputState<T> {
    /// Nothing yet.
    Waiting,
    /// The latest snapshot. Kept stale across a failure.
    Items(Vec<T>),
    /// Failed before any snapshot.
    FailedEmpty,
}

struct MergeInput<T> {
    namespace: String,
    state: InputState<T>,
    /// The last failure while the input is unresolved, rendered because `ClusterError` is not
    /// `Clone`. It is the same text the first announcement shows. The next snapshot clears it.
    failure: Option<String>,
}

// ponytail: every input keeps its own latest snapshot, so before the merge trims, the
// retained summaries reach N namespaces x `limit.max_items`; one shared store if that bites.
struct Merger<T> {
    events: SelectAll<BoxStream<'static, (usize, MergeEvent<T>)>>,
    inputs: Vec<MergeInput<T>>,
    limit: Option<StoreLimit<T>>,
    /// A snapshot arrived since the last emitted merge.
    is_dirty: bool,
    is_ended: bool,
    deadline: Option<Instant>,
    /// Updates produced by one step: a flush followed by failures.
    outbox: VecDeque<WatchUpdate<T>>,
}

impl<T: Clone> Merger<T> {
    async fn next_update(&mut self) -> Option<WatchUpdate<T>> {
        loop {
            if let Some(update) = self.outbox.pop_front() {
                return Some(update);
            }
            if self.is_ended {
                return None;
            }
            // Cancel-safe like the Batcher: the deadline lives in `self`. The timer is
            // polled first so a busy input cannot push a snapshot past its window.
            tokio::select! {
                biased;
                () = wait_until(self.deadline) => self.flush_if_ready(),
                item = self.events.next() => match item {
                    Some((index, MergeEvent::Update(WatchUpdate::Snapshot(items)))) => {
                        self.handle_snapshot(index, items);
                    }
                    Some((index, MergeEvent::Update(WatchUpdate::Failed(error)))) => {
                        self.handle_failure(index, error);
                    }
                    Some((index, MergeEvent::Ended)) => self.handle_end(index),
                    None => {
                        self.flush_if_ready();
                        self.is_ended = true;
                    }
                },
            }
        }
    }

    fn handle_snapshot(&mut self, index: usize, items: Vec<T>) {
        let input = &mut self.inputs[index];
        input.state = InputState::Items(items);
        input.failure = None;
        self.is_dirty = true;
        if self.deadline.is_none() {
            self.deadline = Some(Instant::now() + BATCH_WINDOW);
        }
    }

    fn handle_failure(&mut self, index: usize, error: ClusterError) {
        let input = &mut self.inputs[index];
        if matches!(input.state, InputState::Waiting) {
            input.state = InputState::FailedEmpty;
        }
        // Cleared first so the flush does not repeat the older failure of this input.
        input.failure = None;
        self.flush_if_ready();
        let input = &mut self.inputs[index];
        input.failure = Some(error.to_string());
        self.outbox
            .push_back(WatchUpdate::Failed(ClusterError::Namespace {
                namespace: input.namespace.clone(),
                source: Box::new(error),
            }));
    }

    /// An input that ends before its first snapshot would keep every merge waiting.
    fn handle_end(&mut self, index: usize) {
        if !matches!(self.inputs[index].state, InputState::Waiting) {
            return;
        }
        self.handle_failure(
            index,
            ClusterError::Rendered {
                message: "the watch ended before its first snapshot".to_owned(),
            },
        );
    }

    /// Emits the merged snapshot when something changed, every input has settled, and at
    /// least one holds a snapshot, then repeats the failures still unresolved. Otherwise the
    /// change stays pending for a later input.
    fn flush_if_ready(&mut self) {
        self.deadline = None;
        let is_settled = self
            .inputs
            .iter()
            .all(|input| !matches!(input.state, InputState::Waiting));
        let has_items = self
            .inputs
            .iter()
            .any(|input| matches!(input.state, InputState::Items(_)));
        if !(self.is_dirty && is_settled && has_items) {
            return;
        }
        let mut merged: Vec<T> = self
            .inputs
            .iter()
            .filter_map(|input| match &input.state {
                InputState::Items(items) => Some(items),
                _ => None,
            })
            .flatten()
            .cloned()
            .collect();
        if let Some(limit) = &self.limit {
            keep_newest(&mut merged, limit);
        }
        self.outbox.push_back(WatchUpdate::Snapshot(merged));
        self.is_dirty = false;
        let repeats = self.inputs.iter().filter_map(|input| {
            let message = input.failure.clone()?;
            Some(WatchUpdate::Failed(ClusterError::Namespace {
                namespace: input.namespace.clone(),
                source: Box::new(ClusterError::Rendered { message }),
            }))
        });
        self.outbox.extend(repeats);
    }
}

/// Drops the oldest items beyond `max_items`, keeping the rest in order. Ties drop the
/// earlier item, like `trim`.
fn keep_newest<T>(items: &mut Vec<T>, limit: &StoreLimit<T>) {
    let excess = items.len().saturating_sub(limit.max_items);
    if excess == 0 {
        return;
    }
    let mut ranked: Vec<_> = items
        .iter()
        .enumerate()
        .map(|(index, item)| ((limit.recency)(item), index))
        .collect();
    ranked.sort();
    let dropped: HashSet<usize> = ranked
        .into_iter()
        .take(excess)
        .map(|(_, index)| index)
        .collect();
    let mut index = 0;
    items.retain(|_| {
        let is_kept = !dropped.contains(&index);
        index += 1;
        is_kept
    });
}

pub(crate) async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => future::pending().await,
    }
}

/// Maps a watcher error. `None` is an expected 410: the watcher relists by itself.
fn watch_error(context: &str, action: &'static str, error: watcher::Error) -> Option<ClusterError> {
    let error = match error {
        watcher::Error::InitialListFailed(source)
        | watcher::Error::WatchStartFailed(source)
        | watcher::Error::WatchFailed(source) => classify_error(context, action, source),
        watcher::Error::WatchError(status) => {
            classify_error(context, action, kube::Error::Api(status))
        }
        watcher::Error::NoResourceVersion => ClusterError::UnexpectedResponse {
            context: context.to_owned(),
            action,
            source: Box::new(error),
        },
    };
    if matches!(error, ClusterError::Api { code: 410, .. }) {
        tracing::debug!(context, action, "watch resource version expired; relisting");
        return None;
    }
    Some(error)
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ObjectKey {
    /// Empty for cluster-scoped objects.
    namespace: String,
    name: String,
}

fn object_key<K: kube::Resource>(object: &K) -> ObjectKey {
    ObjectKey {
        namespace: object.namespace().unwrap_or_default(),
        name: object.name_any(),
    }
}

/// Summaries only, never raw objects: that is the memory bound.
struct SummaryStore<T> {
    items: BTreeMap<ObjectKey, T>,
    /// The relist in progress. `items` stay visible until `finish_init`.
    pending_init: Option<BTreeMap<ObjectKey, T>>,
    limit: Option<StoreLimit<T>>,
}

impl<T: Clone + PartialEq> SummaryStore<T> {
    fn new(limit: Option<StoreLimit<T>>) -> Self {
        Self {
            items: BTreeMap::new(),
            pending_init: None,
            limit,
        }
    }

    fn begin_init(&mut self) {
        self.pending_init = Some(BTreeMap::new());
    }

    fn init_apply(&mut self, key: ObjectKey, summary: T) {
        let buffer = self.pending_init.get_or_insert_with(BTreeMap::new);
        buffer.insert(key, summary);
        // Trimming only at twice the limit keeps the relist cost amortized while the
        // buffer stays bounded.
        if let Some(limit) = &self.limit
            && buffer.len() >= limit.max_items.saturating_mul(2)
        {
            trim(buffer, limit);
        }
    }

    /// Swaps the relist in. Objects that vanished while disconnected are dropped.
    fn finish_init(&mut self) {
        self.items = self.pending_init.take().unwrap_or_default();
        if let Some(limit) = &self.limit {
            trim(&mut self.items, limit);
        }
    }

    /// Returns whether the object was absent or different. An object older than all the
    /// retained ones is evicted at once, which is not a change.
    fn apply(&mut self, key: ObjectKey, summary: T) -> bool {
        let previous = self.items.insert(key.clone(), summary);
        if let Some(limit) = &self.limit {
            trim(&mut self.items, limit);
        }
        previous.as_ref() != self.items.get(&key)
    }

    /// Returns whether the object was present.
    fn delete(&mut self, key: &ObjectKey) -> bool {
        self.items.remove(key).is_some()
    }

    fn snapshot(&self) -> Vec<T> {
        self.items.values().cloned().collect()
    }
}

/// Drops the oldest items beyond `max_items`. Ties fall to key order, so it is
/// deterministic.
// ponytail: a bulk trim sorts the whole map, O(n log n) at n = 2,000; a min-heap if a
// profile shows it.
fn trim<T>(map: &mut BTreeMap<ObjectKey, T>, limit: &StoreLimit<T>) {
    let excess = map.len().saturating_sub(limit.max_items);
    if excess == 0 {
        return;
    }
    if excess == 1 {
        // Steady state: one apply at the cap evicts one item, so skip the sort.
        let oldest = map
            .iter()
            .min_by_key(|(key, item)| ((limit.recency)(item), *key))
            .map(|(key, _)| key.clone());
        if let Some(key) = oldest {
            map.remove(&key);
        }
        return;
    }
    let mut ranked: Vec<_> = map
        .iter()
        .map(|(key, item)| ((limit.recency)(item), key.clone()))
        .collect();
    ranked.sort();
    for (_, key) in ranked.into_iter().take(excess) {
        map.remove(&key);
    }
}

#[cfg(test)]
#[path = "resource_watch_tests.rs"]
mod resource_watch_tests;
