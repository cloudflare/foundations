//! Panic-related metrics.

use crate::telemetry::metrics::Counter;

/// Panic metrics.
#[crate::telemetry::metrics::metrics(crate_path = "crate", unprefixed)]
pub mod panics {
    /// Total number of panics observed.
    pub fn total() -> Counter;
}
