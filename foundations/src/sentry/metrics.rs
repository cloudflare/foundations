//! Sentry event related metrics.

use super::Level;
use crate::telemetry::metrics::Counter;

/// Sentry metrics.
#[crate::telemetry::metrics::metrics(crate_path = "crate", unprefixed)]
pub mod sentry {
    /// Total number of sentry events observed.
    pub fn events_total(level: Level) -> Counter;
}
