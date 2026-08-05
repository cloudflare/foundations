//! An unencodable service label name must stop startup rather than surface as an
//! empty exposition later.
//!
//! Separate test binary on purpose: a successful `telemetry::init` elsewhere in
//! the binary would make these observe its once-per-process refusal instead.
#![cfg(all(feature = "foundations-metrics-backend", feature = "settings"))]

use foundations::telemetry::TelemetryConfig;
use foundations::telemetry::settings::{ServiceNameFormat, TelemetrySettings};

fn init_with(format: ServiceNameFormat) -> foundations::BootstrapResult<()> {
    let mut settings = TelemetrySettings::default();
    settings.metrics.service_name_format = format;

    foundations::telemetry::init(TelemetryConfig {
        service_info: &foundations::service_info!(),
        settings: &settings,
        custom_server_routes: vec![],
    })
    .map(|_| ())
}

#[test]
fn empty_service_label_name_is_rejected_at_startup() {
    let error = init_with(ServiceNameFormat::LabelWithName(String::new()))
        .expect_err("an unencodable service label name should stop startup");

    let message = error.to_string();

    assert!(
        message.contains("service_name_format"),
        "the error should name the setting at fault: {message}"
    );
    assert!(
        message.contains("non-empty"),
        "the error should state what was expected: {message}"
    );
}

#[test]
fn service_label_name_with_nul_is_rejected_at_startup() {
    let error = init_with(ServiceNameFormat::LabelWithName("ser\0vice".to_owned()))
        .expect_err("a label name with a NUL byte should stop startup");

    assert!(
        error.to_string().contains("service_name_format"),
        "the error should name the setting at fault: {error}"
    );
}
