//! The evolving layer of the `foundations` metrics stack.
//!
//! This crate provides the concrete metric types ([`Counter`], [`Gauge`], ...)
//! and the logic that encodes them into the Prometheus protobuf data model. It
//! builds on the slow-moving `foundations-metrics-registry` crate, which owns the
//! shared process-global registry and the stable wire format.
#![warn(missing_docs)]

pub mod metrics;
mod registered;
mod value;

pub use metrics::{Counter, CounterAtomic, Gauge, GaugeAtomic, GaugeGuard, RangeGauge};
pub use registered::NamedMetric;
