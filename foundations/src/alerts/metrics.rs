//! Panic and sentry event related metrics.

use crate::telemetry::metrics::Counter;

/// Panic metrics.
#[crate::telemetry::metrics::metrics(crate_path = "crate", unprefixed)]
pub mod panics {
    /// Total number of panics observed.
    pub fn total() -> Counter;
}

/// Sentry metrics.
#[crate::telemetry::metrics::metrics(crate_path = "crate", unprefixed)]
pub mod sentry_events {
    /// Total number of sentry events observed.
    pub fn total() -> Counter;
}
