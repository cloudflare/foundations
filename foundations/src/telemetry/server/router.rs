#[cfg(all(target_os = "linux", feature = "memory-profiling"))]
use super::memory_profiling;
#[cfg(feature = "metrics")]
use crate::telemetry::metrics;
use crate::telemetry::reexports::http_body_util::{BodyExt, Empty, Full, combinators::BoxBody};
use crate::telemetry::settings::TelemetrySettings;
#[cfg(feature = "tracing")]
use crate::telemetry::tracing;
use futures_util::future::{BoxFuture, FutureExt};
use hyper::body::{Bytes, Incoming};
use hyper::service::Service;
use hyper::{Method, Request, Response, StatusCode, header};
use percent_encoding::percent_decode_str;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

/// Body type used in [`TelemetryServerRoute`] responses.
pub type TelemetryRouteBody = BoxBody<Bytes, crate::Error>;

/// Future returned by [`TelemetryServerRoute::handler`].
pub type TelemetryRouteHandlerFuture =
    BoxFuture<'static, Result<Response<TelemetryRouteBody>, Infallible>>;

/// Telemetry route handler.
pub type TelemetryRouteHandler = Box<
    dyn Fn(Request<Incoming>, Arc<TelemetrySettings>) -> TelemetryRouteHandlerFuture
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
    pub(super) fn new(
        custom_routes: Vec<TelemetryServerRoute>,
        settings: Arc<TelemetrySettings>,
    ) -> Self {
        Self {
            routes: Arc::new(RouteMap::new(custom_routes)),
            settings,
        }
    }

    async fn handle_request(&self, req: Request<Incoming>) -> Response<TelemetryRouteBody> {
        let res = Response::builder();

        let Ok(path) = percent_decode_str(req.uri().path()).decode_utf8() else {
            return res
                .status(StatusCode::BAD_REQUEST)
                .body(BoxBody::new(
                    Full::from("can't percent-decode URI path as valid UTF-8").map_err(Into::into),
                ))
                .unwrap();
        };

        let Some(handler) = self
            .routes
            .0
            .get(req.method())
            .and_then(|e| e.get(&path.to_string()))
        else {
            return res
                .status(StatusCode::NOT_FOUND)
                .body(BoxBody::new(Empty::new().map_err(Into::into)))
                .unwrap();
        };

        match (handler)(req, Arc::clone(&self.settings)).await {
            Ok(res) => res,
            Err(e) => match e {},
        }
    }
}

impl Service<Request<Incoming>> for Router {
    type Response = Response<TelemetryRouteBody>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        let router = self.clone();

        async move { Ok(router.handle_request(req).await) }.boxed()
    }
}

fn into_response(
    content_type: &str,
    res: crate::Result<impl Into<Full<Bytes>>>,
) -> Result<Response<TelemetryRouteBody>, Infallible> {
    Ok(match res {
        Ok(data) => Response::builder()
            .header(header::CONTENT_TYPE, content_type)
            .body(BoxBody::new(data.into().map_err(Into::into)))
            .unwrap(),
        Err(err) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(BoxBody::new(
                Full::from(err.to_string()).map_err(Into::into),
            ))
            .unwrap(),
    })
}
