//! Service telemetry.
//!
//! Foundations provides telemetry functionality for:
//!
//! * logging
//! * distributed tracing (backed by [Jaeger])
//! * metrics (backed by [Prometheus])
//! * memory profiling (backed by [jemalloc])
//! * monitoring tokio runtimes
//!
//! The library strives to minimize the bootstrap code required to set up basic telemetry for a
//! service and provide ergonomic API for telemetry-related operations.
//!
//! # Initialization
//!
//! In production code telemetry needs to be initialized on the service start up (usually at the
//! begining of the `main` function) with the [`init`] function for it to be collected by the
//! external sinks.
//!
//! If syscall sandboxing is also being used (see [`crate::security`] for more details), telemetry
//! must be initialized prior to syscall sandboxing, since it uses syscalls during initialization
//! that it will not use later.
//!
//! # Telemetry context
//!
//! Foundations' telemetry is designed to not interfere with the production code, so you usually don't
//! need to carry log handles or tracing spans around. However, it is contextual, allowing different
//! code branches to have different telemetry contexts. For example, in an HTTP service, you may want
//! separate logs for each HTTP request. Contextual log fields are implicitly added to log records
//! and apply only to log records produced for each particular request.
//!
//! The [`TelemetryContext`] structure reflects this concept and contains information about the log
//! and tracing span used in the current code scope. The context doesn't need to be explicitly
//! created, and if the service doesn't need separate logs or traces for different code paths,
//! it is a process-wide singleton.
//!
//! However, in some cases, it may be desirable to have branching of telemetry information. In such
//! cases, new telemetry contexts can be created using the [`TelemetryContext::with_forked_trace`]
//! and [`TelemetryContext::with_forked_log`] methods. These contexts need to be manually propagated
//! to the destination code branches using methods like [`TelemetryContext::scope`] and
//! [`TelemetryContext::apply`].
//!
//! # Testing
//! Telemetry is an important part of the functionality for any production-grade services and
//! Foundations provides API for telemetry testing: special testing context can be created with
//! [`TelemetryContext::test`] method and the library provides a special [`with_test_telemetry`] macro
//! to enable telemetry testing in `#[test]` and `#[tokio::test]`.
//!
//! [Jaeger]: https://www.jaegertracing.io/
//! [Prometheus]: https://prometheus.io/
//! [jemalloc]: https://github.com/jemalloc/jemalloc

#[cfg(any(feature = "logging", feature = "tracing"))]
mod scope;

mod driver;

#[cfg(all(feature = "tracing", feature = "telemetry-otlp-grpc"))]
mod otlp_conversion;

#[cfg(feature = "testing")]
mod testing;

#[cfg(feature = "logging")]
pub mod log;

#[cfg(feature = "metrics")]
pub mod metrics;

#[cfg(feature = "tracing")]
pub mod tracing;

#[cfg(all(target_os = "linux", feature = "memory-profiling"))]
mod memory_profiler;

pub mod settings;

#[cfg(all(
    feature = "tokio-runtime-metrics",
    tokio_unstable,
    foundations_unstable
))]
#[cfg_attr(
    docsrs,
    doc(cfg(all(
        feature = "tokio-runtime-metrics",
        tokio_unstable,
        foundations_unstable
    )))
)]
pub mod tokio_runtime_metrics;

#[cfg(feature = "telemetry-server")]
mod server;

use self::settings::TelemetrySettings;
use crate::utils::feature_use;
use crate::{BootstrapResult, ServiceInfo};
use futures_util::stream::FuturesUnordered;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

feature_use!(cfg(feature = "logging"), {
    use self::log::internal::{current_log, fork_log, LogScope, SharedLog};
    use std::sync::Arc;
});

feature_use!(cfg(feature = "tracing"), {
    use self::tracing::internal::{create_span, current_span, fork_trace, SharedSpan};
    use self::tracing::SpanScope;
    use std::borrow::Cow;

    feature_use!(cfg(feature = "testing"), {
        use self::tracing::internal::Tracer;
        use self::tracing::testing::{current_test_tracer, TestTracerScope};
    });
});

