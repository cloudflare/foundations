#[cfg(feature = "settings")]
use crate::settings::settings;

/// Rate limiting settings for events
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, Default, serde::Deserialize))]
pub struct RateLimitingSettings {
    /// Whether to enable rate limiting of events
    pub enabled: bool,

    /// Maximum number of events that can be emitted per second
    pub max_events_per_second: u32,
}
