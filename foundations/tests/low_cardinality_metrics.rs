//! End-to-end tests for low-cardinality metric families through the Foundations facade.
#![cfg(feature = "foundations-metrics-backend")]

use foundations::telemetry::metrics::{Counter, LowCardinalityLabel, metrics};
use foundations::telemetry::settings::{MetricsSettings, ServiceNameFormat};
use serde::Serialize;

mod common;
use common::collect_text;

mod foundations_reexport {
    pub(crate) use foundations::*;
}

#[derive(Clone, Copy, PartialEq, Serialize, LowCardinalityLabel)]
#[serde(rename_all = "snake_case")]
enum Protocol {
    Tcp = 2,
    Udp = 17,
    Quic = 200,
}

#[derive(Clone, Copy, PartialEq, Serialize, LowCardinalityLabel)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Reused,
    Discarded,
}

#[derive(
    Clone, Copy, PartialEq, Serialize, foundations_reexport::telemetry::metrics::LowCardinalityLabel,
)]
#[low_cardinality_label(crate_path = "crate::foundations_reexport")]
enum ReexportedLabel {
    First,
    Second,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq, Serialize, LowCardinalityLabel)]
#[serde(rename_all = "snake_case")]
enum MixedLabel {
    Selected,
    Unused,
}

static STATIC_SELECTED: MixedLabel = MixedLabel::Selected;

#[metrics]
mod service {
    /// Number of connection reuse decisions.
    #[low_cardinality]
    pub fn connection_reuses(protocol: Protocol, outcome: Outcome) -> Counter;
}

#[metrics]
mod mixed_service {
    /// An ordinary labeled metric in a module containing both family kinds.
    pub fn ordinary_requests(label: MixedLabel) -> Counter;

    /// A low-cardinality metric in a module containing both family kinds.
    #[low_cardinality]
    pub fn bounded_requests(label: MixedLabel) -> Counter;

    /// A low-cardinality metric accepting an explicitly bounded shared reference.
    #[low_cardinality]
    pub fn static_bounded_requests(label: &'static MixedLabel) -> Counter;
}

#[test]
fn low_cardinality_metrics_are_lazy_stable_and_cloneable() {
    // Mention the intermediate sparse-discriminant variant without asking the
    // metric family to initialize its slots.
    assert_eq!(Protocol::Udp as isize, 17);

    let tcp_reused = service::connection_reuses(Protocol::Tcp, Outcome::Reused);
    tcp_reused.inc_by(2);

    let same_tcp_reused = service::connection_reuses(Protocol::Tcp, Outcome::Reused);
    assert!(std::ptr::eq(tcp_reused, same_tcp_reused));
    same_tcp_reused.inc();

    let owned: Counter = tcp_reused.clone();
    owned.inc_by(2);
    assert_eq!(tcp_reused.get(), 5);

    service::connection_reuses(Protocol::Quic, Outcome::Discarded).inc_by(4);

    let settings = MetricsSettings {
        service_name_format: ServiceNameFormat::MetricPrefix,
        report_optional: false,
    };
    let text = collect_text(&settings);

    assert!(
        text.contains("undefined_service_connection_reuses{protocol=\"tcp\",outcome=\"reused\"} 5"),
        "collected output was: {text}"
    );
    assert!(
        text.contains(
            "undefined_service_connection_reuses{protocol=\"quic\",outcome=\"discarded\"} 4"
        ),
        "collected output was: {text}"
    );

    assert!(
        !text.contains("protocol=\"udp\""),
        "collected output was: {text}"
    );
    assert!(
        !text.contains("{protocol=\"tcp\",outcome=\"discarded\"}"),
        "collected output was: {text}"
    );
    assert!(
        !text.contains("{protocol=\"quic\",outcome=\"reused\"}"),
        "collected output was: {text}"
    );
}

#[test]
fn derive_supports_a_reexported_foundations_crate_path() {
    use foundations_reexport::telemetry::metrics::LowCardinalityLabel as _;

    assert_eq!(ReexportedLabel::CARDINALITY.get(), 2);
    assert_eq!(ReexportedLabel::First.index(), 0);
    assert_eq!(ReexportedLabel::Second.index(), 1);
}

#[test]
fn ordinary_and_low_cardinality_families_mix_and_export() {
    assert_eq!(MixedLabel::Unused.index(), 1);

    mixed_service::ordinary_requests(MixedLabel::Selected).inc_by(7);
    mixed_service::bounded_requests(MixedLabel::Selected).inc_by(11);
    mixed_service::static_bounded_requests(&STATIC_SELECTED).inc_by(13);

    let settings = MetricsSettings {
        service_name_format: ServiceNameFormat::MetricPrefix,
        report_optional: false,
    };
    let text = collect_text(&settings);

    assert!(
        text.contains("undefined_mixed_service_ordinary_requests{label=\"selected\"} 7"),
        "collected output was: {text}"
    );
    assert!(
        text.contains("undefined_mixed_service_bounded_requests{label=\"selected\"} 11"),
        "collected output was: {text}"
    );
    assert!(
        !text.contains("undefined_mixed_service_bounded_requests{label=\"unused\"}"),
        "collected output was: {text}"
    );
    assert!(
        text.contains("undefined_mixed_service_static_bounded_requests{label=\"selected\"} 13"),
        "collected output was: {text}"
    );
}