#[cfg(feature = "testing")]
pub use self::testing::TestTelemetryContext;

#[cfg(all(target_os = "linux", feature = "memory-profiling"))]
pub use self::memory_profiler::MemoryProfiler;

#[cfg(feature = "telemetry-server")]
pub use self::server::{TelemetryRouteHandler, TelemetryRouteHandlerFuture, TelemetryServerRoute};

pub use self::driver::TelemetryDriver;

/// A macro that enables telemetry testing in `#[test]` and `#[tokio::test]`.
///
/// # Wrapping `#[test]`
/// ```
/// use foundations::telemetry::tracing::{self, test_trace};
/// use foundations::telemetry::{with_test_telemetry, TestTelemetryContext};
///
/// #[with_test_telemetry(test)]
/// fn sync_rust_test(ctx: TestTelemetryContext) {
///     {
///         let _span = tracing::span("root");
///     }
///
///     assert_eq!(
///         ctx.traces(Default::default()),
///         vec![test_trace! {
///             "root"
///         }]
///     );
/// }
/// ```
///
/// # Wrapping `#[tokio::test]`
/// ```
/// use foundations::telemetry::tracing::{self, test_trace};
/// use foundations::telemetry::{with_test_telemetry, TestTelemetryContext};
///
/// #[with_test_telemetry(tokio::test)]
/// async fn wrap_tokio_test(ctx: TestTelemetryContext) {
///     {
///         let _span = tracing::span("span1");
///     }
///
///     tokio::task::yield_now().await;
///
///     {
///         let _span = tracing::span("span2");
///     }
///
///     assert_eq!(
///         ctx.traces(Default::default()),
///         vec![
///             test_trace! {
///                 "span1"
///             },
///             test_trace! {
///                 "span2"
///             }
///         ]
///     );
/// }
/// ```
///
/// # Renamed or reexported crate
///
/// The macro will fail to compile if `foundations` crate is reexported. However, the crate path
/// can be explicitly specified for the macro to workaround that:
///
/// ```
/// mod reexport {
///     pub use foundations::*;
/// }
///
/// use reexport::telemetry::tracing::{self, test_trace};
/// use reexport::telemetry::{with_test_telemetry, TestTelemetryContext};
///
/// #[with_test_telemetry(test, crate_path = "reexport")]
/// fn sync_rust_test(ctx: TestTelemetryContext) {
///     {
///         let _span = tracing::span("root");
///     }
///
///     assert_eq!(
///         ctx.traces(Default::default()),
///         vec![test_trace! {
///             "root"
///         }]
///     );
/// }
/// ```
#[cfg(feature = "testing")]
pub use foundations_macros::with_test_telemetry;

/// Wrapper for a future that provides it with [`TelemetryContext`].
pub struct WithTelemetryContext<'f, T> {
    // NOTE: we intentionally erase type here as we can get close to the type
    // length limit, adding telemetry wrappers on top causes compiler to fail in some
    // cases.
    inner: Pin<Box<dyn Future<Output = T> + Send + 'f>>,
    ctx: TelemetryContext,
}

impl<'f, T> Future for WithTelemetryContext<'f, T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let _telemetry_scope = self.ctx.scope();

        self.inner.as_mut().poll(cx)
    }
}

/// A handle for the scope in which certain [`TelemetryContext`] is active.
///
/// Scope ends when the handle is dropped.
///
/// The handle is created with [`TelemetryContext::scope`] method.
#[must_use = "Telemetry context is not applied when scope is dropped."]
pub struct TelemetryScope {
    #[cfg(feature = "logging")]
    _log_scope: LogScope,

    #[cfg(feature = "tracing")]
    _span_scope: Option<SpanScope>,

