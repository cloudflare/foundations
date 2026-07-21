use std::fmt::Write as _;

use foundations_metrics_registry::proto::{
    Exemplar, Histogram, LabelPair, Metric, MetricFamily, MetricType,
};

use crate::diagnostics::report_collect_error;
use crate::validation::{ValidationContext, sanitized_metric_family};

/// Encodes metric families as OpenMetrics text.
pub fn encode_to_text(families: &[MetricFamily]) -> String {
    let mut output = String::new();

    for family in families {
        if let Some(family) = sanitized_metric_family(family, ValidationContext::TextEncoding) {
            encode_family(&mut output, &family);
        }
    }

    output.push_str("# EOF\n");
    output
}

fn encode_family(output: &mut String, family: &MetricFamily) {
    let name = family
        .name
        .as_deref()
        .expect("metric family names are validated before text encoding");
    let Some(metric_type) = family
        .r#type
        .and_then(|value| MetricType::try_from(value).ok())
    else {
        report_collect_error(format_args!(
            "non-fatal error while encoding OpenMetrics text: skipped metric family {name:?} with an unknown type"
        ));
        return;
    };
    let metric_type_name = match metric_type {
        MetricType::Counter => "counter",
        MetricType::Gauge => "gauge",
        MetricType::Summary => "summary",
        MetricType::Untyped => "unknown",
        MetricType::Histogram => "histogram",
        MetricType::GaugeHistogram => "gaugehistogram",
    };

    if let Some(help) = &family.help {
        output.push_str("# HELP ");
        output.push_str(name);
        output.push(' ');
        write_escaped(output, help);
        output.push('\n');
    }

    output.push_str("# TYPE ");
    output.push_str(name);
    output.push(' ');
    output.push_str(metric_type_name);
    output.push('\n');

    if let Some(unit) = &family.unit {
        output.push_str("# UNIT ");
        output.push_str(name);
        output.push(' ');
        write_escaped(output, unit);
        output.push('\n');
    }

    for metric in &family.metric {
        encode_metric(output, name, metric_type, metric);
    }
}

fn encode_metric(output: &mut String, name: &str, metric_type: MetricType, metric: &Metric) {
    match metric_type {
        MetricType::Counter => {
            let Some(counter) = &metric.counter else {
                report_missing_value(name, "counter");
                return;
            };
            write_sample(
                output,
                name,
                "",
                metric,
                None,
                SampleValue::Float(counter.value.unwrap_or_default()),
                counter.exemplar.as_ref(),
            );
        }
        MetricType::Gauge => {
            let Some(gauge) = &metric.gauge else {
                report_missing_value(name, "gauge");
                return;
            };
            write_plain_sample(
                output,
                name,
                "",
                metric,
                SampleValue::Float(gauge.value.unwrap_or_default()),
            );
        }
        MetricType::Summary => {
            let Some(summary) = &metric.summary else {
                report_missing_value(name, "summary");
                return;
            };
            for quantile in &summary.quantile {
                write_sample(
                    output,
                    name,
                    "",
                    metric,
                    Some(("quantile", quantile.quantile.unwrap_or_default())),
                    SampleValue::Float(quantile.value.unwrap_or_default()),
                    None,
                );
            }
            write_plain_sample(
                output,
                name,
                "_sum",
                metric,
                SampleValue::Float(summary.sample_sum.unwrap_or_default()),
            );
            write_plain_sample(
                output,
                name,
                "_count",
                metric,
                SampleValue::Unsigned(summary.sample_count.unwrap_or_default()),
            );
        }
        MetricType::Untyped => {
            let Some(untyped) = &metric.untyped else {
                report_missing_value(name, "untyped value");
                return;
            };
            write_plain_sample(
                output,
                name,
                "",
                metric,
                SampleValue::Float(untyped.value.unwrap_or_default()),
            );
        }
        MetricType::Histogram | MetricType::GaugeHistogram => {
            let Some(histogram) = &metric.histogram else {
                report_missing_value(name, "histogram");
                return;
            };
            encode_histogram(output, name, metric_type, metric, histogram);
        }
    }
}

