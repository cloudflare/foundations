mod text;

use prost::Message;

use crate::MetricFamily;
use crate::validation::{ValidationContext, sanitized_metric_family};

pub use text::{OPENMETRICS_CONTENT_TYPE, encode_to_text};

/// Encodes metric families as length-delimited Prometheus protobuf messages.
pub fn encode_to_protobuf(families: &[MetricFamily]) -> Vec<u8> {
    let mut output = Vec::new();
    for family in families {
        if let Some(family) = sanitized_metric_family(family, ValidationContext::ProtobufEncoding) {
            family
                .encode_length_delimited(&mut output)
                .expect("encoding a protobuf message to a Vec cannot fail");
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use foundations_metrics_registry::proto::{
        Bucket, Counter, Exemplar, Gauge, Histogram, LabelPair, Metric, MetricType, Quantile,
        Summary,
    };

    use super::*;

    fn label(name: &str, value: &str) -> LabelPair {
        LabelPair {
            name: Some(name.to_owned()),
            value: Some(value.to_owned()),
        }
    }

    fn decode_families(mut bytes: &[u8]) -> Vec<MetricFamily> {
        let mut families = Vec::new();
        while !bytes.is_empty() {
            families.push(
                MetricFamily::decode_length_delimited(&mut bytes)
                    .expect("encoded family should decode"),
            );
        }
        families
    }

    #[test]
    fn utf8_protobuf_output_is_unchanged() {
        let families = [MetricFamily {
            name: Some("valid counter λ".to_owned()),
            help: Some("Valid counter.".to_owned()),
            r#type: Some(MetricType::Counter as i32),
            metric: vec![Metric {
                label: vec![label("_label.name λ", "value")],
                counter: Some(Counter {
                    value: Some(1.0),
                    exemplar: Some(Exemplar::default()),
                    created_timestamp: None,
                }),
                ..Default::default()
            }],
            unit: None,
        }];
        let expected: Vec<_> = families
            .iter()
            .flat_map(Message::encode_length_delimited_to_vec)
            .collect();

        let encoded = encode_to_protobuf(&families);
        assert_eq!(encoded, expected);
        assert!(
            decode_families(&encoded)[0].metric[0]
                .counter
                .as_ref()
                .unwrap()
                .exemplar
                .is_some(),
            "empty exemplars retain their existing protobuf behavior"
        );
    }

    #[test]
    fn protobuf_keeps_nonstandard_names_and_omits_empty_duplicate_and_reserved_names() {
        let families = [
            MetricFamily {
                name: Some(String::new()),
                help: None,
                r#type: Some(MetricType::Gauge as i32),
                metric: vec![Metric {
                    gauge: Some(Gauge { value: Some(100.0) }),
                    ..Default::default()
                }],
                unit: None,
            },
            MetricFamily {
                name: Some("bad\nfamily".to_owned()),
                help: None,
                r#type: Some(MetricType::Gauge as i32),
                metric: vec![Metric {
                    gauge: Some(Gauge { value: Some(99.0) }),
                    ..Default::default()
                }],
                unit: None,
            },
            MetricFamily {
                name: Some("protobuf_counter".to_owned()),
                help: None,
                r#type: Some(MetricType::Counter as i32),
                metric: vec![
                    Metric {
                        label: vec![label("id", "kept")],
                        counter: Some(Counter {
                            value: Some(1.0),
                            exemplar: Some(Exemplar {
                                label: vec![label("trace:id", "nonstandard")],
                                ..Default::default()
                            }),
                            created_timestamp: None,
                        }),
                        ..Default::default()
                    },
                    Metric {
                        label: vec![label("bad name", "kept_nonstandard")],
                        counter: Some(Counter {
                            value: Some(2.0),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Metric {
                        label: vec![label("dup", "a"), label("dup", "b")],
                        counter: Some(Counter {
                            value: Some(3.0),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ],
                unit: None,
            },
            MetricFamily {
                name: Some("protobuf_histogram".to_owned()),
                help: None,
                r#type: Some(MetricType::Histogram as i32),
                metric: vec![
                    Metric {
                        histogram: Some(Histogram {
                            bucket: vec![Bucket {
                                exemplar: Some(Exemplar {
                                    label: vec![label("dup", "a"), label("dup", "b")],
                                    ..Default::default()
                                }),
                                ..Default::default()
                            }],
                            exemplars: vec![
                                Exemplar {
                                    label: vec![label("bad#name", "nonstandard")],
                                    timestamp: Some(Default::default()),
                                    ..Default::default()
                                },
                                Exemplar {
                                    label: vec![label("trace_id", "missing_timestamp")],
                                    ..Default::default()
                                },
                                Exemplar {
                                    label: vec![label("trace_id", "good")],
                                    timestamp: Some(Default::default()),
                                    ..Default::default()
                                },
                            ],
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Metric {
                        label: vec![label("le", "1")],
                        histogram: Some(Histogram::default()),
                        ..Default::default()
                    },
                ],
                unit: None,
            },
            MetricFamily {
                name: Some("protobuf_sibling".to_owned()),
                help: None,
                r#type: Some(MetricType::Gauge as i32),
                metric: vec![Metric {
                    gauge: Some(Gauge { value: Some(4.0) }),
                    ..Default::default()
                }],
                unit: None,
            },
        ];

        let decoded = decode_families(&encode_to_protobuf(&families));
        assert_eq!(
            decoded
                .iter()
                .filter_map(|family| family.name.as_deref())
                .collect::<Vec<_>>(),
            [
                "bad\nfamily",
                "protobuf_counter",
                "protobuf_histogram",
                "protobuf_sibling",
            ]
        );

        assert_eq!(decoded[1].metric.len(), 2);
        assert_eq!(
            decoded[1].metric[0]
                .counter
                .as_ref()
                .unwrap()
                .exemplar
                .as_ref()
                .unwrap()
                .label[0]
                .name
                .as_deref(),
            Some("trace:id")
        );
        assert_eq!(
            decoded[1].metric[1].label[0].name.as_deref(),
            Some("bad name")
        );

        assert_eq!(decoded[2].metric.len(), 1);
        let histogram = decoded[2].metric[0].histogram.as_ref().unwrap();
        assert!(histogram.bucket[0].exemplar.is_none());
        assert_eq!(histogram.exemplars.len(), 2);
        assert_eq!(
            histogram
                .exemplars
                .iter()
                .map(|exemplar| exemplar.label[0].name.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["bad#name", "trace_id"]
        );

        assert_eq!(decoded[3].metric.len(), 1);
    }

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
