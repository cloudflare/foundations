//! Tracing-related metrics.

use crate::telemetry::metrics::{Counter, Gauge};

/// Tracing metrics.
#[crate::telemetry::metrics::metrics(crate_path = "crate", unprefixed)]
pub mod tracing {
    /// Current size of the span consumer queue.
    pub fn queue_size() -> Gauge;

    /// Maximum allowed size of the span consumer queue. `usize::MAX` for
    /// unbounded queues.
    pub fn max_queue_size() -> Gauge;

    /// Total number of spans produced.
    pub fn spans_total() -> Counter;

    /// Total number of spans dropped because the consumer queue was full.
    pub fn spans_dropped() -> Counter;
}
