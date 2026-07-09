#[cfg(all(target_os = "linux", feature = "memory-profiling"))]
use super::memory_profiling;
#[cfg(feature = "memory-profiling")]
use super::pprof_symbol;
use crate::BootstrapResult;
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
type RouteHandlerShared = Arc<
    dyn Fn(Request<Incoming>, Arc<TelemetrySettings>) -> TelemetryRouteHandlerFuture
        + Send
        + Sync
        + 'static,
>;

/// A telemetry server route descriptor.
///
/// There can be only one route handler per (Method, Path) pair. If there is a
/// collision, the first route to be inserted wins. This includes built-in routes,
/// which are inserted before any custom routes. The current set of built-in routes
/// are:
/// - `/health`
/// - `/metrics` (`metrics` feature)
/// - `/pprof/heap` (`memory-profiling` feature)
/// - `/pprof/heap_stats` (`memory-profiling` feature)
/// - `/pprof/symbol` (`memory-profiling` feature)
/// - `/debug/traces` (`tracing` feature)
///
/// New built-in routes may be added from time to time. We reserve the `/foundations/`
/// prefix for this purpose, but other paths may be used if there are existing conventions
/// for a feature (such as `/pprof/*` and `/metrics`.)
///
/// # Pattern-Based Routing
///
/// If the optional `telemetry-server-pattern-routing` feature is enabled,
/// [`TelemetryServerRoute`]s can include parameters in their `path` attributes.
/// (If the feature is disabled, `path` is interpreted entirely literally.) Parameters
/// are delimited by single curly braces, i.e. `{my_param}`. To include a literal curly
/// brace in a pattern-based path, escape it by duplicating it: `{ -> {{` and `} -> }}`.
///
/// For routing, paths are split into segments delimited by `/`. Each segment may be
/// either:
/// - A static string, like `/foo`.
/// - A named parameter, like `/{my_param}`. Prefixes and suffixes can be added as
///   well (`/foo{my_param}bar`).
/// - A catch-all parameter, like `/{*rest}`. This is only allowed at the end of the
///   path.
///
/// Note that this allows patterns to be overlapping. For the exact details on conflict
/// handling and priority, see the [`matchit`](matchit#conflict-rules) docs. In essence,
/// static segments take precedence over parameters, and named parameters take precedence
/// over catch-all parameters.
///
/// We do not pass the parsed parameters to the `handler` function (to keep the route
/// interface as simply as possible.) If necessary, the handler has to re-parse the
/// request's path itself.
pub struct TelemetryServerRoute {
    /// URL path of the route.
    pub path: String,

    /// A list of HTTP methods for which this route is active.
    pub methods: Vec<Method>,

    /// A route handler.
    pub handler: TelemetryRouteHandler,
}

struct Routes(HashMap<Method, matchit::Router<RouteHandlerShared>>);

impl Routes {
    fn new(custom_routes: Vec<TelemetryServerRoute>) -> BootstrapResult<Self> {
        let mut map = Self(Default::default());

        map.init_built_in_routes()?;

        for route in custom_routes {
            map.set(route)?;
        }

        Ok(map)
    }

    fn init_built_in_routes(&mut self) -> BootstrapResult<()> {
        self.set(TelemetryServerRoute {
            path: "/health".into(),
            methods: vec![Method::GET],
            handler: Box::new(|_, _| async { into_response("text/plain", Ok("")) }.boxed()),
        })?;

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
        })?;

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
        })?;

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
        })?;

        #[cfg(feature = "memory-profiling")]
        self.set(TelemetryServerRoute {
            path: "/pprof/symbol".into(),
            methods: vec![Method::GET, Method::POST],
            handler: Box::new(|req, _| {
                async move {
                    into_response(
                        "text/plain; charset=utf-8",
                        pprof_symbol::pprof_symbol(req).await,
                    )
                }
                .boxed()
            }),
        })?;

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
        })?;

        Ok(())
    }

    #[allow(unused_mut, reason = "conditional mutation")]
    fn set(&mut self, mut route: TelemetryServerRoute) -> BootstrapResult<()> {
        let handler = Arc::from(route.handler);

        #[cfg(not(feature = "telemetry-server-pattern-routing"))]
        {
            // Escape parameter delimiters so `matchit` interprets them literally
            route.path = route.path.replace('{', "{{").replace('}', "}}");
        }

        for method in route.methods {
            let res = self
                .0
                .entry(method)
                .or_default()
                .insert(route.path.clone(), Arc::clone(&handler));

            match res {
                Ok(()) => {}
                // Exact matches were allowed and ignored historically. Any other errors
                // should be reported up.
                Err(matchit::InsertError::Conflict { with }) if with == route.path => {}
                Err(e) => anyhow::bail!("tried to insert route `{}`, but {}", route.path, e),
            }
        }

        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct Router {
    routes: Arc<Routes>,
    settings: Arc<TelemetrySettings>,
}

impl Router {
    pub(super) fn new(
        custom_routes: Vec<TelemetryServerRoute>,
        settings: Arc<TelemetrySettings>,
    ) -> BootstrapResult<Self> {
        Ok(Self {
            routes: Arc::new(Routes::new(custom_routes)?),
            settings,
        })
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
            .and_then(|r| Some(r.at(&path).ok()?.value))
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
