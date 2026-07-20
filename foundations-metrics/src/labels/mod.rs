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
///
/// Unit structs and fieldless enum variants serialize to their Rust type or
/// variant *name* (after any `serde` rename), so renaming the type silently
/// changes the emitted label value. Use `#[serde(rename = "...")]` to pin a
/// stable value that is independent of the Rust name.
pub fn to_label_pairs<S>(labels: &S) -> Result<Vec<LabelPair>, LabelError>
where
    S: Serialize + ?Sized,
{
    labels.serialize(serializer::LabelSetSerializer)
}
