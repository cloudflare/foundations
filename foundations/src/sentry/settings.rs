use std::num::NonZeroU32;

#[cfg(feature = "settings")]
use crate::settings::settings;

/// Sentry hook settings.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Default, Debug))]
pub struct SentrySettings {
    /// Maximum number of events that can be emitted per second, per fingerprint.
    pub max_events_per_second: Option<NonZeroU32>,
    // In the future, we may offer different fingerprinting modes here.
    // For example, we could offer a "dummy" mode that assigns the same fingerprint
    // to all events so they all share a single rate limit.
}