    // NOTE: certain tracing APIs start a new trace, so we need to scope the test tracer
    // for them to use the tracer from the test scope instead of production tracer in
    // the harness.
    #[cfg(all(feature = "tracing", feature = "testing"))]
    _test_tracer_scope: Option<TestTracerScope>,
}

/// Implicit context for logging and tracing.
///
/// Current context can be obtained with the [`TelemetryContext::current`] method.
///
/// The context can be forked with [`TelemetryContext::with_forked_log`] and
/// [`TelemetryContext::with_forked_trace`] methods. And propagated in different scopes with
/// [`TelemetryContext::scope`] and [`TelemetryContext::apply`] methods.
#[derive(Debug, Clone)]
pub struct TelemetryContext {
    #[cfg(feature = "logging")]
    log: SharedLog,

    // NOTE: we might not have tracing root at this point
    #[cfg(feature = "tracing")]
    span: Option<SharedSpan>,

    #[cfg(all(feature = "tracing", feature = "testing"))]
    test_tracer: Option<Tracer>,
}

impl TelemetryContext {
    /// Returns the telemetry context that is active in the current scope.
    pub fn current() -> Self {
        Self {
            #[cfg(feature = "logging")]
            log: current_log(),

            #[cfg(feature = "tracing")]
            span: current_span(),

            #[cfg(all(feature = "tracing", feature = "testing"))]
            test_tracer: current_test_tracer(),
        }
    }

    /// Creates a scope handle for the telemetry context.
    ///
    /// The telemetry context is active in the scope unless the handle is dropped.
    ///
    /// Note that the scope can only be used in the sync contexts. To propagate telemetry context
    /// to async contexts, [`TelemetryContext::apply`] should be used instead.
    ///
    /// The scope handle is useful to propagate the telemetry context to callbacks provided
    /// by third party libraries or other threads.
    ///
    /// The other use case is situations where code control flow doesn't align with telemetry flow.
    /// E.g. there is a centralized dispatcher that drives certain tasks and tasks are registered
    /// with the dispatcher via callbacks. It's desirable for each task to have its own telemetry
    /// context, so scope can be used to propagate task context to the dispatcher's callbacks.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::TelemetryContext;
    /// use foundations::telemetry::tracing::{self, test_trace};
    ///
    /// // Test context is used for demonstration purposes to show the resulting traces.
    /// let ctx = TelemetryContext::test();
    ///
    /// {
    ///     let _scope = ctx.scope();
    ///     let _root = tracing::span("root");
    ///     let telemetry_ctx = TelemetryContext::current();
    ///
    ///     let handle = std::thread::spawn(move || {
    ///         let _scope = telemetry_ctx.scope();
    ///         let _child = tracing::span("child");
    ///     });
    ///
    ///     handle.join();
    /// }
    ///
    /// assert_eq!(
    ///     ctx.traces(Default::default()),
    ///     vec![
    ///         test_trace! {
    ///             "root" => {
    ///                 "child"
    ///             }
    ///         },
    ///     ]
    /// );
    /// ```
    pub fn scope(&self) -> TelemetryScope {
        TelemetryScope {
            #[cfg(feature = "logging")]
            _log_scope: LogScope::new(Arc::clone(&self.log)),

            #[cfg(feature = "tracing")]
            _span_scope: self.span.as_ref().cloned().map(SpanScope::new),

            #[cfg(all(feature = "tracing", feature = "testing"))]
            _test_tracer_scope: self.test_tracer.as_ref().cloned().map(TestTracerScope::new),
        }
    }

