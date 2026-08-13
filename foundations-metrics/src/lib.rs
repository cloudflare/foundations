//! Metric types, collection, and Prometheus encoders for `foundations`.
//!
//! This crate provides counters, gauges, histograms, metric families, and
//! encoders for OpenMetrics text and Prometheus protobuf formats. It uses
//! `foundations-metrics-registry` for the process-global registry and protobuf
//! data model.
//!
//! `foundations` currently re-exports these APIs from
//! `foundations::telemetry::metrics`. Libraries can depend on this crate
//! directly, to maximize compatibility with foundations' versions. However,
//! binaries which use foundations should still use the top-level `foundations`
//! crate. See the crate README for migration instructions.
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
    NativeHistogramBuilder, NativeTimeHistogram, RangeGauge, TimeHistogram, WithExemplar,
};
pub use registered::NamedMetric;
pub use validation::{NAME_REQUIREMENT, is_valid_name};
pub use value::EncodeMetricValue;
