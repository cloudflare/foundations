//! Serialization of metric label sets into the protobuf data model.

mod error;
mod serializer;

pub use error::LabelError;

use foundations_metrics_registry::proto::LabelPair;
use serde::Serialize;

// Adapted from prometools' `serde` label serializer
// (https://github.com/nox/prometools, licensed MIT OR Apache-2.0).
/// Serializes a label set into protobuf label pairs.
///
/// Label values are stored as raw strings. OpenMetrics escaping is deliberately
/// deferred until text encoding.
pub fn to_label_pairs<S>(labels: &S) -> Result<Vec<LabelPair>, LabelError>
where
    S: Serialize + ?Sized,
{
    labels.serialize(serializer::LabelSetSerializer)
}
