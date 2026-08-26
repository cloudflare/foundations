//! The catch-all bucket of a classic histogram must be exposed as `+Inf` by
//! every encoder.
//!
//! Prometheus treats an infinite bound as the terminator of a classic bucket
//! list and appends a synthetic `+Inf` bucket carrying the sample count when it
//! does not find one. A finite terminal bound is therefore scraped as an extra
//! `le` series duplicating `+Inf`, which inflates series counts and lets
//! `histogram_quantile` interpolate between the last real bound and the
//! sentinel.

use foundations_metrics::{
    EncodeMetric, Histogram, NamedMetric, NativeHistogram, TimeHistogram, encode_to_protobuf,
    encode_to_text,
};
use foundations_metrics_registry::proto::MetricFamily;
use prost::Message;

const BOUNDS: [f64; 2] = [0.1, 0.5];

fn decode(mut bytes: &[u8]) -> Vec<MetricFamily> {
    let mut families = Vec::new();
    while !bytes.is_empty() {
        families.push(
            MetricFamily::decode_length_delimited(&mut bytes).expect("encoded family decodes"),
        );
    }
    families
}

/// Upper bounds of every classic bucket in the protobuf exposition.
fn protobuf_upper_bounds(families: &[MetricFamily]) -> Vec<f64> {
    decode(&encode_to_protobuf(families))
        .iter()
        .flat_map(|family| &family.metric)
        .filter_map(|metric| metric.histogram.as_ref())
        .flat_map(|histogram| &histogram.bucket)
        .map(|bucket| bucket.upper_bound.expect("bucket has an upper bound"))
        .collect()
}

/// `le` label values of every `_bucket` line in the text exposition.
fn text_upper_bounds(families: &[MetricFamily]) -> Vec<String> {
    encode_to_text(families)
        .lines()
        .filter(|line| line.contains("_bucket{"))
        .map(|line| {
            let le = line.split("le=\"").nth(1).expect("bucket line carries le");
            le.split('"').next().expect("le is quoted").to_owned()
        })
        .collect()
}

#[track_caller]
fn assert_terminates_with_infinity(families: &[MetricFamily]) {
    let protobuf = protobuf_upper_bounds(families);
    let text = text_upper_bounds(families);

    // One bucket per bound, plus the catch-all: no duplicate terminal series.
    assert_eq!(
        protobuf.len(),
        BOUNDS.len() + 1,
        "unexpected protobuf bucket count: {protobuf:?}"
    );
    assert_eq!(
        text.len(),
        BOUNDS.len() + 1,
        "unexpected text bucket count: {text:?}"
    );

    assert!(
        protobuf.last().is_some_and(|bound| bound.is_infinite()),
        "protobuf must terminate with +Inf, got {protobuf:?}"
    );
    assert_eq!(
        text.last().map(String::as_str),
        Some("+Inf"),
        "text must terminate with +Inf, got {text:?}"
    );

    // The `prometheus_client` sentinel must never reach the wire.
    assert!(
        !protobuf.contains(&f64::MAX),
        "f64::MAX leaked into the protobuf exposition: {protobuf:?}"
    );

    // Both encoders must describe the same bucket boundaries.
    let protobuf_as_text: Vec<_> = protobuf
        .iter()
        .map(|bound| {
            if bound.is_infinite() {
                "+Inf".to_owned()
            } else {
                bound.to_string()
            }
        })
        .collect();
    assert_eq!(
        protobuf_as_text, text,
        "text and protobuf disagree on bucket bounds"
    );
}

#[test]
fn classic_histogram_terminates_with_infinity() {
    let histogram = Histogram::new(BOUNDS);
    histogram.observe(0.05);
    histogram.observe(0.2);
    histogram.observe(999.0); // Overflows every configured bound.

    assert_terminates_with_infinity(&NamedMetric::new("demo_seconds", "Demo.", histogram).encode());
}

#[test]
fn time_histogram_terminates_with_infinity() {
    let histogram = TimeHistogram::new(BOUNDS);
    histogram.observe(50_000_000);
    histogram.observe(200_000_000);
    histogram.observe(999_000_000_000);

    assert_terminates_with_infinity(
        &NamedMetric::new("demo_time_seconds", "Demo.", histogram).encode(),
    );
}

#[test]
fn native_histogram_classic_buckets_terminate_with_infinity() {
    let histogram = NativeHistogram::new_classic_and_native(BOUNDS, 1.1);
    histogram.observe(0.05);
    histogram.observe(0.2);
    histogram.observe(999.0);

    assert_terminates_with_infinity(
        &NamedMetric::new("demo_native_seconds", "Demo.", histogram).encode(),
    );
}
