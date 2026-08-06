//! The evolving layer of the `foundations` metrics stack.
//!
//! This crate provides the concrete metric types ([`Counter`], [`Gauge`], ...)
//! and the logic that encodes them into the Prometheus protobuf data model. It
//! builds on the slow-moving `foundations-metrics-registry` crate, which owns the
//! shared process-global registry and the stable wire format.
#![warn(missing_docs)]

mod collect;
mod diagnostics;
mod encoding;
mod info;
mod labels;
pub mod metrics;
mod registered;
mod validation;
mod value;

pub use collect::{CollectionOptions, ServiceNameFormat, collect};
pub use diagnostics::{CollectErrorHookAlreadySet, set_collect_error_hook};
pub use encoding::{
    OPENMETRICS_CONTENT_TYPE, PROTOBUF_CONTENT_TYPE, encode_to_protobuf, encode_to_text,
};
pub use foundations_metrics_registry::{
    EncodeMetric, IntoMetrics, MetricFamily, RegistrationMetadata, proto, register,
};
pub use info::{InfoMetric, report_info};
pub use labels::{LabelError, to_label_pairs};
pub use metrics::{
    Counter, CounterAtomic, Family, FamilyMetricGuard, Gauge, GaugeAtomic, GaugeGuard, Histogram,
    HistogramBuilder, HistogramSnapshot, HistogramTimer, MetricConstructor, NativeHistogram,
    NativeHistogramBuilder, RangeGauge, TimeHistogram, WithExemplar,
};
pub use registered::NamedMetric;
pub use validation::{NAME_REQUIREMENT, is_valid_name};
pub use value::EncodeMetricValue;
