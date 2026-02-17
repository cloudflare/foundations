use crate::BootstrapResult;
#[cfg(feature = "telemetry-server")]
use crate::addr::ListenAddr;
use crate::utils::feature_use;
use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt, Stream};
#[cfg(feature = "logging")]
use slog_async::AsyncGuard;
use std::future::Future;
#[cfg(feature = "logging")]
use std::mem::ManuallyDrop;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

feature_use!(cfg(feature = "telemetry-server"), {
    use super::server::TelemetryServerFuture;
});

/// A future that drives async telemetry functionality and that is returned
/// by [`crate::telemetry::init`].
///
/// You would usually need to pass it to [`tokio::spawn`] once all the [security syscall-related]
/// configuration steps are completed.
///
/// [security syscall-related]: `crate::security`
pub struct TelemetryDriver {
    #[cfg(feature = "telemetry-server")]
    server_addr: Option<ListenAddr>,

    #[cfg(feature = "telemetry-server")]
    server_fut: Option<TelemetryServerFuture>,

    #[cfg(feature = "logging")]
    logging_guard: Option<ManuallyDrop<AsyncGuard>>,

    tele_futures: FuturesUnordered<BoxFuture<'static, BootstrapResult<()>>>,
}

impl TelemetryDriver {
    pub(super) fn new(
        #[cfg(feature = "telemetry-server")] server_fut: Option<TelemetryServerFuture>,
        tele_futures: FuturesUnordered<BoxFuture<'static, BootstrapResult<()>>>,
    ) -> Self {
        Self {
            #[cfg(feature = "telemetry-server")]
            server_addr: server_fut.as_ref().and_then(|fut| fut.local_addr().ok()),

            #[cfg(feature = "telemetry-server")]
            server_fut,

            #[cfg(feature = "logging")]
            logging_guard: None,

            tele_futures,
        }
    }

    /// Binds to a `slog::AsyncGuard` to ensure logs get dropped when calling `shutdown_logger`
    #[cfg(feature = "logging")]
    pub(super) fn set_logging_guard(&mut self, logging_async_guard: Option<AsyncGuard>) {
        self.logging_guard = logging_async_guard.map(ManuallyDrop::new);
    }

    /// Address of the telemetry server.
    ///
    /// Returns `None` if the server wasn't spawned.
    #[cfg(feature = "telemetry-server")]
    pub fn server_addr(&self) -> Option<&ListenAddr> {
        self.server_addr.as_ref()
    }

    /// Instructs the telemetry driver and server to perform an orderly shutdown when the given
    /// future `signal` completes.
    ///
    /// If telemetry is disabled, the given signal is still awaited before driver termination.
    pub fn with_graceful_shutdown(
        &mut self,
        signal: impl Future<Output = ()> + Send + Sync + 'static,
    ) {
        #[cfg(feature = "telemetry-server")]
        {
            if let Some(server_fut) = self.server_fut.take() {
                self.tele_futures.push(Box::pin(async move {
                    server_fut.with_graceful_shutdown(signal).await;

                    Ok(())
                }));

                return;
            }
        }

        self.tele_futures.push(
            async move {
                signal.await;
                Ok(())
            }
            .boxed(),
        );
    }

    /// Waits for all pending records to flush, then shuts down logging permanently.
    ///
    /// By default, logging is not automatically shut down when TelemetryDriver goes out of scope,
    /// and manual shutdown is necessary. Calling this blocks the calling thread, so it is advised
    /// to wrap in `[spawn_blocking](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)`
    /// in async contexts.
    #[cfg(feature = "logging")]
    pub fn shutdown_logger(&mut self) {
        if let Some(guard) = self.logging_guard.take() {
            drop(ManuallyDrop::into_inner(guard))
        }
    }
}

impl Future for TelemetryDriver {
    type Output = BootstrapResult<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[cfg_attr(not(feature = "telemetry-server"), allow(unused_mut))]
        let mut server_res = Poll::Ready(Ok(()));

        #[cfg(feature = "telemetry-server")]
        if let Some(server_fut) = &mut self.server_fut {
            // This future is always pending
            let Poll::Pending = Pin::new(server_fut).poll(cx);
            server_res = Poll::Pending;
        }

        loop {
            // Keep polling tele_futures until it becomes pending, empty, or a future errors
            let tele_res = ready!(Pin::new(&mut self.tele_futures).poll_next(cx)?);
            if tele_res.is_none() {
                // tele_futures is done, but we may still need to poll server_fut
                return server_res;
            }
        }
    }
}
