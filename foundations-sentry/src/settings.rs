use foundations::settings::settings;
use std::num::NonZeroU32;

/// Sentry hook settings.
#[settings]
pub struct SentrySettings {
    /// Maximum number of events that can be emitted per second, per fingerprint.
    pub max_events_per_second: Option<NonZeroU32>,
    // In the future, we may offer different fingerprinting modes here.
    // For example, we could offer a "dummy" mode that assigns the same fingerprint
    // to all events so they all share a single rate limit.
}
