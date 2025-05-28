use crate::utils::feature_use;
use crate::BootstrapResult;
use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt, Stream};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

feature_use!(cfg(feature = "telemetry-server"), {
    use super::server::TelemetryServerFuture;
    use std::net::SocketAddr;
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
    server_addr: Option<SocketAddr>,

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
            server_addr: server_fut.as_ref().map(|fut| fut.local_addr()),

            #[cfg(feature = "telemetry-server")]
            server_fut,

            tele_futures,
        }
    }

    /// Address of the telemetry server.
    ///
    /// Returns `None` if the server wasn't spawned.
    #[cfg(feature = "telemetry-server")]
    pub fn server_addr(&self) -> Option<SocketAddr> {
        self.server_addr
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
        let mut ready_res = vec![];

        #[cfg(feature = "telemetry-server")]
        if let Some(server_fut) = &mut self.server_fut {
            if let Poll::Ready(res) = Pin::new(server_fut).poll(cx) {
                match res {}
            }
        }

        let tele_futures_poll = Pin::new(&mut self.tele_futures).poll_next(cx);

        if let Poll::Ready(Some(res)) = tele_futures_poll {
            ready_res.push(res);
        }

        if ready_res.is_empty() {
            Poll::Pending
        } else {
            Poll::Ready(ready_res.into_iter().collect())
        }
    }
}
