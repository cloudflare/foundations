//! The Prometheus protobuf data model (package `io.prometheus.client`).
//!
//! These types mirror [`prometheus/client_model`] and are the canonical wire
//! format for the protobuf `/metrics` endpoint. The generated Rust model is
//! checked in and verified against the vendored `proto/metrics.proto`.
//!
//! [`prometheus/client_model`]: https://github.com/prometheus/client_model

#[allow(missing_docs, unreachable_pub, clippy::all)]
mod model;

pub use model::{BucketSpan, Counter, Gauge, Histogram, Metric, MetricFamily, MetricType};

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn native_histogram_family_round_trips() {
        let family = MetricFamily {
            name: Some("requests_duration_seconds".to_owned()),
            help: Some("Request duration.".to_owned()),
            r#type: Some(MetricType::Histogram as i32),
            unit: Some("seconds".to_owned()),
            metric: vec![Metric {
                histogram: Some(Histogram {
                    sample_count: Some(3),
                    sample_sum: Some(1.5),
                    schema: Some(2),
                    zero_threshold: Some(1e-9),
                    zero_count: Some(1),
                    positive_span: vec![BucketSpan {
                        offset: Some(0),
                        length: Some(2),
                    }],
                    positive_delta: vec![1, 0],
                    ..Default::default()
                }),
                ..Default::default()
            }],
        };

        let decoded = MetricFamily::decode(family.encode_to_vec().as_slice())
            .expect("protobuf roundtrip should succeed");

        assert_eq!(decoded, family);
        assert_eq!(decoded.r#type(), MetricType::Histogram);

        let histogram = decoded.metric[0]
            .histogram
            .as_ref()
            .expect("histogram present");

        assert_eq!(histogram.schema, Some(2));
        assert_eq!(histogram.zero_count, Some(1));
        assert_eq!(histogram.positive_span.len(), 1);
        assert_eq!(histogram.positive_delta, vec![1, 0]);
    }
}
