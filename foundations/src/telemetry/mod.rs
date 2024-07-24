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
mod telemetry_context;

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

feature_use!(cfg(feature = "tracing"), {
    use self::tracing::SpanScope;

    feature_use!(cfg(feature = "testing"), {
        use self::tracing::testing::TestTracerScope;
    });
});

#[cfg(feature = "logging")]
use self::log::internal::LogScope;

#[cfg(feature = "testing")]
pub use self::testing::TestTelemetryContext;

#[cfg(all(target_os = "linux", feature = "memory-profiling"))]
pub use self::memory_profiler::MemoryProfiler;

#[cfg(feature = "telemetry-server")]
pub use self::server::{TelemetryRouteHandler, TelemetryRouteHandlerFuture, TelemetryServerRoute};

pub use self::driver::TelemetryDriver;
pub use self::telemetry_context::{
    TelemetryContext, WithTelemetryContext, WithTelemetryContextLocal,
};

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
