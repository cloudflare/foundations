//! Telemetry settings.

#[cfg(feature = "tracing")]
mod tracing;

#[cfg(feature = "logging")]
mod logging;

#[cfg(feature = "tracing")]
pub use self::tracing::*;

#[cfg(feature = "logging")]
pub use self::logging::*;

#[cfg(feature = "settings")]
use crate::settings::settings;

#[cfg(all(feature = "telemetry-server", feature = "settings"))]
use crate::settings::net::SocketAddr;
#[cfg(all(feature = "telemetry-server", not(feature = "settings")))]
use std::net::SocketAddr;

#[cfg(feature = "telemetry-server")]
use std::net::Ipv4Addr;

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

    /// Server settings.
    #[cfg(feature = "telemetry-server")]
    pub server: TelemetryServerSettings,
}

/// Metrics settings.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Default, Debug))]
pub struct MetricsSettings {
    /// Whether to report optional metrics in the telemetry server.
    pub report_optional: bool,
}

/// Telemetry server settings.
#[cfg(feature = "telemetry-server")]
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug))]
pub struct TelemetryServerSettings {
    /// Enables telemetry server
    pub enabled: bool,

    /// Telemetry server address.
    #[cfg_attr(
        feature = "settings",
        serde(default = "TelemetryServerSettings::default_server_addr")
    )]
    pub addr: SocketAddr,
}

#[cfg(feature = "telemetry-server")]
#[cfg(not(feature = "settings"))]
impl Default for TelemetryServerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            addr: Self::default_server_addr(),
        }
    }
}

#[cfg(feature = "telemetry-server")]
impl TelemetryServerSettings {
    fn default_server_addr() -> SocketAddr {
        let default_addr: std::net::SocketAddr = (Ipv4Addr::LOCALHOST, 6831).into();

        #[cfg(feature = "settings")]
        let default_addr = default_addr.into();

        default_addr
    }
}

fn _assert_traits_implemented_for_all_features() {
    fn assert<S: std::fmt::Debug + Clone + Default>() {}

    assert::<TelemetrySettings>();
}
