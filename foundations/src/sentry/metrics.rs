//! Sentry event related metrics.

use super::Level;
use crate::telemetry::metrics::Counter;

/// Sentry metrics.
#[crate::telemetry::metrics::metrics(crate_path = "crate", unprefixed)]
pub mod sentry_events {
    /// Total number of sentry events observed.
    pub fn total(level: Level) -> Counter;
}
