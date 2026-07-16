//! The evolving layer of the `foundations` metrics stack.
//!
//! This crate provides the concrete metric types ([`Counter`], [`Gauge`], ...)
//! and the logic that encodes them into the Prometheus protobuf data model. It
//! builds on the slow-moving `foundations-metrics-registry` crate, which owns the
//! shared process-global registry and the stable wire format.
#![warn(missing_docs)]

mod diagnostics;
mod labels;
pub mod metrics;
mod registered;
mod value;

pub use diagnostics::{CollectErrorHookAlreadySet, set_collect_error_hook};
pub use foundations_metrics_registry::{
    EncodeMetric, IntoMetrics, MetricFamily, RegistrationMetadata, register,
};
pub use labels::{LabelError, to_label_pairs};
pub use metrics::{
    Counter, CounterAtomic, Family, FamilyMetricGuard, Gauge, GaugeAtomic, GaugeGuard, Histogram,
    HistogramBuilder, HistogramTimer, MetricConstructor, RangeGauge, TimeHistogram,
};
pub use registered::NamedMetric;