fn encode_histogram(
    output: &mut String,
    name: &str,
    metric_type: MetricType,
    metric: &Metric,
    histogram: &Histogram,
) {
    if histogram.bucket.is_empty() && has_native_buckets(histogram) {
        report_collect_error(format_args!(
            "non-fatal error while encoding OpenMetrics text: skipped native histogram row for {name:?}; native histograms require protobuf output"
        ));
        return;
    }

    if histogram
        .sample_count_float
        .is_some_and(|count| count > 0.0)
        || histogram.bucket.iter().any(|bucket| {
            bucket
                .cumulative_count_float
                .is_some_and(|count| count > 0.0)
        })
    {
        report_collect_error(format_args!(
            "non-fatal error while encoding OpenMetrics text: skipped histogram row for {name:?} with floating-point bucket counts"
        ));
        return;
    }

    let (sum_suffix, count_suffix) = if metric_type == MetricType::GaugeHistogram {
        ("_gsum", "_gcount")
    } else {
        ("_sum", "_count")
    };
    let sample_count = histogram.sample_count.unwrap_or_default();

    write_plain_sample(
        output,
        name,
        sum_suffix,
        metric,
        SampleValue::Float(histogram.sample_sum.unwrap_or_default()),
    );
    write_plain_sample(
        output,
        name,
        count_suffix,
        metric,
        SampleValue::Unsigned(sample_count),
    );

    let mut infinity_bucket_seen = false;
    for bucket in &histogram.bucket {
        let upper_bound = bucket.upper_bound.unwrap_or_default();
        let upper_bound = if upper_bound == f64::MAX {
            f64::INFINITY
        } else {
            upper_bound
        };
        infinity_bucket_seen |= upper_bound == f64::INFINITY;

        write_sample(
            output,
            name,
            "_bucket",
            metric,
            Some(("le", upper_bound)),
            SampleValue::Unsigned(bucket.cumulative_count.unwrap_or_default()),
            bucket.exemplar.as_ref(),
        );
    }

    if !infinity_bucket_seen {
        write_sample(
            output,
            name,
            "_bucket",
            metric,
            Some(("le", f64::INFINITY)),
            SampleValue::Unsigned(sample_count),
            None,
        );
    }
}

fn has_native_buckets(histogram: &Histogram) -> bool {
    histogram.schema.is_some()
        || histogram.zero_threshold.is_some()
        || histogram.zero_count.is_some()
        || histogram.zero_count_float.is_some()
        || !histogram.negative_span.is_empty()
        || !histogram.negative_delta.is_empty()
        || !histogram.negative_count.is_empty()
        || !histogram.positive_span.is_empty()
        || !histogram.positive_delta.is_empty()
        || !histogram.positive_count.is_empty()
}

#[derive(Clone, Copy)]
enum SampleValue {
    Float(f64),
    Unsigned(u64),
}

fn write_plain_sample(
    output: &mut String,
    name: &str,
    suffix: &str,
    metric: &Metric,
    value: SampleValue,
) {
    write_sample(output, name, suffix, metric, None, value, None);
}

fn write_sample(
    output: &mut String,
    name: &str,
    suffix: &str,
    metric: &Metric,
    additional_label: Option<(&str, f64)>,
    value: SampleValue,
    exemplar: Option<&Exemplar>,
) {
    output.push_str(name);
    output.push_str(suffix);
    write_labels(output, &metric.label, additional_label);
    output.push(' ');
    match value {
        SampleValue::Float(value) => write_float(output, value),
        SampleValue::Unsigned(value) => {
            write!(output, "{value}").expect("writing to a String cannot fail");
        }
    }

    if let Some(timestamp_ms) = metric.timestamp_ms {
        output.push(' ');
        write_float(output, timestamp_ms as f64 / 1_000.0);
    }

    if let Some(exemplar) = exemplar.filter(|exemplar| !exemplar.label.is_empty()) {
        output.push_str(" # ");
        write_labels(output, &exemplar.label, None);
        output.push(' ');
        write_float(output, exemplar.value.unwrap_or_default());

        if let Some(timestamp) = &exemplar.timestamp {
            output.push(' ');
            write_float(
                output,
                timestamp.seconds as f64 + f64::from(timestamp.nanos) * 1e-9,
            );
        }
    }

    output.push('\n');
}

