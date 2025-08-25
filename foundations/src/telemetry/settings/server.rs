use crate::addr::ListenAddr;
#[cfg(feature = "settings")]
use crate::settings::settings;

/// Telemetry server settings.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, serde::Deserialize))]
pub struct TelemetryServerSettings {
    /// Enables telemetry server
    #[serde(default = "TelemetryServerSettings::default_enabled")]
    pub enabled: bool,

    /// Telemetry server address.
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
