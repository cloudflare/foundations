#[cfg(feature = "settings")]
use crate::settings::net::SocketAddr;
#[cfg(feature = "settings")]
use crate::settings::settings;
use std::net::Ipv4Addr;
#[cfg(not(feature = "settings"))]
use std::net::SocketAddr;

/// Telemetry server settings.
#[cfg_attr(
    feature = "settings",
    settings(crate_path = "crate", impl_default = false)
)]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug))]
pub struct TelemetryServerSettings {
    /// Enables telemetry server
    pub enabled: bool,

    /// Telemetry server address.
    pub addr: SocketAddr,
}

impl Default for TelemetryServerSettings {
    fn default() -> Self {
        let addr: std::net::SocketAddr = (Ipv4Addr::LOCALHOST, 0).into();

        #[cfg(feature = "settings")]
        let addr = addr.into();

        Self {
            enabled: true,
            addr,
        }
    }
}
