//! Sentry event related metrics.

use foundations::telemetry::metrics::{Counter, metrics};
use sentry_core::Level;

/// Sentry metrics.
#[metrics(unprefixed)]
pub mod sentry {
    /// Total number of sentry events observed.
    pub fn events_total(level: Level) -> Counter;

    /// Total number of sentry events dropped by the rate limits, by the rate limit that dropped
    /// them: `fingerprint`, `global`, or `fatal`.
    pub fn events_rate_limited_total(level: Level, limit: &'static str) -> Counter;
}
