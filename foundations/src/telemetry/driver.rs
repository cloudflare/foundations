#[cfg(feature = "telemetry-server")]
use crate::addr::ListenAddr;
use crate::utils::feature_use;
use crate::BootstrapResult;
use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt, Stream};
use std::future::Future;
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

            tele_futures,
        }
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
