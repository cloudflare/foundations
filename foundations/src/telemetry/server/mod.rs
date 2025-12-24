use super::settings::TelemetrySettings;
use crate::BootstrapResult;
use crate::addr::ListenAddr;
use crate::telemetry::log;
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
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::net::{TcpStream, UnixListener, UnixStream};
use tokio::sync::watch;

mod router;

use router::Router;

enum TelemetryStream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl AsyncRead for TelemetryStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            TelemetryStream::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(unix)]
            TelemetryStream::Unix(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for TelemetryStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        match self.get_mut() {
            TelemetryStream::Tcp(stream) => Pin::new(stream).poll_write(cx, buf),
            #[cfg(unix)]
            TelemetryStream::Unix(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        match self.get_mut() {
            TelemetryStream::Tcp(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(unix)]
            TelemetryStream::Unix(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        match self.get_mut() {
            TelemetryStream::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(unix)]
            TelemetryStream::Unix(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

enum TelemetryListener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(UnixListener),
}

impl TelemetryListener {
    pub(crate) fn local_addr(&self) -> BootstrapResult<ListenAddr> {
        match self {
            TelemetryListener::Tcp(listener) => Ok(listener.local_addr()?.into()),
            #[cfg(unix)]
            TelemetryListener::Unix(listener) => match listener.local_addr()?.as_pathname() {
                Some(path) => Ok(path.to_path_buf().into()),
                None => Err(anyhow::anyhow!("unix socket listener has no pathname")),
            },
        }
    }

    pub(crate) async fn accept(&self) -> std::io::Result<TelemetryStream> {
        match self {
            TelemetryListener::Tcp(listener) => listener
                .accept()
                .await
                .map(|(conn, _)| TelemetryStream::Tcp(conn)),
            #[cfg(unix)]
            TelemetryListener::Unix(listener) => listener
                .accept()
                .await
                .map(|(conn, _)| TelemetryStream::Unix(conn)),
        }
    }

    pub(crate) fn poll_accept(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<TelemetryStream>> {
        match self {
            TelemetryListener::Tcp(listener) => match std::task::ready!(listener.poll_accept(cx)) {
                Ok((conn, _)) => std::task::Poll::Ready(Ok(TelemetryStream::Tcp(conn))),
                Err(e) => std::task::Poll::Ready(Err(e)),
            },
            #[cfg(unix)]
            TelemetryListener::Unix(listener) => {
                match std::task::ready!(listener.poll_accept(cx)) {
                    Ok((conn, _)) => std::task::Poll::Ready(Ok(TelemetryStream::Unix(conn))),
                    Err(e) => std::task::Poll::Ready(Err(e)),
                }
            }
        }
    }
}

pub use router::{
    TelemetryRouteBody, TelemetryRouteHandler, TelemetryRouteHandlerFuture, TelemetryServerRoute,
};

pub(super) struct TelemetryServerFuture {
    listener: TelemetryListener,
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

        let router = Router::new(custom_routes, Arc::clone(&settings));

        let listener = match &settings.server.addr {
            ListenAddr::Tcp(addr) => {
                let std_listener = std::net::TcpListener::from(
                    bind_socket(*addr)
                        .with_context(|| format!("binding to TCP socket {addr:?}"))?,
                );
                std_listener.set_nonblocking(true)?;
                let tokio_listener = tokio::net::TcpListener::from_std(std_listener)?;
                TelemetryListener::Tcp(tokio_listener)
            }
            #[cfg(unix)]
            ListenAddr::Unix(path) => {
                // Remove existing socket file if it exists to avoid bind errors
                if path.exists()
                    && let Err(e) = std::fs::remove_file(path)
                {
                    log::warn!("failed to remove existing Unix socket file"; "path" => %path.display(), "error" => e);
                }

                let unix_listener = UnixListener::bind(path)
                    .with_context(|| format!("binding to Unix socket {path:?}"))?;
                TelemetryListener::Unix(unix_listener)
            }
        };

        Ok(Some(TelemetryServerFuture { listener, router }))
    }

    pub(super) fn local_addr(&self) -> BootstrapResult<ListenAddr> {
        self.listener.local_addr()
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

        loop {
            let socket = tokio::select! {
                conn = listener.accept() => match conn {
                    Ok(conn) => TokioIo::new(conn),
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
            let socket = match ready!(this.listener.poll_accept(cx)) {
                Ok(conn) => TokioIo::new(conn),
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
    use crate::Result;
    use crate::telemetry::MemoryProfiler;

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
