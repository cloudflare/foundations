#[cfg(feature = "metrics")]
use super::metrics;
use super::settings::TelemetrySettings;
use crate::{BootstrapResult, Result};
use anyhow::anyhow;
use futures_util::future::BoxFuture;
use futures_util::ready;
use hyper::server::conn::AddrIncoming;
use hyper::{header, Body, Method, Request, Response, Server, StatusCode};
use routerify::{Router, RouterService};
use socket2::{Domain, SockAddr, Socket, Type};
use std::convert::Infallible;
use std::future::Future;
use std::net::{SocketAddr, TcpListener};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

/// Telemetry server future returned by [`crate::telemetry::init_with_server`].
///
/// This future drives a HTTP server as configured by [`TelemetryServerSettings`].
///
/// [`TelemetryServerSettings`]: `crate::telemetry::settings::TelemetryServerSettings`
pub struct TelemetryServerFuture {
    pub(super) inner: Option<Server<AddrIncoming, RouterService<Body, Infallible>>>,
}

impl TelemetryServerFuture {
    /// Address of the telemetry server.
    ///
    /// Returns `None` if the server wasn't spawned.
    pub fn server_addr(&self) -> Option<SocketAddr> {
        self.inner.as_ref().map(Server::local_addr)
    }

    /// Prepares the telemetry server to handle graceful shutdown when
    /// the provided future completes.
    pub async fn with_graceful_shutdown(
        self,
        signal: impl Future<Output = ()> + Send + Sync + 'static,
    ) -> BootstrapResult<()> {
        match self.inner {
            Some(server) => Ok(server.with_graceful_shutdown(signal).await?),
            None => {
                signal.await;
                Ok(())
            }
        }
    }
}

impl Future for TelemetryServerFuture {
    type Output = BootstrapResult<()>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(server) = &mut self.inner {
            Poll::Ready(Ok(ready!(Pin::new(server).poll(cx))?))
        } else {
            Poll::Pending
        }
    }
}

/// Future returned by [`TelemetryServerRoute::handler`].
pub type TelemetryRouteHandlerFuture =
    BoxFuture<'static, std::result::Result<Response<Body>, Infallible>>;

/// Telemetry route handler.
pub type TelemetryRouteHandler = Box<
    dyn Fn(Request<Body>, Arc<TelemetrySettings>) -> TelemetryRouteHandlerFuture
        + Send
        + Sync
        + 'static,
>;

/// A telemetry server route descriptor.
pub struct TelemetryServerRoute {
    /// URL path of the route.
    pub path: String,

    /// A list of HTTP methods for which this route is active.
    pub methods: Vec<Method>,

    /// A route handler.
    pub handler: TelemetryRouteHandler,
}

pub(super) fn init(
    settings: TelemetrySettings,
    custom_routes: Vec<TelemetryServerRoute>,
) -> BootstrapResult<TelemetryServerFuture> {
    if !settings.server.enabled {
        return Ok(TelemetryServerFuture { inner: None });
    }

    let settings = Arc::new(settings);
    let router = create_router(&settings, custom_routes)?;
    let addr = settings.server.addr;

    #[cfg(feature = "settings")]
    let addr = SocketAddr::from(addr);

    let socket = TcpListener::from(bind_socket(addr)?);
    let builder = Server::from_tcp(socket)?;
    let service = RouterService::new(router).map_err(|err| anyhow!(err))?;

    Ok(TelemetryServerFuture {
        inner: Some(builder.serve(service)),
    })
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
    socket.set_reuse_port(true)?;
    socket.bind(&SockAddr::from(addr))?;
    socket.listen(1024)?;

    Ok(socket)
}

fn create_router(
    settings: &Arc<TelemetrySettings>,
    custom_routes: Vec<TelemetryServerRoute>,
) -> BootstrapResult<Router<Body, Infallible>> {
    let mut router = Router::builder();

    macro_rules! route {
        ($path:expr, $content_type:expr, $f:expr) => {
            router = router.get($path, {
                let settings = Arc::clone(&settings);
                move |_| {
                    let res = $f(Arc::clone(&settings));
                    async move { Ok(into_response($content_type, res.await)) }
                }
            })
        };
    }

    route!("/health", "text/plain", health);

    #[cfg(feature = "metrics")]
    route!("/metrics", "text/plain; version=0.0.4", metrics);

    #[cfg(all(target_os = "linux", feature = "memory-profiling"))]
    route!(
        "/pprof/heap",
        "application/x-gperftools-profile",
        memory_profiling::heap_profile
    );

    #[cfg(all(target_os = "linux", feature = "memory-profiling"))]
    route!(
        "/pprof/heap_stats",
        "text/plain; charset=utf-8",
        memory_profiling::heap_stats
    );

    for route in custom_routes {
        let TelemetryServerRoute {
            path,
            methods,
            handler,
        } = route;

        router = router.add(path, methods, {
            let settings = Arc::clone(settings);
            move |req| handler(req, Arc::clone(&settings))
        });
    }

    router.build().map_err(|err| anyhow!(err))
}

fn into_response(content_type: &str, res: crate::Result<impl Into<Body>>) -> Response<Body> {
    match res {
        Ok(data) => Response::builder()
            .header(header::CONTENT_TYPE, content_type)
            .body(data.into())
            .unwrap(),
        Err(err) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(err.to_string().into())
            .unwrap(),
    }
}

async fn health(_settings: Arc<TelemetrySettings>) -> Result<&'static str> {
    Ok("")
}

#[cfg(feature = "metrics")]
async fn metrics(settings: Arc<TelemetrySettings>) -> Result<String> {
    metrics::collect(&settings.metrics)
}

#[cfg(all(target_os = "linux", feature = "memory-profiling"))]
mod memory_profiling {
    use super::*;
    use crate::telemetry::MemoryProfiler;

    fn profiler(settings: Arc<TelemetrySettings>) -> Result<MemoryProfiler> {
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