    /// Creates a test telemetry context.
    ///
    /// Returned context has the same API as standard context, but also exposes API to obtain the
    /// telemetry collected in it.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::TelemetryContext;
    /// use foundations::telemetry::tracing::{self, test_trace};
    /// use foundations::telemetry::log::{self, TestLogRecord};
    /// use foundations::telemetry::settings::Level;
    ///
    /// #[tracing::span_fn("sync_fn")]
    /// fn some_sync_production_fn_that_we_test() {
    ///     log::warn!("Sync hello!");
    /// }
    ///
    /// #[tracing::span_fn("async_fn")]
    /// async fn some_async_production_fn_that_we_test() {
    ///     log::warn!("Async hello!");
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let ctx = TelemetryContext::test();
    ///     
    ///     {
    ///         let _scope = ctx.scope();
    ///         let _root = tracing::span("root");
    ///
    ///         let handle = tokio::spawn(TelemetryContext::current().apply(async {
    ///             some_async_production_fn_that_we_test().await;
    ///         }));
    ///
    ///         handle.await;
    ///
    ///         some_sync_production_fn_that_we_test();
    ///     }
    ///
    ///     assert_eq!(*ctx.log_records(), &[
    ///         TestLogRecord {
    ///             level: Level::Warning,
    ///             message: "Async hello!".into(),
    ///             fields: vec![]
    ///         },
    ///         TestLogRecord {
    ///             level: Level::Warning,
    ///             message: "Sync hello!".into(),
    ///             fields: vec![]
    ///         }
    ///     ]);  
    ///
    ///     assert_eq!(
    ///         ctx.traces(Default::default()),
    ///         vec![
    ///             test_trace! {
    ///                 "root" => {
    ///                     "async_fn",
    ///                     "sync_fn"
    ///                 }
    ///             }
    ///         ]
    ///     );
    /// }
    /// ```
    #[cfg(feature = "testing")]
    pub fn test() -> TestTelemetryContext {
        TestTelemetryContext::new()
    }

    /// Wraps a future with the telemetry context.
    ///
    /// [`TelemetryScope`] can't be used across `await` points to propagate the telemetry context,
    /// so to use telemetry context in async blocks, futures should be wrapped using this
    /// method instead.
    ///
    /// Note that you don't need to use this method to wrap async function's bodies,
    /// as [`tracing::span_fn`] macro takes care of that.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::TelemetryContext;
    /// use foundations::telemetry::tracing::{self, test_trace};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // Test context is used for demonstration purposes to show the resulting traces.
    ///     let ctx = TelemetryContext::test();
    ///
    ///     {
    ///         let _scope = ctx.scope();
    ///         let _root = tracing::span("root");
    ///
    ///         let handle = tokio::spawn(
    ///             TelemetryContext::current().apply(async {
    ///                 let _child = tracing::span("child");
    ///             })
    ///         );
    ///
    ///         handle.await;
    ///     }
    ///
    ///     assert_eq!(
    ///         ctx.traces(Default::default()),
    ///         vec![
    ///             test_trace! {
    ///                 "root" => {
    ///                     "child"
    ///                 }
    ///             }
    ///         ]
    ///     );
    /// }
    /// ```
    pub fn apply<'f, F>(&self, fut: F) -> WithTelemetryContext<'f, F::Output>
    where
        F: Future + Send + 'f,
    {
        WithTelemetryContext {
            inner: Box::pin(fut),
            ctx: self.clone(),
        }
    }
}

#[cfg(feature = "tracing")]
impl TelemetryContext {
    /// Creates a new telemetry context, that includes a forked trace, creating a
    /// linked child trace.
    ///
    /// If the current trace is sampled, the new child trace also will be sampled.
    /// If the current trace isn't sampled, no new child trace is created.
    ///
    /// This method is useful to avoid a single trace from ballooning in size
    /// while still keeping navigability from the source trace to the child
    /// traces and vice-versa.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::TelemetryContext;
    /// use foundations::telemetry::tracing::{self, test_trace};
    ///
    /// // Test context is used for demonstration purposes to show the resulting traces.
    /// let ctx = TelemetryContext::test();
    ///
    /// {
    ///     let _scope = ctx.scope();
    ///     let _root = tracing::span("root");
    ///
    ///     {
    ///         let _span1 = tracing::span("span1");
    ///     }
    ///
    ///     let _scope = TelemetryContext::current()
    ///         .with_forked_trace("new fork")
    ///         .scope();
    ///
    ///     let _span2 = tracing::span("span2");
    /// }
    ///
    /// assert_eq!(
    ///     ctx.traces(Default::default()),
    ///     vec![
    ///         test_trace! {
    ///             "root" => {
    ///                 "span1",
    ///                 "[new fork ref]"
    ///             }
    ///         },
    ///         test_trace! {
    ///             "new fork" => {
    ///                 "span2"
    ///             }
    ///         }
    ///     ]
    /// );
    pub fn with_forked_trace(&self, fork_name: impl Into<Cow<'static, str>>) -> Self {
        Self {
            #[cfg(feature = "logging")]
            log: Arc::clone(&self.log),

            span: Some(fork_trace(fork_name)),

            #[cfg(feature = "testing")]
            test_tracer: self.test_tracer.clone(),
        }
    }

