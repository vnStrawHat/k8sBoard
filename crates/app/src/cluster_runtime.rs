use std::future::Future;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};

use futures::{Stream, StreamExt as _};
use gpui_kit::{Context, Global, Task};

/// Handle to the tokio runtime that `main` owns. Everything from the `cluster` crate is
/// polled on it; the GPUI main thread only awaits channels and `RuntimeTask`s.
#[derive(Clone)]
pub(crate) struct ClusterRuntime {
    handle: tokio::runtime::Handle,
}

impl Global for ClusterRuntime {}

/// A tokio task that is aborted when dropped. Awaiting it yields the task output.
pub(crate) struct RuntimeTask<T>(tokio::task::JoinHandle<T>);

impl<T> Future for RuntimeTask<T> {
    type Output = Result<T, tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx)
    }
}

impl<T> Drop for RuntimeTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A live watch feeding one GPUI entity. Dropping it stops both halves.
pub(crate) struct WatchSubscription {
    _pump: RuntimeTask<()>,
    _receiver: Task<()>,
}

/// Whether an applied update repaints its owner.
#[derive(Clone, Copy)]
enum UpdateNotice {
    Notify,
    Silent,
}

impl UpdateNotice {
    fn notify<V: 'static>(self, cx: &mut Context<V>) {
        if matches!(self, Self::Notify) {
            cx.notify();
        }
    }
}

impl ClusterRuntime {
    pub(crate) fn new(handle: tokio::runtime::Handle) -> Self {
        Self { handle }
    }

    pub(crate) fn spawn<F>(&self, future: F) -> RuntimeTask<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        RuntimeTask(self.handle.spawn(future))
    }

    /// Feeds `updates` (resource watches and pod logs) into `view`: the stream is polled on
    /// tokio, and `apply` runs on the
    /// GPUI thread, followed by one `cx.notify()` per update (the crate already batches
    /// at 100 ms). `on_closed` runs when the update stream ends for any reason (the watch
    /// streams normally never end), so the owner can mark the list as stopped.
    pub(crate) fn subscribe<V: 'static, U: Send + 'static>(
        &self,
        updates: impl Stream<Item = U> + Send + 'static,
        cx: &mut Context<V>,
        apply: impl Fn(&mut V, U, &mut Context<V>) + 'static,
        on_closed: impl FnOnce(&mut V, &mut Context<V>) + 'static,
    ) -> WatchSubscription {
        self.subscribe_with(UpdateNotice::Notify, updates, cx, apply, on_closed)
    }

    /// `subscribe` without the repaint: for feeds that only mark the issue board dirty, which
    /// repaints on its own tick when something changed.
    pub(crate) fn subscribe_silent<V: 'static, U: Send + 'static>(
        &self,
        updates: impl Stream<Item = U> + Send + 'static,
        cx: &mut Context<V>,
        apply: impl Fn(&mut V, U, &mut Context<V>) + 'static,
        on_closed: impl FnOnce(&mut V, &mut Context<V>) + 'static,
    ) -> WatchSubscription {
        self.subscribe_with(UpdateNotice::Silent, updates, cx, apply, on_closed)
    }

    fn subscribe_with<V: 'static, U: Send + 'static>(
        &self,
        notice: UpdateNotice,
        updates: impl Stream<Item = U> + Send + 'static,
        cx: &mut Context<V>,
        apply: impl Fn(&mut V, U, &mut Context<V>) + 'static,
        on_closed: impl FnOnce(&mut V, &mut Context<V>) + 'static,
    ) -> WatchSubscription {
        // Capacity 1 bounds memory: a slow UI pauses the pump instead of queueing snapshots.
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let pump = self.spawn(async move {
            let mut updates = std::pin::pin!(updates);
            while let Some(update) = updates.next().await {
                if sender.send(update).await.is_err() {
                    return;
                }
            }
        });
        let receiver = cx.spawn(async move |this, cx| {
            while let Some(update) = receiver.recv().await {
                let applied = this.update(cx, |view, cx| {
                    apply(view, update, cx);
                    notice.notify(cx);
                });
                if applied.is_err() {
                    return;
                }
            }
            // The entity may already be gone, in which case there is nothing to mark.
            let _ = this.update(cx, |view, cx| {
                on_closed(view, cx);
                notice.notify(cx);
            });
        });
        WatchSubscription {
            _pump: pump,
            _receiver: receiver,
        }
    }
}
