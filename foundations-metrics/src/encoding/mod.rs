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

#[cfg(test)]
mod tests {
    use foundations_metrics_registry::proto::{
        Bucket, Gauge, Histogram, LabelPair, Metric, MetricType, Quantile, Summary,
    };

    use super::*;

    #[test]
    fn preserves_summary_and_gauge_histogram_families() {
        let families = [
            MetricFamily {
                name: Some("empty_summary".to_owned()),
                help: None,
                r#type: Some(MetricType::Summary as i32),
                metric: Vec::new(),
                unit: None,
            },
            MetricFamily {
                name: Some("request_size".to_owned()),
                help: Some("Request size.".to_owned()),
                r#type: Some(MetricType::Summary as i32),
                metric: vec![Metric {
                    summary: Some(Summary {
                        sample_count: Some(2),
                        sample_sum: Some(6.0),
                        quantile: vec![Quantile {
                            quantile: Some(0.5),
                            value: Some(3.0),
                        }],
                        created_timestamp: None,
                    }),
                    ..Default::default()
                }],
                unit: None,
            },
            MetricFamily {
                name: Some("empty_gauge_histogram".to_owned()),
                help: None,
                r#type: Some(MetricType::GaugeHistogram as i32),
                metric: Vec::new(),
                unit: None,
            },
            MetricFamily {
                name: Some("queue_item_age".to_owned()),
                help: Some("Current age distribution of queued items.".to_owned()),
                r#type: Some(MetricType::GaugeHistogram as i32),
                metric: vec![Metric {
                    histogram: Some(Histogram {
                        sample_count: Some(3),
                        sample_sum: Some(8.0),
                        bucket: vec![Bucket {
                            cumulative_count: Some(1),
                            upper_bound: Some(1.0),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                unit: None,
            },
        ];

        let expected: Vec<_> = families
            .iter()
            .flat_map(Message::encode_length_delimited_to_vec)
            .collect();

        assert_eq!(encode_to_protobuf(&families), expected);
    }

    #[test]
    fn preserves_legacy_info_gauge_representation() {
        let families = [MetricFamily {
            name: Some("build_info".to_owned()),
            help: Some("Build information.".to_owned()),
            r#type: Some(MetricType::Gauge as i32),
            metric: vec![Metric {
                label: vec![LabelPair {
                    name: Some("version".to_owned()),
                    value: Some("1.2.3".to_owned()),
                }],
                gauge: Some(Gauge { value: Some(1.0) }),
                ..Default::default()
            }],
            unit: None,
        }];

        let encoded = encode_to_protobuf(&families);
        let decoded = MetricFamily::decode_length_delimited(encoded.as_slice()).unwrap();

        assert_eq!(decoded, families[0]);
    }
}