fn write_labels(output: &mut String, labels: &[LabelPair], additional_label: Option<(&str, f64)>) {
    if labels.is_empty() && additional_label.is_none() {
        return;
    }

    output.push('{');
    let mut separator = "";
    for label in labels {
        output.push_str(separator);
        output.push_str(label.name.as_deref().unwrap_or_default());
        output.push_str("=\"");
        write_escaped(output, label.value.as_deref().unwrap_or_default());
        output.push('"');
        separator = ",";
    }

    if let Some((name, value)) = additional_label {
        output.push_str(separator);
        output.push_str(name);
        output.push_str("=\"");
        write_float(output, value);
        output.push('"');
    }

    output.push('}');
}

fn write_float(output: &mut String, value: f64) {
    if value.is_nan() {
        output.push_str("NaN");
    } else if value == f64::INFINITY {
        output.push_str("+Inf");
    } else if value == f64::NEG_INFINITY {
        output.push_str("-Inf");
    } else {
        let mut buffer = ryu::Buffer::new();
        let formatted = buffer.format(value);
        output.push_str(formatted);
        if !formatted.contains(['.', 'e', 'E']) {
            output.push_str(".0");
        }
    }
}

fn write_escaped(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '"' => output.push_str("\\\""),
            _ => output.push(character),
        }
    }
}

fn report_missing_value(name: &str, expected: &str) {
    report_collect_error(format_args!(
        "non-fatal error while encoding OpenMetrics text: skipped row in metric family {name:?}; expected {expected} data"
    ));
}

#[cfg(test)]
mod tests {
    use foundations_metrics_registry::proto::{
        Bucket, Counter, Exemplar, Gauge, Histogram, LabelPair, Metric, MetricFamily, MetricType,
        Quantile, Summary,
    };

    use super::*;

    fn label(name: &str, value: &str) -> LabelPair {
        LabelPair {
            name: Some(name.to_owned()),
            value: Some(value.to_owned()),
        }
    }

    #[test]
    fn encodes_counter_metadata_labels_timestamps_and_exemplars() {
        let families = [MetricFamily {
            name: Some("requests".to_owned()),
            help: Some("A \"quoted\" help\\line\nnext".to_owned()),
            r#type: Some(MetricType::Counter as i32),
            metric: vec![Metric {
                label: vec![LabelPair {
                    name: Some("kind".to_owned()),
                    value: Some("a\"b\\c\nd".to_owned()),
                }],
                counter: Some(Counter {
                    value: Some(1.0),
                    exemplar: Some(Exemplar {
                        label: vec![LabelPair {
                            name: Some("trace_id".to_owned()),
                            value: Some("abc".to_owned()),
                        }],
                        value: Some(2.0),
                        timestamp: None,
                    }),
                    created_timestamp: None,
                }),
                timestamp_ms: Some(1_500),
                ..Default::default()
            }],
            unit: None,
        }];

        assert_eq!(
            encode_to_text(&families),
            "# HELP requests A \\\"quoted\\\" help\\\\line\\nnext\n\
# TYPE requests counter\n\
requests{kind=\"a\\\"b\\\\c\\nd\"} 1.0 1.5 # {trace_id=\"abc\"} 2.0\n\
# EOF\n"
        );
    }

