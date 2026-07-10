//! The stable core of the `foundations` metrics stack.
//!
//! `foundations-metrics-registry` holds the parts of the metrics stack that must
//! stay stable across `foundations` major versions. Today that is the [`proto`]
//! data model: the [`prometheus/client_model`] protobuf types that are the
//! canonical wire format for the protobuf `/metrics` endpoint.
//!
//! The crate is kept small and dependency-light on purpose. The metrics registry
//! is a process-global singleton, so when two `foundations` majors are linked
//! into the same binary they must resolve to the *same* version of this crate to
//! share one registry rather than splitting metrics between them. A minimal,
//! slow-moving crate is what keeps that shared version easy to hold still — the
//! one expected source-breaking change is a change to the protobuf data model.
//!
//! Everything that can evolve more freely — metric types, encoders, label
//! serialisation, and service-name handling — lives in the sibling
//! `foundations-metrics` crate, not here.
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
