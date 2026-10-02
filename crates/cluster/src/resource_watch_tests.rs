use std::pin::pin;

use futures::channel::mpsc::{self, UnboundedSender};
use futures::stream;
use k8s_openapi::api::core::v1::{Pod, PodSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
use kube::core::{PartialObjectMeta, PartialObjectMetaExt, Status};

use super::*;
use crate::pod::{PodSummary, pod_summary};

type FakeItem = Result<Event<Pod>, watcher::Error>;
type FakeSender = UnboundedSender<FakeItem>;

const ACTION: &str = "watching pods";

fn pod(namespace: &str, name: &str) -> Pod {
    Pod {
        metadata: ObjectMeta {
            namespace: Some(namespace.to_owned()),
            name: Some(name.to_owned()),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn pod_on_node(namespace: &str, name: &str, node: &str) -> Pod {
    let mut pod = pod(namespace, name);
    pod.spec = Some(PodSpec {
        node_name: Some(node.to_owned()),
        ..Default::default()
    });
    pod
}

fn key(namespace: &str, name: &str) -> ObjectKey {
    ObjectKey {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
    }
}

fn status_error(code: u16) -> watcher::Error {
    watcher::Error::WatchError(Box::new(Status {
        code,
        message: "scripted failure".to_owned(),
        ..Default::default()
    }))
}

fn api_error(code: u16) -> kube::Error {
    kube::Error::Api(Box::new(Status {
        code,
        message: "scripted failure".to_owned(),
        ..Default::default()
    }))
}

fn watch_of(
    source: impl Stream<Item = FakeItem> + Send + 'static,
) -> impl Stream<Item = WatchUpdate<PodSummary>> {
    batch_updates(source, "test".to_owned(), ACTION, pod_summary, None)
}

fn channel() -> (FakeSender, mpsc::UnboundedReceiver<FakeItem>) {
    mpsc::unbounded()
}

fn send(sender: &FakeSender, items: impl IntoIterator<Item = FakeItem>) {
    for item in items {
        sender.unbounded_send(item).expect("receiver is alive");
    }
}

/// Init, one `InitApply` per pod, `InitDone`.
fn initial_list(pods: &[Pod]) -> Vec<FakeItem> {
    let mut items = vec![Ok(Event::Init)];
    items.extend(pods.iter().cloned().map(|pod| Ok(Event::InitApply(pod))));
    items.push(Ok(Event::InitDone));
    items
}

/// The next update, or `None` when the stream stays silent for a long (virtual) time.
async fn next_update<S>(stream: &mut S) -> Option<WatchUpdate<PodSummary>>
where
    S: Stream<Item = WatchUpdate<PodSummary>> + Unpin,
{
    tokio::time::timeout(Duration::from_secs(10), stream.next())
        .await
        .ok()
        .flatten()
}

fn names(update: &WatchUpdate<PodSummary>) -> Vec<String> {
    match update {
        WatchUpdate::Snapshot(pods) => pods
            .iter()
            .map(|pod| format!("{}/{}", pod.namespace, pod.name))
            .collect(),
        WatchUpdate::Failed(error) => panic!("expected a snapshot, got failure: {error}"),
    }
}

// Store

#[test]
fn store_snapshot_is_ordered_by_namespace_then_name() {
    let mut store = SummaryStore::new(None);
    for (namespace, name) in [("b", "a"), ("a", "z"), ("a", "b")] {
        store.apply(key(namespace, name), pod_summary(&pod(namespace, name)));
    }
    let ordered: Vec<_> = store
        .snapshot()
        .into_iter()
        .map(|pod| (pod.namespace, pod.name))
        .collect();
    let expected = [("a", "b"), ("a", "z"), ("b", "a")]
        .map(|(namespace, name)| (namespace.to_owned(), name.to_owned()));
    assert_eq!(ordered, expected);
}

#[test]
fn store_apply_of_equal_summary_reports_unchanged() {
    let mut store = SummaryStore::new(None);
    let summary = pod_summary(&pod("ns", "a"));
    assert!(store.apply(key("ns", "a"), summary.clone()));
    assert!(!store.apply(key("ns", "a"), summary));
    assert!(store.apply(key("ns", "a"), pod_summary(&pod_on_node("ns", "a", "n1"))));
}

#[test]
fn store_delete_of_missing_key_reports_unchanged() {
    let mut store = SummaryStore::new(None);
    assert!(!store.delete(&key("ns", "a")));
    store.apply(key("ns", "a"), pod_summary(&pod("ns", "a")));
    assert!(store.delete(&key("ns", "a")));
}

#[test]
fn store_keeps_old_items_visible_during_init() {
    let mut store = SummaryStore::new(None);
    store.apply(key("ns", "old"), pod_summary(&pod("ns", "old")));
    store.begin_init();
    store.init_apply(key("ns", "new"), pod_summary(&pod("ns", "new")));
    let visible: Vec<_> = store.snapshot().into_iter().map(|pod| pod.name).collect();
    assert_eq!(visible, ["old"]);
}

#[test]
fn store_finish_init_replaces_items_and_drops_vanished() {
    let mut store = SummaryStore::new(None);
    store.apply(key("ns", "old"), pod_summary(&pod("ns", "old")));
    store.begin_init();
    store.init_apply(key("ns", "new"), pod_summary(&pod("ns", "new")));
    store.finish_init();
    let visible: Vec<_> = store.snapshot().into_iter().map(|pod| pod.name).collect();
    assert_eq!(visible, ["new"]);
}

// Batching

#[tokio::test(start_paused = true)]
async fn no_snapshot_before_init_done() {
    let (sender, receiver) = channel();
    send(
        &sender,
        [Ok(Event::Init), Ok(Event::InitApply(pod("ns", "a")))],
    );
    let mut stream = pin!(watch_of(receiver));
    assert!(next_update(&mut stream).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn init_done_emits_one_snapshot_with_all_items() {
    let source = stream::iter(initial_list(&[pod("ns", "b"), pod("ns", "a")]));
    let updates: Vec<_> = watch_of(source).collect().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(names(&updates[0]), ["ns/a", "ns/b"]);
}

#[tokio::test(start_paused = true)]
async fn empty_initial_list_emits_empty_snapshot() {
    let updates: Vec<_> = watch_of(stream::iter(initial_list(&[]))).collect().await;
    assert_eq!(updates.len(), 1);
    assert!(names(&updates[0]).is_empty());
}

#[tokio::test(start_paused = true)]
async fn burst_within_window_emits_one_snapshot() {
    let (sender, receiver) = channel();
    send(&sender, initial_list(&[]));
    send(
        &sender,
        (0..50).map(|index| Ok(Event::Apply(pod("ns", &format!("pod-{index:02}"))))),
    );
    let mut stream = pin!(watch_of(receiver));
    let update = next_update(&mut stream).await.expect("a snapshot");
    assert_eq!(names(&update).len(), 50);
    assert!(next_update(&mut stream).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn changes_in_separate_windows_emit_separate_snapshots() {
    let (sender, receiver) = channel();
    send(&sender, initial_list(&[]));
    send(&sender, [Ok(Event::Apply(pod("ns", "a")))]);
    let mut stream = pin!(watch_of(receiver));
    let first = next_update(&mut stream).await.expect("first snapshot");
    assert_eq!(names(&first), ["ns/a"]);

    tokio::time::advance(Duration::from_millis(150)).await;
    send(&sender, [Ok(Event::Apply(pod("ns", "b")))]);
    let second = next_update(&mut stream).await.expect("second snapshot");
    assert_eq!(names(&second), ["ns/a", "ns/b"]);
}

#[tokio::test(start_paused = true)]
async fn unchanged_apply_emits_nothing() {
    let (sender, receiver) = channel();
    send(&sender, initial_list(&[pod("ns", "a")]));
    let mut stream = pin!(watch_of(receiver));
    assert!(next_update(&mut stream).await.is_some());

    send(&sender, [Ok(Event::Apply(pod("ns", "a")))]);
    assert!(next_update(&mut stream).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn gone_410_is_silent_and_relist_replaces_items() {
    let (sender, receiver) = channel();
    send(&sender, initial_list(&[pod("ns", "a"), pod("ns", "b")]));
    let mut stream = pin!(watch_of(receiver));
    let first = next_update(&mut stream).await.expect("first snapshot");
    assert_eq!(names(&first), ["ns/a", "ns/b"]);

    send(&sender, [Err(status_error(410))]);
    send(&sender, initial_list(&[pod("ns", "a")]));
    let second = next_update(&mut stream).await.expect("relist snapshot");
    assert_eq!(names(&second), ["ns/a"]);
}

#[tokio::test(start_paused = true)]
async fn failure_flushes_pending_snapshot_before_failed() {
    let (sender, receiver) = channel();
    send(&sender, initial_list(&[]));
    send(
        &sender,
        [Ok(Event::Apply(pod("ns", "a"))), Err(status_error(500))],
    );
    let mut stream = pin!(watch_of(receiver));
    let snapshot = next_update(&mut stream).await.expect("snapshot");
    assert_eq!(names(&snapshot), ["ns/a"]);
    let failed = next_update(&mut stream).await.expect("failure");
    assert!(matches!(failed, WatchUpdate::Failed(_)));
}

#[tokio::test(start_paused = true)]
async fn failure_during_initial_list_emits_failed_without_snapshot() {
    let (sender, receiver) = channel();
    send(
        &sender,
        [
            Ok(Event::Init),
            Ok(Event::InitApply(pod("ns", "a"))),
            Err(status_error(403)),
        ],
    );
    let mut stream = pin!(watch_of(receiver));
    let failed = next_update(&mut stream).await.expect("failure");
    assert!(matches!(failed, WatchUpdate::Failed(_)));
    assert!(next_update(&mut stream).await.is_none());

    send(&sender, initial_list(&[pod("ns", "a")]));
    let snapshot = next_update(&mut stream).await.expect("full snapshot");
    assert_eq!(names(&snapshot), ["ns/a"]);
}

#[tokio::test(start_paused = true)]
async fn forbidden_maps_to_cluster_error_forbidden() {
    let (sender, receiver) = channel();
    send(&sender, [Err(status_error(403))]);
    let mut stream = pin!(watch_of(receiver));
    let update = next_update(&mut stream).await.expect("failure");
    assert!(matches!(
        update,
        WatchUpdate::Failed(ClusterError::Forbidden {
            action: "watching pods",
            ..
        })
    ));
}

#[tokio::test(start_paused = true)]
async fn event_after_failure_reemits_snapshot_even_if_unchanged() {
    let (sender, receiver) = channel();
    send(&sender, initial_list(&[pod("ns", "a")]));
    let mut stream = pin!(watch_of(receiver));
    assert!(next_update(&mut stream).await.is_some());

    send(&sender, [Err(status_error(500))]);
    let failed = next_update(&mut stream).await.expect("failure");
    assert!(matches!(failed, WatchUpdate::Failed(_)));

    send(&sender, [Ok(Event::Apply(pod("ns", "a")))]);
    let snapshot = next_update(&mut stream).await.expect("recovery snapshot");
    assert_eq!(names(&snapshot), ["ns/a"]);
}

#[tokio::test(start_paused = true)]
async fn source_end_flushes_pending_changes() {
    let mut items = initial_list(&[]);
    items.push(Ok(Event::Apply(pod("ns", "a"))));
    let updates: Vec<_> = watch_of(stream::iter(items)).collect().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(names(&updates[0]), ["ns/a"]);
}

#[tokio::test(start_paused = true)]
async fn dropping_stream_drops_source() {
    let (sender, receiver) = channel();
    send(&sender, initial_list(&[]));
    let mut stream = Box::pin(watch_of(receiver));
    assert!(next_update(&mut stream).await.is_some());
    assert!(!sender.is_closed());

    drop(stream);
    assert!(sender.is_closed());
}

// watch_error

#[test]
fn watch_error_watch_410_is_none() {
    assert!(watch_error("test", ACTION, status_error(410)).is_none());
}

#[test]
fn watch_error_initial_list_410_is_none() {
    let error = watcher::Error::InitialListFailed(api_error(410));
    assert!(watch_error("test", ACTION, error).is_none());
}

#[test]
fn watch_error_no_resource_version_is_unexpected_response() {
    let error = watch_error("test", ACTION, watcher::Error::NoResourceVersion);
    assert!(matches!(
        error,
        Some(ClusterError::UnexpectedResponse { action: ACTION, .. })
    ));
}

#[tokio::test(start_paused = true)]
async fn retry_failure_after_sync_emits_failed_without_snapshot() {
    let (sender, receiver) = channel();
    send(&sender, initial_list(&[pod("ns", "a")]));
    let mut stream = pin!(watch_of(receiver));
    assert!(next_update(&mut stream).await.is_some());

    send(&sender, [Err(status_error(500))]);
    let first_failure = next_update(&mut stream).await.expect("first failure");
    assert!(matches!(first_failure, WatchUpdate::Failed(_)));

    send(&sender, [Ok(Event::Init), Err(status_error(500))]);
    let retry_failure = next_update(&mut stream).await.expect("retry failure");
    assert!(matches!(retry_failure, WatchUpdate::Failed(_)));
}

#[tokio::test(start_paused = true)]
async fn delete_event_emits_snapshot() {
    let (sender, receiver) = channel();
    send(&sender, initial_list(&[pod("ns", "a"), pod("ns", "b")]));
    let mut stream = pin!(watch_of(receiver));
    assert!(next_update(&mut stream).await.is_some());

    send(&sender, [Ok(Event::Delete(pod("ns", "a")))]);
    let snapshot = next_update(&mut stream).await.expect("snapshot");
    assert_eq!(names(&snapshot), ["ns/b"]);
}

#[test]
fn watch_error_maps_watch_start_and_watch_failed() {
    let start = watch_error(
        "test",
        ACTION,
        watcher::Error::WatchStartFailed(api_error(403)),
    );
    assert!(matches!(start, Some(ClusterError::Forbidden { .. })));
    let stream = watch_error("test", ACTION, watcher::Error::WatchFailed(api_error(401)));
    assert!(matches!(stream, Some(ClusterError::Unauthorized { .. })));
}

// Store limit

/// An item with an id and a last-seen time in seconds; `None` means unknown.
type Stamped = (u32, Option<i64>);

fn stamped_limit(max_items: usize) -> StoreLimit<Stamped> {
    StoreLimit {
        max_items,
        recency: |(_, seconds)| {
            seconds.and_then(|seconds| jiff::Timestamp::from_second(seconds).ok())
        },
    }
}

fn limited_store(max_items: usize) -> SummaryStore<Stamped> {
    SummaryStore::new(Some(stamped_limit(max_items)))
}

fn stamped_key(id: u32) -> ObjectKey {
    key("ns", &format!("item-{id:03}"))
}

fn apply_stamped(store: &mut SummaryStore<Stamped>, id: u32, seconds: Option<i64>) -> bool {
    store.apply(stamped_key(id), (id, seconds))
}

fn kept_ids(store: &SummaryStore<Stamped>) -> Vec<u32> {
    store.snapshot().into_iter().map(|(id, _)| id).collect()
}

#[test]
fn store_limit_evicts_oldest_on_apply() {
    let mut store = limited_store(3);
    for (id, seconds) in [(1, 40), (2, 10), (3, 30), (4, 20)] {
        apply_stamped(&mut store, id, Some(seconds));
    }
    assert_eq!(kept_ids(&store), [1, 3, 4]);
}

#[test]
fn store_limit_unknown_recency_is_evicted_first() {
    let mut store = limited_store(2);
    apply_stamped(&mut store, 1, None);
    apply_stamped(&mut store, 2, Some(1));
    apply_stamped(&mut store, 3, Some(2));
    assert_eq!(kept_ids(&store), [2, 3]);
}

#[test]
fn store_limit_new_item_older_than_all_is_not_a_change() {
    let mut store = limited_store(2);
    apply_stamped(&mut store, 1, Some(10));
    apply_stamped(&mut store, 2, Some(20));
    assert!(!apply_stamped(&mut store, 3, Some(5)));
    assert_eq!(kept_ids(&store), [1, 2]);
}

#[test]
fn store_limit_ties_fall_to_key_order() {
    let mut store = limited_store(2);
    for id in [3, 1, 2] {
        apply_stamped(&mut store, id, Some(10));
    }
    assert_eq!(kept_ids(&store), [2, 3]);
}

#[test]
fn store_limit_keeps_newest_after_relist() {
    let mut store = limited_store(3);
    store.begin_init();
    for (id, seconds) in [(1, 50), (2, 10), (3, 40), (4, 20), (5, 30)] {
        store.init_apply(stamped_key(id), (id, Some(seconds)));
    }
    store.finish_init();
    assert_eq!(kept_ids(&store), [1, 3, 5]);
}

#[test]
fn store_limit_trims_relist_buffer_at_twice_the_limit() {
    let mut store = limited_store(2);
    store.begin_init();
    let buffered = |store: &SummaryStore<Stamped>| store.pending_init.as_ref().map(BTreeMap::len);
    for id in 1..=3 {
        store.init_apply(stamped_key(id), (id, Some(i64::from(id))));
    }
    assert_eq!(buffered(&store), Some(3));
    store.init_apply(stamped_key(4), (4, Some(4)));
    assert_eq!(buffered(&store), Some(2));
}

#[tokio::test(start_paused = true)]
async fn limited_watch_snapshot_holds_at_most_limit() {
    let stamped = |name: &str, seconds: i64| {
        let mut pod = pod("ns", name);
        let created = jiff::Timestamp::from_second(seconds).expect("valid timestamp");
        pod.metadata.creation_timestamp = Some(Time(created));
        pod
    };
    let limit = StoreLimit {
        max_items: 3,
        recency: |pod: &PodSummary| pod.created_at,
    };
    let (sender, receiver) = channel();
    send(
        &sender,
        initial_list(&[
            stamped("a", 10),
            stamped("b", 50),
            stamped("c", 20),
            stamped("d", 40),
            stamped("e", 30),
        ]),
    );
    send(&sender, [Ok(Event::Apply(stamped("f", 60)))]);
    let source = receiver;
    let mut stream = pin!(batch_updates(
        source,
        "test".to_owned(),
        ACTION,
        pod_summary,
        Some(limit),
    ));
    let first = next_update(&mut stream).await.expect("a snapshot");
    assert_eq!(names(&first), ["ns/b", "ns/d", "ns/f"]);
    assert!(next_update(&mut stream).await.is_none());
}

// Merging

fn merge_input(
    namespace: &str,
    updates: impl Stream<Item = WatchUpdate<u32>> + Send + 'static,
) -> (String, BoxStream<'static, WatchUpdate<u32>>) {
    (namespace.to_owned(), updates.boxed())
}

/// One live input per namespace: the senders feed the merge.
fn live_merge(
    namespaces: &[&str],
    limit: Option<StoreLimit<u32>>,
) -> (
    Vec<UnboundedSender<WatchUpdate<u32>>>,
    impl Stream<Item = WatchUpdate<u32>> + Unpin,
) {
    let mut senders = Vec::new();
    let mut inputs = Vec::new();
    for namespace in namespaces {
        let (sender, receiver) = mpsc::unbounded();
        senders.push(sender);
        inputs.push(merge_input(namespace, receiver));
    }
    (senders, Box::pin(merge_snapshots(inputs, limit)))
}

fn feed(sender: &UnboundedSender<WatchUpdate<u32>>, update: WatchUpdate<u32>) {
    sender.unbounded_send(update).expect("receiver is alive");
}

async fn next_merged<S>(stream: &mut S) -> Option<WatchUpdate<u32>>
where
    S: Stream<Item = WatchUpdate<u32>> + Unpin,
{
    tokio::time::timeout(Duration::from_secs(10), stream.next())
        .await
        .ok()
        .flatten()
}

/// Lets the merge take in what was fed so far without outlasting its window, so the next
/// feed arrives in a known order.
async fn absorb<S>(stream: &mut S)
where
    S: Stream<Item = WatchUpdate<u32>> + Unpin,
{
    let early = tokio::time::timeout(Duration::from_millis(10), stream.next()).await;
    assert!(early.is_err(), "nothing is due inside the window");
}

fn merged_items(update: Option<WatchUpdate<u32>>) -> Vec<u32> {
    match update {
        Some(WatchUpdate::Snapshot(items)) => items,
        other => panic!("expected a snapshot, got {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn merge_waits_for_every_input() {
    let (senders, mut merged) = live_merge(&["a", "b"], None);
    feed(&senders[0], WatchUpdate::Snapshot(vec![1]));
    assert!(next_merged(&mut merged).await.is_none());

    feed(&senders[1], WatchUpdate::Snapshot(vec![2]));
    assert_eq!(merged_items(next_merged(&mut merged).await), [1, 2]);
}

#[tokio::test(start_paused = true)]
async fn merge_concatenates_in_input_order() {
    let inputs = vec![
        merge_input("a", stream::iter([WatchUpdate::Snapshot(vec![1, 2])])),
        merge_input("b", stream::iter([WatchUpdate::Snapshot(vec![3])])),
    ];
    let updates: Vec<_> = merge_snapshots(inputs, None).collect().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(merged_items(updates.into_iter().next()), [1, 2, 3]);
}

#[tokio::test(start_paused = true)]
async fn merge_reemits_with_latest_of_each_input() {
    let (senders, mut merged) = live_merge(&["a", "b"], None);
    feed(&senders[0], WatchUpdate::Snapshot(vec![1]));
    feed(&senders[1], WatchUpdate::Snapshot(vec![2]));
    assert_eq!(merged_items(next_merged(&mut merged).await), [1, 2]);

    feed(&senders[0], WatchUpdate::Snapshot(vec![3]));
    assert_eq!(merged_items(next_merged(&mut merged).await), [3, 2]);
}

#[tokio::test(start_paused = true)]
async fn merge_emits_partial_snapshot_then_failure() {
    let (senders, mut merged) = live_merge(&["a", "b"], None);
    feed(&senders[0], WatchUpdate::Snapshot(vec![1]));
    absorb(&mut merged).await;
    let failure = ClusterError::Forbidden {
        context: "test".to_owned(),
        action: "watching pods",
        message: "scripted failure".to_owned(),
    };
    feed(&senders[1], WatchUpdate::Failed(failure));

    assert_eq!(merged_items(next_merged(&mut merged).await), [1]);
    let Some(WatchUpdate::Failed(ClusterError::Namespace { namespace, source })) =
        next_merged(&mut merged).await
    else {
        panic!("expected a failure naming the namespace");
    };
    assert_eq!(namespace, "b");
    assert!(matches!(*source, ClusterError::Forbidden { .. }));

    feed(&senders[1], WatchUpdate::Snapshot(vec![2]));
    assert_eq!(merged_items(next_merged(&mut merged).await), [1, 2]);
}

fn forbidden() -> ClusterError {
    ClusterError::Forbidden {
        context: "test".to_owned(),
        action: "watching pods",
        message: "scripted failure".to_owned(),
    }
}

/// The namespace and the repeated message of a re-announced failure.
fn repeated_failure(update: Option<WatchUpdate<u32>>) -> (String, String) {
    let Some(WatchUpdate::Failed(ClusterError::Namespace { namespace, source })) = update else {
        panic!("expected a failure naming the namespace, got {update:?}");
    };
    let ClusterError::Rendered { message } = *source else {
        panic!("expected a repeated failure");
    };
    (namespace, message)
}

#[tokio::test(start_paused = true)]
async fn merge_coalesces_snapshots_within_a_window() {
    let (senders, mut merged) = live_merge(&["a", "b"], None);
    feed(&senders[0], WatchUpdate::Snapshot(vec![1]));
    feed(&senders[1], WatchUpdate::Snapshot(vec![2]));
    feed(&senders[0], WatchUpdate::Snapshot(vec![3]));
    assert_eq!(merged_items(next_merged(&mut merged).await), [3, 2]);
    assert!(next_merged(&mut merged).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn merge_reemits_after_the_window() {
    let (senders, mut merged) = live_merge(&["a", "b"], None);
    feed(&senders[0], WatchUpdate::Snapshot(vec![1]));
    feed(&senders[1], WatchUpdate::Snapshot(vec![2]));
    assert_eq!(merged_items(next_merged(&mut merged).await), [1, 2]);

    tokio::time::advance(BATCH_WINDOW).await;
    feed(&senders[1], WatchUpdate::Snapshot(vec![4]));
    assert_eq!(merged_items(next_merged(&mut merged).await), [1, 4]);
}

#[tokio::test(start_paused = true)]
async fn merge_reannounces_unresolved_failure_after_other_snapshot() {
    let (senders, mut merged) = live_merge(&["a", "b"], None);
    feed(&senders[0], WatchUpdate::Snapshot(vec![1]));
    absorb(&mut merged).await;
    feed(&senders[1], WatchUpdate::Failed(forbidden()));
    assert_eq!(merged_items(next_merged(&mut merged).await), [1]);
    assert!(matches!(
        next_merged(&mut merged).await,
        Some(WatchUpdate::Failed(ClusterError::Namespace { .. }))
    ));

    feed(&senders[0], WatchUpdate::Snapshot(vec![3]));
    assert_eq!(merged_items(next_merged(&mut merged).await), [3]);
    let (namespace, message) = repeated_failure(next_merged(&mut merged).await);
    assert_eq!(namespace, "b");
    assert_eq!(message, forbidden().to_string());
}

#[tokio::test(start_paused = true)]
async fn merge_clears_failure_on_that_namespace_snapshot() {
    let (senders, mut merged) = live_merge(&["a", "b"], None);
    feed(&senders[0], WatchUpdate::Snapshot(vec![1]));
    absorb(&mut merged).await;
    feed(&senders[1], WatchUpdate::Failed(forbidden()));
    assert_eq!(merged_items(next_merged(&mut merged).await), [1]);
    assert!(next_merged(&mut merged).await.is_some());

    feed(&senders[1], WatchUpdate::Snapshot(vec![2]));
    assert_eq!(merged_items(next_merged(&mut merged).await), [1, 2]);
    feed(&senders[0], WatchUpdate::Snapshot(vec![3]));
    assert_eq!(merged_items(next_merged(&mut merged).await), [3, 2]);
    assert!(next_merged(&mut merged).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn merge_keeps_stale_items_of_failed_input() {
    let (senders, mut merged) = live_merge(&["a", "b"], None);
    feed(&senders[0], WatchUpdate::Snapshot(vec![1]));
    feed(&senders[1], WatchUpdate::Snapshot(vec![2]));
    assert_eq!(merged_items(next_merged(&mut merged).await), [1, 2]);

    feed(&senders[1], WatchUpdate::Failed(forbidden()));
    assert!(matches!(
        next_merged(&mut merged).await,
        Some(WatchUpdate::Failed(ClusterError::Namespace { .. }))
    ));
    assert!(next_merged(&mut merged).await.is_none());

    feed(&senders[0], WatchUpdate::Snapshot(vec![5]));
    assert_eq!(merged_items(next_merged(&mut merged).await), [5, 2]);
    let (namespace, _) = repeated_failure(next_merged(&mut merged).await);
    assert_eq!(namespace, "b");
}

#[tokio::test(start_paused = true)]
async fn merge_all_failed_emits_only_failures() {
    let (senders, mut merged) = live_merge(&["a", "b"], None);
    feed(&senders[0], WatchUpdate::Failed(forbidden()));
    feed(&senders[1], WatchUpdate::Failed(forbidden()));
    for namespace in ["a", "b"] {
        let Some(WatchUpdate::Failed(ClusterError::Namespace {
            namespace: named, ..
        })) = next_merged(&mut merged).await
        else {
            panic!("expected a failure naming {namespace}");
        };
        assert_eq!(named, namespace);
    }
    assert!(next_merged(&mut merged).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn merge_input_ending_while_waiting_does_not_stall() {
    let (mut senders, mut merged) = live_merge(&["a", "b"], None);
    feed(&senders[0], WatchUpdate::Snapshot(vec![1]));
    absorb(&mut merged).await;
    drop(senders.remove(1));

    assert_eq!(merged_items(next_merged(&mut merged).await), [1]);
    let Some(WatchUpdate::Failed(ClusterError::Namespace { namespace, source })) =
        next_merged(&mut merged).await
    else {
        panic!("expected a failure naming the ended namespace");
    };
    assert_eq!(namespace, "b");
    assert!(matches!(*source, ClusterError::Rendered { .. }));
}

#[tokio::test(start_paused = true)]
async fn merge_limit_keeps_newest_in_order() {
    let limit = StoreLimit {
        max_items: 2,
        recency: |item: &u32| jiff::Timestamp::from_second(i64::from(*item)).ok(),
    };
    let inputs = vec![
        merge_input("a", stream::iter([WatchUpdate::Snapshot(vec![10, 30])])),
        merge_input("b", stream::iter([WatchUpdate::Snapshot(vec![20])])),
    ];
    let updates: Vec<_> = merge_snapshots(inputs, Some(limit)).collect().await;
    assert_eq!(merged_items(updates.into_iter().next()), [30, 20]);
}

// Metadata watches

fn partial(namespace: &str, name: &str) -> PartialObjectMeta<Pod> {
    ObjectMeta {
        namespace: Some(namespace.to_owned()),
        name: Some(name.to_owned()),
        ..Default::default()
    }
    .into_response_partial::<Pod>()
}

fn partial_name(meta: &PartialObjectMeta<Pod>) -> String {
    meta.metadata.name.clone().unwrap_or_default()
}

#[tokio::test(start_paused = true)]
async fn batch_updates_accepts_partial_object_meta() {
    let mut items = vec![Ok(Event::Init)];
    items.extend(
        [partial("ns", "b"), partial("ns", "a")]
            .into_iter()
            .map(|meta| Ok(Event::InitApply(meta))),
    );
    items.push(Ok(Event::InitDone));
    items.push(Ok(Event::Apply(partial("ns", "c"))));
    let updates: Vec<_> = batch_updates(
        stream::iter(items),
        "test".to_owned(),
        ACTION,
        partial_name,
        None,
    )
    .collect()
    .await;
    // The apply after the initial list lands inside the same batch window.
    assert_eq!(updates.len(), 1);
    let WatchUpdate::Snapshot(names) = &updates[0] else {
        panic!("expected a snapshot");
    };
    assert_eq!(names, &["a", "b", "c"]);
}

#[tokio::test(start_paused = true)]
async fn closure_summarizer_captures_runtime_data() {
    // Runtime data a `fn` pointer cannot carry, such as printer columns chosen per custom kind.
    let suffix = "-custom".to_owned();
    let summarize =
        move |pod: &Pod| format!("{}{suffix}", pod.metadata.name.clone().unwrap_or_default());
    let updates: Vec<_> = batch_updates(
        stream::iter(initial_list(&[pod("ns", "a")])),
        "test".to_owned(),
        ACTION,
        summarize,
        None,
    )
    .collect()
    .await;
    let WatchUpdate::Snapshot(items) = &updates[0] else {
        panic!("expected a snapshot");
    };
    assert_eq!(items, &["a-custom"]);
}
