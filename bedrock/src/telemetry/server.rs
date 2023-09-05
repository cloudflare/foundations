#[cfg(feature = "metrics")]
use super::metrics;
use super::settings::TelemetrySettings;
use super::TelemetryServerFuture;
use crate::{BootstrapResult, Result};
use anyhow::anyhow;
use futures_util::TryFutureExt;
use hyper::{header, Body, Method, Request, Response, Server, StatusCode};
use routerify::{Router, RouterService};
use socket2::{Domain, SockAddr, Socket, Type};
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

/// A telemetry server route descriptor.
pub struct TelemetryServerRoute<P, H> {
    /// URL path of the route.
    pub path: P,

    /// A list of HTTP methods for which this route is active.
    pub methods: Vec<Method>,

    /// A route handler.
    pub handler: H,
}

pub(super) fn init<P, H, R>(
    settings: TelemetrySettings,
    custom_routes: Vec<TelemetryServerRoute<P, H>>,
) -> BootstrapResult<TelemetryServerFuture>
where
    P: Into<String>,
    H: Fn(Request<Body>, Arc<TelemetrySettings>) -> R + Send + Sync + 'static,
    R: Future<Output = std::result::Result<Response<Body>, Infallible>> + Send + 'static,
{
    let settings = Arc::new(settings);
    let router = create_router(&settings, custom_routes)?;
    let addr = settings.server.addr;

    #[cfg(feature = "settings")]
    let addr = SocketAddr::from(addr);

    let socket = bind_socket(addr)?;
    let builder = Server::from_tcp(socket.into())?;
    let service = RouterService::new(router).map_err(|err| anyhow!(err))?;

    Ok(Box::pin(builder.serve(service).err_into()))
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

fn create_router<P, H, R>(
    settings: &Arc<TelemetrySettings>,
    custom_routes: Vec<TelemetryServerRoute<P, H>>,
) -> BootstrapResult<Router<Body, Infallible>>
where
    P: Into<String>,
    H: Fn(Request<Body>, Arc<TelemetrySettings>) -> R + Send + Sync + 'static,
    R: Future<Output = std::result::Result<Response<Body>, Infallible>> + Send + 'static,
{
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