    /// Provides the same functionality as [`TelemetryContext::apply`], but also creates a tracing
    /// span that is active during the future execution.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::TelemetryContext;
    /// use foundations::telemetry::tracing::{self, test_trace};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // Test context is used for demonstration purposes to show the resulting traces.
    ///     let ctx = TelemetryContext::test();
    ///
    ///     {
    ///         let _scope = ctx.scope();
    ///         let _root = tracing::span("root");
    ///
    ///         let handle = tokio::spawn(
    ///             TelemetryContext::current().apply_with_tracing_span("future", async {
    ///                 let _child = tracing::span("child");
    ///             })
    ///         );
    ///
    ///         handle.await;
    ///     }
    ///
    ///     assert_eq!(
    ///         ctx.traces(Default::default()),
    ///         vec![
    ///             test_trace! {
    ///                 "root" => {
    ///                     "future" => {
    ///                         "child"
    ///                     }
    ///                 }
    ///             }
    ///         ]
    ///     );
    /// }
    /// ```
    pub fn apply_with_tracing_span<'f, F, N>(
        &self,
        span_name: N,
        fut: F,
    ) -> WithTelemetryContext<'f, F::Output>
    where
        F: Future + Send + 'f,
        N: Into<Cow<'static, str>>,
    {
        let mut ctx = self.clone();
        let _scope = ctx.span.as_ref().cloned().map(SpanScope::new);

        ctx.span = Some(create_span(span_name));

        WithTelemetryContext {
            inner: Box::pin(fut),
            ctx,
        }
    }
}

#[cfg(feature = "logging")]
impl TelemetryContext {
    /// Creates a telemetry context with log that is detached from the current context's log, but
    /// inherits its log fields.
    ///
    /// For example, can be used in server software to produce separate logs for HTTP requests, each
    /// of which has log fields added during the HTTP connection establishment.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::TelemetryContext;
    /// use foundations::telemetry::log::{self, TestLogRecord};
    /// use foundations::telemetry::settings::Level;
    ///
    /// // Test context is used for demonstration purposes to show the resulting log records.
    /// let ctx = TelemetryContext::test();
    /// let _scope = ctx.scope();
    ///
    /// log::add_fields!("conn_field" => 42);
    ///
    /// {
    ///     let _scope = TelemetryContext::current().with_forked_log().scope();
    ///
    ///     log::add_fields!("req1_field" => "foo");
    ///     log::warn!("Hello from request 1");
    /// }
    ///
    /// {
    ///     let _scope = TelemetryContext::current().with_forked_log().scope();
    ///
    ///     log::add_fields!("req2_field" => "bar");
    ///     log::warn!("Hello from request 2");
    /// }
    ///
    /// log::warn!("Hello from connection");
    ///
    /// assert_eq!(*ctx.log_records(), &[
    ///     TestLogRecord {
    ///         level: Level::Warning,
    ///         message: "Hello from request 1".into(),
    ///         fields: vec![
    ///             ("req1_field".into(), "foo".into()),
    ///             ("conn_field".into(), "42".into()),
    ///         ]
    ///     },
    ///     TestLogRecord {
    ///         level: Level::Warning,
    ///         message: "Hello from request 2".into(),
    ///         fields: vec![
    ///             ("req2_field".into(), "bar".into()),
    ///             ("conn_field".into(), "42".into()),
    ///         ]
    ///     },
    ///     TestLogRecord {
    ///         level: Level::Warning,
    ///         message: "Hello from connection".into(),
    ///         fields: vec![
    ///             ("conn_field".into(), "42".into()),
    ///         ]
    ///     }
    /// ]);
    /// ```
    pub fn with_forked_log(&self) -> Self {
        Self {
            log: fork_log(),

            #[cfg(feature = "tracing")]
            span: self.span.clone(),

            #[cfg(all(feature = "tracing", feature = "testing"))]
            test_tracer: self.test_tracer.clone(),
        }
    }
}

