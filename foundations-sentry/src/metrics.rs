//! Sentry event related metrics.

use foundations::telemetry::metrics::{Counter, metrics};
use sentry_core::Level;

/// Sentry metrics.
#[metrics(unprefixed)]
pub mod sentry {
    /// Total number of sentry events observed.
    pub fn events_total(level: Level) -> Counter;
}
