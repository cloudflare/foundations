mod text;

use prost::Message;

use crate::MetricFamily;

pub use text::encode_to_text;

/// Encodes metric families as length-delimited Prometheus protobuf messages.
pub fn encode_to_protobuf(families: &[MetricFamily]) -> Vec<u8> {
    families
        .iter()
        .flat_map(Message::encode_length_delimited_to_vec)
        .collect()
}
