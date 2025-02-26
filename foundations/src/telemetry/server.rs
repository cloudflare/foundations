#[cfg(feature = "metrics")]
use super::metrics;
use super::settings::TelemetrySettings;
#[cfg(feature = "tracing")]
use super::tracing;
use crate::BootstrapResult;
use anyhow::Context as _;
use futures_util::future::{BoxFuture, FutureExt};
use hyper::server::conn::{AddrIncoming, AddrStream};
use hyper::service::Service;
use hyper::{header, Body, Method, Request, Response, Server, StatusCode};
use percent_encoding::percent_decode_str;
use socket2::{Domain, SockAddr, Socket, Type};
use std::collections::HashMap;
use std::convert::Infallible;
use std::future::{ready, Future, Ready};
use std::net::{SocketAddr, TcpListener};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

pub(super) type TelemetryServerFuture = Server<AddrIncoming, Router>;

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

struct RouteMap(HashMap<Method, HashMap<String, Arc<TelemetryRouteHandler>>>);

impl RouteMap {
    fn new(custom_routes: Vec<TelemetryServerRoute>) -> Self {
        let mut map = RouteMap(Default::default());

        map.init_built_in_routes();

        for route in custom_routes {
            map.set(route);
        }

        map
    }

    fn init_built_in_routes(&mut self) {
        self.set(TelemetryServerRoute {
            path: "/health".into(),
            methods: vec![Method::GET],
            handler: Box::new(|_, _| async { into_response("text/plain", Ok("")) }.boxed()),
        });

        #[cfg(feature = "metrics")]
        self.set(TelemetryServerRoute {
            path: "/metrics".into(),
            methods: vec![Method::GET],
            handler: Box::new(|_, settings| {
                async move {
                    into_response(
                        "application/openmetrics-text; version=1.0.0; charset=utf-8",
                        metrics::collect(&settings.metrics),
                    )
                }
                .boxed()
            }),
        });

        #[cfg(all(target_os = "linux", feature = "memory-profiling"))]
        self.set(TelemetryServerRoute {
            path: "/pprof/heap".into(),
            methods: vec![Method::GET],
            handler: Box::new(|_, settings| {
                async move {
                    into_response(
                        "application/x-gperftools-profile",
                        memory_profiling::heap_profile(settings).await,
                    )
                }
                .boxed()
            }),
        });

        #[cfg(all(target_os = "linux", feature = "memory-profiling"))]
        self.set(TelemetryServerRoute {
            path: "/pprof/heap_stats".into(),
            methods: vec![Method::GET],
            handler: Box::new(|_, settings| {
                async move {
                    into_response(
                        "text/plain; charset=utf-8",
                        memory_profiling::heap_stats(settings).await,
                    )
                }
                .boxed()
            }),
        });

        #[cfg(feature = "tracing")]
        self.set(TelemetryServerRoute {
            path: "/debug/traces".into(),
            methods: vec![Method::GET],
            handler: Box::new(|_, _settings| {
                async move {
                    into_response(
                        "application/json; charset=utf-8",
                        Ok(tracing::get_active_traces()),
                    )
                }
                .boxed()
            }),
        });
    }

    fn set(&mut self, route: TelemetryServerRoute) {
        let handler = Arc::new(route.handler);

        for method in route.methods {
            self.0
                .entry(method)
                .or_default()
                .entry(route.path.clone())
                .or_insert(Arc::clone(&handler));
        }
    }
}

#[derive(Clone)]
pub(super) struct Router {
    routes: Arc<RouteMap>,
    settings: Arc<TelemetrySettings>,
}

impl Router {
    async fn handle_request(&self, req: Request<Body>) -> Response<Body> {
        let res = Response::builder();

        let Ok(path) = percent_decode_str(req.uri().path()).decode_utf8() else {
            return res
                .status(StatusCode::BAD_REQUEST)
                .body("can't percent-decode URI path as valid UTF-8".into())
                .unwrap();
        };

        let Some(handler) = self
            .routes
            .0
            .get(req.method())
            .and_then(|e| e.get(&path.to_string()))
        else {
            return res.status(StatusCode::NOT_FOUND).body("".into()).unwrap();
        };

        match (handler)(req, Arc::clone(&self.settings)).await {
            Ok(res) => res,
            Err(e) => match e {},
        }
    }
}

impl Service<&AddrStream> for Router {
    type Response = Self;
    type Error = Infallible;
    type Future = Ready<std::result::Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _conn: &AddrStream) -> Self::Future {
        ready(Ok(self.clone()))
    }
}

impl Service<Request<Body>> for Router {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Pin<
        Box<dyn Future<Output = std::result::Result<Self::Response, Self::Error>> + Send + 'static>,
    >;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<hyper::Body>) -> Self::Future {
        let router = self.clone();

        async move { Ok(router.handle_request(req).await) }.boxed()
    }
}

pub(super) fn init(
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
        memory_profiling::profiler(Arc::clone(&settings)).map_err(|err| anyhow::anyhow!(err))?;
    }

    let addr = settings.server.addr;

    #[cfg(feature = "settings")]
    let addr = SocketAddr::from(addr);

    let router = Router {
        routes: Arc::new(RouteMap::new(custom_routes)),
        settings,
    };

    let socket = TcpListener::from(
        bind_socket(addr).with_context(|| format!("binding to socket {addr:?}"))?,
    );

    let builder = Server::from_tcp(socket)?;

    Ok(Some(builder.serve(router)))
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

fn into_response(
    content_type: &str,
    res: crate::Result<impl Into<Body>>,
) -> std::result::Result<Response<Body>, Infallible> {
    Ok(match res {
        Ok(data) => Response::builder()
            .header(header::CONTENT_TYPE, content_type)
            .body(data.into())
            .unwrap(),
        Err(err) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(err.to_string().into())
            .unwrap(),
    })
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