/// Telemetry configuration that is passed to [`init`].
pub struct TelemetryConfig<'c> {
    /// Service information that is used in telemetry reporting.
    ///
    /// Can be obtained using [`crate::service_info`] macro.
    pub service_info: &'c ServiceInfo,

    /// Telemetry settings.
    pub settings: &'c TelemetrySettings,

    /// Custom telemetry server routes.
    ///
    /// Refer to the [`init`] documentation to learn more about the telemetry server.
    #[cfg(feature = "telemetry-server")]
    pub custom_server_routes: Vec<TelemetryServerRoute>,
}

/// Initializes service telemetry.
///
/// The function sets up telemetry collection endpoints and other relevant settings. The function
/// doesn't need to be called in tests and any specified settings will be ignored in test
/// environments. Instead, all the telemetry will be collected in the [`TestTelemetryContext`].
///
/// The function should be called once on service initialization (prior to any [syscall sandboxing]). Consequent calls to the function
/// don't have any effect.
///
/// # Telemetry server
///
/// Foundations can expose optional server endpoint to serve telemetry-related information if
/// [`TelemetryServerSettings::enabled`] is set to `true`.
///
/// The server exposes the following URL paths:
/// - `/health` - telemetry server healtcheck endpoint, returns `200 OK` response if server is functional.
/// - `/metrics` - returns service metrics in [Prometheus text format] (requires **metrics** feature).
/// - `/pprof/heap` - returns [jemalloc] heap profile (requires **memory-profiling** feature).
/// - `/pprof/heap_stats` returns [jemalloc] heap stats (requires **memory-profiling** feature).
///
/// Additional custom routes can be added via [`TelemetryConfig::custom_server_routes`].
///
/// [Prometheus text format]: https://prometheus.io/docs/instrumenting/exposition_formats/#text-based-format
/// [jemalloc]: https://github.com/jemalloc/jemalloc
/// [`TelemetryServerSettings::enabled`]: `crate::telemetry::settings::TelemetryServerSettings::enabled`
/// [syscall sandboxing]: `crate::security`
pub fn init(config: TelemetryConfig) -> BootstrapResult<TelemetryDriver> {
    let tele_futures: FuturesUnordered<_> = Default::default();

    #[cfg(feature = "logging")]
    self::log::init::init(config.service_info, &config.settings.logging)?;

    #[cfg(feature = "tracing")]
    {
        if let Some(reporter_fut) =
            self::tracing::init::init(config.service_info.clone(), &config.settings.tracing)?
        {
            tele_futures.push(reporter_fut);
        }
    }

    #[cfg(feature = "metrics")]
    self::metrics::init::init(config.service_info, &config.settings.metrics);

    #[cfg(feature = "telemetry-server")]
    {
        let server_fut = self::server::init(config.settings.clone(), config.custom_server_routes)?;

        Ok(TelemetryDriver::new(server_fut, tele_futures))
    }

    #[cfg(not(feature = "telemetry-server"))]
    Ok(TelemetryDriver::new(tele_futures))
}
