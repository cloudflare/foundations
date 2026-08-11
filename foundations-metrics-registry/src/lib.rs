//! Process-global metric registry and Prometheus protobuf data model for
//! `foundations`.
//!
//! Most applications should use the sibling `foundations-metrics` crate, which
//! provides metric types, encoders, label serialisation, and collection. Use
//! this crate directly when you need to inspect registered metrics through
//! [`iter`].
//!
//! [`prometheus/client_model`]: https://github.com/prometheus/client_model
#![warn(missing_docs)]

mod encode_metric;
mod iter;
mod metadata;
mod registry;

pub mod proto;

pub use encode_metric::EncodeMetric;
pub use iter::{MetricsIter, RegisteredMetric};
pub use metadata::RegistrationMetadata;
pub use proto::MetricFamily;
pub use registry::{IntoMetrics, iter, register};
