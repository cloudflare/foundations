use cf_rustracing::span::SpanConsumer;
use cf_rustracing_jaeger::span::{FinishedSpan, SpanContextState as JaegerContext};
use std::future::poll_fn;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::{Mutex, mpsc};

// Use enum instead of trait to avoid generics everywhere. Under the hood,
// both channel types share their implementations. The compiler optimizes
// the match statements away.
enum Receiver<T> {
    Bounded(mpsc::Receiver<T>),
    Unbounded(mpsc::UnboundedReceiver<T>),
}

impl<T> Receiver<T> {
    #[allow(dead_code, reason = "only used if `metrics` feature is enabled")]
    #[inline]
    fn len(&self) -> usize {
        match self {
            Self::Bounded(r) => r.len(),
            Self::Unbounded(r) => r.len(),
        }
    }

    #[allow(dead_code, reason = "only used if `testing` feature is enabled")]
    #[inline]
    fn try_recv(&mut self) -> Result<T, mpsc::error::TryRecvError> {
        match self {
            Self::Bounded(r) => r.try_recv(),
            Self::Unbounded(r) => r.try_recv(),
        }
    }

    #[inline]
    fn poll_recv_many(
        &mut self,
        cx: &mut Context,
        buffer: &mut Vec<T>,
        limit: usize,
    ) -> Poll<usize> {
        match self {
            Self::Bounded(r) => r.poll_recv_many(cx, buffer, limit),
            Self::Unbounded(r) => r.poll_recv_many(cx, buffer, limit),
        }
    }
}

/// Identifies the tracing pipeline that a span channel belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub(super) enum PipelineType {
    /// Regular (system) tracing pipeline.
    ///
    /// Encodes to an empty string for backwards compatibility with existing
    /// metrics. Prometheus treats this the same as an absent label.
    #[serde(rename = "")]
    System,
}

/// An instrumented, multi-consumer span receiver layered on top of
/// [`tokio::sync::mpsc`].
///
/// Multi-consumer semantics are achieved by wrapping tokio's single receiver in
/// an async [`Mutex`]. This means only a single task can actively wait for messages
/// at a time, while other tasks are queueing for the Mutex in FIFO order.
///
/// To amortize the overhead of locking, we enforce batching by only exposing the
/// `recv_many` operation.
#[derive(Clone)]
pub(super) struct SharedSpanReceiver {
    rx: Arc<Mutex<Receiver<FinishedSpan>>>,

    #[allow(dead_code, reason = "only used if `metrics` feature is enabled")]
    metrics_label: PipelineType,
}

impl SharedSpanReceiver {
    fn new_bounded(receiver: mpsc::Receiver<FinishedSpan>, metrics_label: PipelineType) -> Self {
        Self {
            rx: Arc::new(Mutex::new(Receiver::Bounded(receiver))),
            metrics_label,
        }
    }

    fn new_unbounded(
        receiver: mpsc::UnboundedReceiver<FinishedSpan>,
        metrics_label: PipelineType,
    ) -> Self {
        Self {
            rx: Arc::new(Mutex::new(Receiver::Unbounded(receiver))),
            metrics_label,
        }
    }

    /// Tries to receive a span from the channel if the receiver is unique.
    ///
    /// This will return `None` if there are multiple receivers for this channel.
    #[cfg(any(test, feature = "testing"))]
    pub(super) fn try_unique_recv(&mut self) -> Option<FinishedSpan> {
        let rx = Arc::get_mut(&mut self.rx)?.get_mut();
        let res = rx.try_recv();

        #[cfg(feature = "metrics")]
        super::metrics::tracing::queue_size(self.metrics_label).set(rx.len() as u64);

        res.ok()
    }