    #[test]
    fn encodes_classic_histogram_and_maps_terminal_bucket_to_infinity() {
        let families = [MetricFamily {
            name: Some("request_duration_seconds".to_owned()),
            help: Some("Request duration.".to_owned()),
            r#type: Some(MetricType::Histogram as i32),
            metric: vec![Metric {
                label: vec![LabelPair {
                    name: Some("route".to_owned()),
                    value: Some("/test".to_owned()),
                }],
                histogram: Some(Histogram {
                    sample_count: Some(3),
                    sample_sum: Some(4.5),
                    bucket: vec![
                        Bucket {
                            cumulative_count: Some(1),
                            upper_bound: Some(1.0),
                            ..Default::default()
                        },
                        Bucket {
                            cumulative_count: Some(3),
                            upper_bound: Some(f64::MAX),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            unit: Some("seconds".to_owned()),
        }];

        assert_eq!(
            encode_to_text(&families),
            "# HELP request_duration_seconds Request duration.\n\
# TYPE request_duration_seconds histogram\n\
# UNIT request_duration_seconds seconds\n\
request_duration_seconds_sum{route=\"/test\"} 4.5\n\
request_duration_seconds_count{route=\"/test\"} 3\n\
request_duration_seconds_bucket{route=\"/test\",le=\"1.0\"} 1\n\
request_duration_seconds_bucket{route=\"/test\",le=\"+Inf\"} 3\n\
# EOF\n"
        );
    }

    #[test]
    fn appends_an_infinite_histogram_bucket_when_missing() {
        let families = [MetricFamily {
            name: Some("values".to_owned()),
            help: None,
            r#type: Some(MetricType::Histogram as i32),
            metric: vec![Metric {
                histogram: Some(Histogram {
                    sample_count: Some(2),
                    sample_sum: Some(3.0),
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
        }];

        let output = encode_to_text(&families);
        assert!(output.contains("values_bucket{le=\"+Inf\"} 2\n"));
    }

    #[test]
    fn encodes_summaries_and_gauge_histograms() {
        let families = [
            MetricFamily {
                name: Some("request_size".to_owned()),
                help: None,
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
                name: Some("queue_depth".to_owned()),
                help: None,
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

        assert_eq!(
            encode_to_text(&families),
            "# TYPE request_size summary\n\
request_size{quantile=\"0.5\"} 3.0\n\
request_size_sum 6.0\n\
request_size_count 2\n\
# TYPE queue_depth gaugehistogram\n\
queue_depth_gsum 8.0\n\
queue_depth_gcount 3\n\
queue_depth_bucket{le=\"1.0\"} 1\n\
queue_depth_bucket{le=\"+Inf\"} 3\n\
# EOF\n"
        );
    }

    #[test]
    fn encodes_gauges_and_special_float_values() {
        let families = [MetricFamily {
            name: Some("temperature".to_owned()),
            help: None,
            r#type: Some(MetricType::Gauge as i32),
            metric: [f64::NAN, f64::INFINITY, f64::NEG_INFINITY]
                .into_iter()
                .map(|value| Metric {
                    gauge: Some(Gauge { value: Some(value) }),
                    ..Default::default()
                })
                .collect(),
            unit: None,
        }];

        assert_eq!(
            encode_to_text(&families),
            "# TYPE temperature gauge\n\
temperature NaN\n\
temperature +Inf\n\
temperature -Inf\n\
# EOF\n"
        );
    }

    #[test]
    fn encodes_legacy_info_metric_as_gauge() {
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

        assert_eq!(
            encode_to_text(&families),
            "# HELP build_info Build information.\n\
# TYPE build_info gauge\n\
build_info{version=\"1.2.3\"} 1.0\n\
# EOF\n"
        );
    }

    #[test]
    fn invalid_family_names_cannot_inject_metadata_and_valid_siblings_remain() {
        let families = [
            MetricFamily {
                name: Some("bad\n# HELP injected metadata".to_owned()),
                help: Some("should not be written".to_owned()),
                r#type: Some(MetricType::Gauge as i32),
                metric: vec![Metric {
                    gauge: Some(Gauge { value: Some(99.0) }),
                    ..Default::default()
                }],
                unit: None,
            },
            MetricFamily {
                name: Some("valid:metric".to_owned()),
                help: None,
                r#type: Some(MetricType::Gauge as i32),
                metric: vec![Metric {
                    gauge: Some(Gauge { value: Some(1.0) }),
                    ..Default::default()
                }],
                unit: None,
            },
        ];

        assert_eq!(
            encode_to_text(&families),
            "# TYPE valid:metric gauge\nvalid:metric 1.0\n# EOF\n"
        );
    }

    #[test]
    fn invalid_duplicate_and_reserved_row_labels_skip_only_their_rows() {
        let families = [
            MetricFamily {
                name: Some("row_gauge".to_owned()),
                help: None,
                r#type: Some(MetricType::Gauge as i32),
                metric: vec![
                    Metric {
                        label: vec![label("id", "valid")],
                        gauge: Some(Gauge { value: Some(1.0) }),
                        ..Default::default()
                    },
                    Metric {
                        label: vec![label("bad name", "invalid")],
                        gauge: Some(Gauge { value: Some(99.0) }),
                        ..Default::default()
                    },
                    Metric {
                        label: vec![label("dup", "a"), label("dup", "b")],
                        gauge: Some(Gauge { value: Some(98.0) }),
                        ..Default::default()
                    },
                ],
                unit: None,
            },
            MetricFamily {
                name: Some("row_histogram".to_owned()),
                help: None,
                r#type: Some(MetricType::Histogram as i32),
                metric: vec![
                    Metric {
                        histogram: Some(Histogram {
                            sample_count: Some(1),
                            sample_sum: Some(2.0),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Metric {
                        label: vec![label("le", "1")],
                        histogram: Some(Histogram {
                            sample_count: Some(99),
                            sample_sum: Some(99.0),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ],
                unit: None,
            },
            MetricFamily {
                name: Some("row_summary".to_owned()),
                help: None,
                r#type: Some(MetricType::Summary as i32),
                metric: vec![
                    Metric {
                        summary: Some(Default::default()),
                        ..Default::default()
                    },
                    Metric {
                        label: vec![label("quantile", "0.5")],
                        summary: Some(Default::default()),
                        ..Default::default()
                    },
                ],
                unit: None,
            },
        ];

        let output = encode_to_text(&families);
        assert!(output.contains("row_gauge{id=\"valid\"} 1.0\n"));
        assert!(!output.contains("99.0"));
        assert!(!output.contains("98.0"));
        assert_eq!(output.matches("row_histogram_sum").count(), 1);
        assert_eq!(output.matches("row_summary_sum").count(), 1);
        assert!(output.ends_with("# EOF\n"));
    }

    #[test]
    fn invalid_exemplar_labels_drop_only_the_exemplar() {
        let families = [
            MetricFamily {
                name: Some("exemplar_counter".to_owned()),
                help: None,
                r#type: Some(MetricType::Counter as i32),
                metric: vec![
                    Metric {
                        label: vec![label("id", "invalid")],
                        counter: Some(Counter {
                            value: Some(1.0),
                            exemplar: Some(Exemplar {
                                label: vec![label("trace:id", "bad")],
                                value: Some(2.0),
                                timestamp: None,
                            }),
                            created_timestamp: None,
                        }),
                        ..Default::default()
                    },
                    Metric {
                        label: vec![label("id", "valid")],
                        counter: Some(Counter {
                            value: Some(3.0),
                            exemplar: Some(Exemplar {
                                label: vec![label("trace_id", "good")],
                                value: Some(4.0),
                                timestamp: None,
                            }),
                            created_timestamp: None,
                        }),
                        ..Default::default()
                    },
                ],
                unit: None,
            },
            MetricFamily {
                name: Some("exemplar_histogram".to_owned()),
                help: None,
                r#type: Some(MetricType::Histogram as i32),
                metric: vec![Metric {
                    histogram: Some(Histogram {
                        sample_count: Some(1),
                        sample_sum: Some(1.0),
                        bucket: vec![Bucket {
                            cumulative_count: Some(1),
                            upper_bound: Some(1.0),
                            exemplar: Some(Exemplar {
                                label: vec![label("dup", "a"), label("dup", "b")],
                                value: Some(5.0),
                                timestamp: None,
                            }),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                unit: None,
            },
        ];

        let output = encode_to_text(&families);
        assert!(output.contains("exemplar_counter{id=\"invalid\"} 1.0\n"));
        assert!(output.contains("exemplar_counter{id=\"valid\"} 3.0 # {trace_id=\"good\"} 4.0\n"));
        assert!(output.contains("exemplar_histogram_bucket{le=\"1.0\"} 1\n"));
        assert!(!output.contains("trace:id"));
        assert!(!output.contains("{dup="));
    }
}
