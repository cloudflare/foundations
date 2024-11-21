#[cfg(feature = "metrics")]
use super::metrics;
use super::settings::TelemetrySettings;
use crate::telemetry::log;
use crate::BootstrapResult;
use anyhow::Context as _;
use futures_util::future::FutureExt;
use futures_util::{pin_mut, ready};
use hyper_util::rt::TokioIo;
use socket2::{Domain, SockAddr, Socket, Type};
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::net::TcpListener;
use tokio::sync::watch;

mod router;

use router::Router;
pub use router::{
    BoxError, TelemetryRouteHandler, TelemetryRouteHandlerFuture, TelemetryServerRoute,
};

pub(super) struct TelemetryServerFuture {
    listener: TcpListener,
    router: Router,
}

impl TelemetryServerFuture {
    pub(super) fn new(
        settings: TelemetrySettings,
        custom_routes: Vec<TelemetryServerRoute>,
    ) -> BootstrapResult<Option<TelemetryServerFuture>> {
        if !settings.server.enabled {
            return Ok(None);
        }

        let settings = Arc::new(settings);

        // Eagerly init the memory profiler so it gets set up before syscalls are sandboxed with seccomp.
        #[cfg(all(target_os = "linux", feature = "memory-profiling"))]
        if settings.memory_profiler.enabled {
            memory_profiling::profiler(Arc::clone(&settings))
                .map_err(|err| anyhow::anyhow!(err))?;
        }

        let addr = settings.server.addr;

        #[cfg(feature = "settings")]
        let addr = SocketAddr::from(addr);

        let router = Router::new(custom_routes, settings);

        let listener = {
            let std_listener = std::net::TcpListener::from(
                bind_socket(addr).with_context(|| format!("binding to socket {addr:?}"))?,
            );

            std_listener.set_nonblocking(true)?;

            tokio::net::TcpListener::from_std(std_listener)?
        };

        Ok(Some(TelemetryServerFuture { listener, router }))
    }
    pub(super) fn local_addr(&self) -> SocketAddr {
        self.listener.local_addr().unwrap()
    }

    // Adapted from Hyper 0.14 Server stuff and axum::serve::serve.
    pub(super) async fn with_graceful_shutdown(
        self,
        shutdown_signal: impl Future<Output = ()> + Send + Sync + 'static,
    ) {
        let (signal_tx, signal_rx) = watch::channel(());
        let signal_tx = Arc::new(signal_tx);

        tokio::spawn(async move {
            shutdown_signal.await;

            drop(signal_rx);
        });

        let (close_tx, close_rx) = watch::channel(());
        let listener = self.listener;

        pin_mut!(listener);

        loop {
            let socket = tokio::select! {
                conn = listener.accept() => match conn {
                    Ok((conn, _)) => TokioIo::new(conn),
                    Err(e) => {
                        log::warn!("failed to accept connection"; "error" => e);

                        continue;
                    }
                },
                _ = signal_tx.closed() => { break },
            };

            let router = self.router.clone();
            let signal_tx = Arc::clone(&signal_tx);
            let close_rx = close_rx.clone();

            tokio::spawn(async move {
                let conn = hyper::server::conn::http1::Builder::new()
                    .serve_connection(socket, router)
                    .with_upgrades();

                let signal_closed = signal_tx.closed().fuse();

                pin_mut!(conn);
                pin_mut!(signal_closed);

                loop {
                    tokio::select! {
                        _ = conn.as_mut() => break,
                        _ = &mut signal_closed => conn.as_mut().graceful_shutdown(),
                    }
                }

                drop(close_rx);
            });
        }

        drop(close_rx);

        close_tx.closed().await;
    }
}

impl Future for TelemetryServerFuture {
    type Output = Infallible;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = &mut *self;

        loop {
            let socket = match ready!(Pin::new(&mut this.listener).poll_accept(cx)) {
                Ok((conn, _)) => TokioIo::new(conn),
                Err(e) => {
                    log::warn!("failed to accept connection"; "error" => e);

                    continue;
                }
            };

            let router = this.router.clone();

            tokio::spawn(
                hyper::server::conn::http1::Builder::new()
                    // upgrades needed for websockets
                    .serve_connection(socket, router)
                    .with_upgrades(),
            );
        }
    }
}

fn bind_socket(addr: SocketAddr) -> BootstrapResult<Socket> {
    let socket = Socket::new(
        if addr.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        },
        Type::STREAM,
        None,
    )?;

    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.bind(&SockAddr::from(addr))?;
    socket.listen(1024)?;

    Ok(socket)
}

#[cfg(all(target_os = "linux", feature = "memory-profiling"))]
mod memory_profiling {
    use super::*;
    use crate::telemetry::MemoryProfiler;
    use crate::Result;

    pub(super) fn profiler(settings: Arc<TelemetrySettings>) -> Result<MemoryProfiler> {
        MemoryProfiler::get_or_init_with(&settings.memory_profiler)?.ok_or_else(|| {
            "profiling should be enabled via `_RJEM_MALLOC_CONF=prof:true` env var".into()
        })
    }

    pub(super) async fn heap_profile(settings: Arc<TelemetrySettings>) -> Result<String> {
        profiler(settings)?.heap_profile().await
    }

    pub(super) async fn heap_stats(settings: Arc<TelemetrySettings>) -> Result<String> {
        profiler(settings)?.heap_stats()
    }
}
