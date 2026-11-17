use crate::addr::ListenAddr;
#[cfg(feature = "settings")]
use crate::settings::settings;

/// Telemetry server settings.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, serde::Deserialize))]
pub struct TelemetryServerSettings {
    /// Enables telemetry server.
    ///
    /// Defaults to `true`.
    #[serde(default = "TelemetryServerSettings::default_enabled")]
    pub enabled: bool,

    /// Telemetry server address.
    ///
    /// Defaults to `127.0.0.1:0`, a port the OS picks on the loopback interface.
    #[serde(default = "TelemetryServerSettings::default_addr")]
    pub addr: ListenAddr,
}

#[cfg(not(feature = "settings"))]
impl Default for TelemetryServerSettings {
    fn default() -> Self {
        Self {
            enabled: TelemetryServerSettings::default_enabled(),
            addr: ListenAddr::default(),
        }
    }
}

impl TelemetryServerSettings {
    fn default_enabled() -> bool {
        true
    }

    fn default_addr() -> ListenAddr {
        ListenAddr::default()
    }
}
