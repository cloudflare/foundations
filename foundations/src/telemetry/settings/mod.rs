//! Telemetry settings.

#[cfg(feature = "tracing")]
mod otlp_output;

#[cfg(feature = "tracing")]
mod tracing;

#[cfg(feature = "logging")]
mod logging;

#[cfg(feature = "metrics")]
mod metrics;

#[cfg(all(target_os = "linux", feature = "memory-profiling"))]
mod memory_profiler;

#[cfg(any(feature = "logging", feature = "tracing" ))]
mod rate_limit;

#[cfg(feature = "telemetry-server")]
mod server;

#[cfg(feature = "tracing")]
pub use self::otlp_output::*;

#[cfg(feature = "tracing")]
pub use self::tracing::*;

#[cfg(feature = "logging")]
pub use self::logging::*;

#[cfg(feature = "metrics")]
pub use self::metrics::*;

#[cfg(all(target_os = "linux", feature = "memory-profiling"))]
pub use self::memory_profiler::*;

#[cfg(any(feature = "logging", feature = "tracing" ))]
pub use self::rate_limit::RateLimitingSettings;

#[cfg(feature = "telemetry-server")]
pub use self::server::*;

#[cfg(feature = "settings")]
use crate::settings::settings;

/// Telemetry settings.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Default, Debug))]
pub struct TelemetrySettings {
    /// Distributed tracing settings
    #[cfg(feature = "tracing")]
    pub tracing: TracingSettings,

    /// Logging settings.
    #[cfg(feature = "logging")]
    pub logging: LoggingSettings,

    /// Metrics settings.
    #[cfg(feature = "metrics")]
    pub metrics: MetricsSettings,

    /// Memory profiler settings
    #[cfg(all(target_os = "linux", feature = "memory-profiling"))]
    pub memory_profiler: MemoryProfilerSettings,

    /// Server settings.
    #[cfg(feature = "telemetry-server")]
    pub server: TelemetryServerSettings,
}

fn _assert_traits_implemented_for_all_features() {
    fn assert<S: std::fmt::Debug + Clone + Default>() {}

    assert::<TelemetrySettings>();
}
