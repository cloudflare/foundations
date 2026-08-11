//! The `/metrics` endpoint must serve the format a scraper asks for through its
//! `Accept` header, and must describe what it served.
//!
//! Protobuf is the only format able to carry native histograms, so this covers
//! the path that makes them reachable at all. That the protobuf bytes are
//! well formed is covered by unit tests in `foundations-metrics`; what matters
//! here is that the route pairs the right encoder with the right content type.
#![cfg(feature = "foundations-metrics-backend")]

use std::future::IntoFuture;
use std::net::{Ipv4Addr, SocketAddr};

#[cfg(target_os = "linux")]
use foundations::telemetry::settings::{MetricsSettings, ServiceNameFormat};
use foundations::telemetry::settings::{TelemetryServerSettings, TelemetrySettings};
use foundations::telemetry::{TelemetryConfig, TelemetryContext};

const PROTOBUF_ACCEPT: &str = "application/vnd.google.protobuf;\
                               proto=io.prometheus.client.MetricFamily;\
                               encoding=delimited;q=0.5,\
                               application/openmetrics-text;version=1.0.0;q=0.4,\
                               */*;q=0.1";

const TEXT_ACCEPT: &str = "application/openmetrics-text;version=1.0.0;q=0.5,\
                           text/plain;version=0.0.4;q=0.4,*/*;q=0.1";

#[cfg(target_os = "linux")]
const PROCESS_CPU: &str = "process_cpu_seconds_total";

#[tokio::test]
async fn metrics_endpoint_serves_the_negotiated_format() {
    let _ctx = TelemetryContext::test();
    let server_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 1347));

    let settings = TelemetrySettings {
        server: TelemetryServerSettings {
            enabled: true,
            addr: server_addr.into(),
        },
        ..Default::default()
    };

    tokio::spawn(
        foundations::telemetry::init(TelemetryConfig {
            service_info: &foundations::service_info!(),
            settings: &settings,
            custom_server_routes: vec![],
        })
        .unwrap()
        .into_future(),
    );

    let client = reqwest::Client::new();
    let url = format!("http://{server_addr}/metrics");

    let scrape = |accept: &'static str| {
        let client = client.clone();
        let url = url.clone();
        async move {
            let response = client
                .get(url)
                .header(reqwest::header::ACCEPT, accept)
                .send()
                .await
                .unwrap();

            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .expect("/metrics should declare a content type")
                .to_str()
                .unwrap()
                .to_owned();

            (content_type, response.bytes().await.unwrap())
        }
    };

    let (content_type, body) = scrape(PROTOBUF_ACCEPT).await;

    assert_eq!(content_type, foundations_metrics::PROTOBUF_CONTENT_TYPE);
    assert!(!body.is_empty(), "protobuf response should not be empty");
    assert!(
        !body.starts_with(b"# "),
        "protobuf response looks like text: {:?}",
        String::from_utf8_lossy(&body[..body.len().min(64)])
    );
    assert!(
        !body.ends_with(b"# EOF\n"),
        "protobuf response carries the text terminator"
    );
    #[cfg(target_os = "linux")]
    assert!(
        body.windows(PROCESS_CPU.len())
            .any(|window| window == PROCESS_CPU.as_bytes()),
        "protobuf response is missing {PROCESS_CPU}"
    );

    let (content_type, body) = scrape(TEXT_ACCEPT).await;

    assert!(
        content_type.starts_with("application/openmetrics-text"),
        "unexpected content type: {content_type}"
    );

    let text = String::from_utf8(body.to_vec()).expect("text response should be UTF-8");

    assert!(text.contains("# TYPE"), "text response was: {text}");
    assert!(text.ends_with("# EOF\n"), "text response was: {text}");

    #[cfg(target_os = "linux")]
    {
        assert!(
            text.lines()
                .any(|line| line.starts_with(&format!("{PROCESS_CPU} "))),
            "text response is missing unprefixed {PROCESS_CPU}: {text}"
        );

        let label_settings = MetricsSettings {
            service_name_format: ServiceNameFormat::LabelWithName("service".to_owned()),
            report_optional: false,
        };
        let labelled = foundations::telemetry::metrics::collect(&label_settings).unwrap();

        assert!(
            labelled
                .lines()
                .any(|line| line.starts_with(&format!("{PROCESS_CPU} "))),
            "service labels should not be added to {PROCESS_CPU}: {labelled}"
        );
    }
}
