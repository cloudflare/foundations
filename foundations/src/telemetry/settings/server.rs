#[cfg(feature = "settings")]
use crate::settings::net::SocketAddr;
#[cfg(feature = "settings")]
use crate::settings::settings;
use std::net::Ipv4Addr;
#[cfg(not(feature = "settings"))]
use std::net::SocketAddr;

/// Telemetry server settings.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug))]
pub struct TelemetryServerSettings {
    /// Enables telemetry server
    #[cfg_attr(
        feature = "settings",
        serde(default = "TelemetryServerSettings::default_enabled")
    )]
    pub enabled: bool,

    /// Telemetry server address.
    #[cfg_attr(
        feature = "settings",
        serde(default = "TelemetryServerSettings::default_addr")
    )]
    pub addr: SocketAddr,
}

#[cfg(not(feature = "settings"))]
impl Default for TelemetryServerSettings {
    fn default() -> Self {
        Self {
            enabled: TelemetryServerSettings::default_enabled(),
            addr: TelemetryServerSettings::default_addr(),
        }
    }
}

impl TelemetryServerSettings {
    fn default_enabled() -> bool {
        true
    }

    fn default_addr() -> SocketAddr {
        let addr: std::net::SocketAddr = (Ipv4Addr::LOCALHOST, 0).into();

        #[cfg(feature = "settings")]
        let addr = addr.into();

        addr
    }
}