    pub(super) async fn recv_many(&self, buffer: &mut Vec<FinishedSpan>, limit: usize) -> usize {
        // Obtain the lock. Tasks that are waiting here are not the active consumer,
        // so we don't need to update `length_gauge` while waiting.
        let rx = &mut *self.rx.lock().await;

        #[cfg(feature = "metrics")]
        let queue_size = super::metrics::tracing::queue_size(self.metrics_label);

        // Execute the recv_many operation. This means we are the active consumer and
        // are woken up if the channel length changes.
        let res = poll_fn(|cx| {
            #[cfg(feature = "metrics")]
            queue_size.set(rx.len() as u64);
            rx.poll_recv_many(cx, buffer, limit)
        })
        .await;

        #[cfg(feature = "metrics")]
        queue_size.set(rx.len() as u64);

        res
    }
}

trait Sender<T> {
    fn try_send(&self, message: T) -> Result<(), mpsc::error::TrySendError<T>>;
}

impl<T> Sender<T> for mpsc::Sender<T> {
    #[inline]
    fn try_send(&self, message: T) -> Result<(), mpsc::error::TrySendError<T>> {
        mpsc::Sender::try_send(self, message)
    }
}

impl<T> Sender<T> for mpsc::UnboundedSender<T> {
    #[inline]
    fn try_send(&self, message: T) -> Result<(), mpsc::error::TrySendError<T>> {
        self.send(message)?;
        Ok(())
    }
}

/// An instrumented sender for [`cf_rustracing_jaeger`] spans.
#[derive(Clone)]
pub(super) struct SpanSender<S> {
    inner: S,

    #[allow(dead_code, reason = "only used if `metrics` feature is enabled")]
    metrics_label: PipelineType,
}

impl<S: Sender<FinishedSpan> + Send + Sync> SpanConsumer<JaegerContext> for SpanSender<S> {
    fn consume_span(&self, span: FinishedSpan) {
        let _res = self.inner.try_send(span);

        #[cfg(feature = "metrics")]
        {
            super::metrics::tracing::spans_total(self.metrics_label).inc();
            if _res.is_err() {
                super::metrics::tracing::spans_dropped(self.metrics_label).inc();
            }
        }
    }
}

pub(super) type BoundedSpanSender = SpanSender<mpsc::Sender<FinishedSpan>>;
pub(super) type UnboundedSpanSender = SpanSender<mpsc::UnboundedSender<FinishedSpan>>;

/// Creates a bounded MPMC channel for [`cf_rustracing_jaeger`] spans.
pub(super) fn channel(
    buffer: NonZeroUsize,
    metrics_label: PipelineType,
) -> (BoundedSpanSender, SharedSpanReceiver) {
    let (send, recv) = mpsc::channel(buffer.get());
    (
        SpanSender {
            inner: send,
            metrics_label,
        },
        SharedSpanReceiver::new_bounded(recv, metrics_label),
    )
}

/// Creates an unbounded MPMC channel for [`cf_rustracing_jaeger`] spans.
pub(super) fn unbounded_channel(
    metrics_label: PipelineType,
) -> (UnboundedSpanSender, SharedSpanReceiver) {
    let (send, recv) = mpsc::unbounded_channel();
    (
        SpanSender {
            inner: send,
            metrics_label,
        },
        SharedSpanReceiver::new_unbounded(recv, metrics_label),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::tracing::metrics::tracing as tracing_metrics;

    use cf_rustracing::Tracer;
    use cf_rustracing::sampler::AllSampler;

    #[tokio::test]
    async fn test_span_metrics() {
        let (send, recv) = channel(NonZeroUsize::new(3).unwrap(), PipelineType::System);
        let tracer = Tracer::with_consumer(AllSampler, send);

        for _ in 0..5 {
            let _span = tracer.span("my span").start();
        }

        assert_eq!(tracing_metrics::spans_total(PipelineType::System).get(), 5);
        assert_eq!(
            tracing_metrics::spans_dropped(PipelineType::System).get(),
            2,
        );

        let mut spans = Vec::new();
        let got = recv.recv_many(&mut spans, 1).await;
        assert_eq!(got, 1);
        assert_eq!(spans.len(), 1);
        assert_eq!(tracing_metrics::queue_size(PipelineType::System).get(), 2);

        let got = recv.recv_many(&mut spans, 100).await;
        assert_eq!(got, 2);
        assert_eq!(spans.len(), 3);
        assert_eq!(tracing_metrics::queue_size(PipelineType::System).get(), 0);
    }
}
