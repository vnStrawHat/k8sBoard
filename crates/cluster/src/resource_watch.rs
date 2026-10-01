use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::future;
use std::pin::Pin;
use std::time::Duration;

use futures::{Stream, StreamExt};
use k8s_openapi::serde::de::DeserializeOwned;
use kube::runtime::WatchStreamExt;
use kube::runtime::watcher::{self, Event};
use kube::{Api, ResourceExt};
use tokio::time::Instant;

use crate::connection::{ClusterConnection, ClusterError, classify_error};

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

/// Watches `api` and streams batched snapshots of `summarize`d objects. Nothing happens
/// until the stream is polled, and dropping it drops the HTTP watch.
pub(crate) fn summary_watch<K, T>(
    connection: &ClusterConnection,
    api: Api<K>,
    action: &'static str,
    summarize: fn(&K) -> T,
) -> impl Stream<Item = WatchUpdate<T>> + Send + 'static
where
    K: kube::Resource + Clone + DeserializeOwned + Debug + Send + 'static,
    T: Clone + PartialEq + Send + 'static,
{
    let events = watcher::watcher(api, watcher::Config::default()).default_backoff();
    batch_updates(events, connection.context().to_owned(), action, summarize)
}

/// The testable core: any source of watcher events works, including fakes.
fn batch_updates<K, T, S>(
    events: S,
    context: String,
    action: &'static str,
    summarize: fn(&K) -> T,
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
        store: SummaryStore::default(),
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
}

impl<T> Default for SummaryStore<T> {
    fn default() -> Self {
        Self {
            items: BTreeMap::new(),
            pending_init: None,
        }
    }
}

impl<T: Clone + PartialEq> SummaryStore<T> {
    fn begin_init(&mut self) {
        self.pending_init = Some(BTreeMap::new());
    }

    fn init_apply(&mut self, key: ObjectKey, summary: T) {
        self.pending_init
            .get_or_insert_with(BTreeMap::new)
            .insert(key, summary);
    }

    /// Swaps the relist in. Objects that vanished while disconnected are dropped.
    fn finish_init(&mut self) {
        self.items = self.pending_init.take().unwrap_or_default();
    }

    /// Returns whether the object was absent or different.
    fn apply(&mut self, key: ObjectKey, summary: T) -> bool {
        self.items.insert(key, summary.clone()).as_ref() != Some(&summary)
    }

    /// Returns whether the object was present.
    fn delete(&mut self, key: &ObjectKey) -> bool {
        self.items.remove(key).is_some()
    }

    fn snapshot(&self) -> Vec<T> {
        self.items.values().cloned().collect()
    }
}

#[cfg(test)]
#[path = "resource_watch_tests.rs"]
mod resource_watch_tests;
