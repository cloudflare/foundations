//! Protobuf must not be served while text-only extra producers are registered,
//! since it cannot carry what they emit and the loss would be invisible.
//!
//! Separate test binary on purpose: producer registration is process-global and
//! one-way, so registering one here would make protobuf permanently unreachable
//! for every other test in the binary.
#![cfg(feature = "foundations-metrics-backend")]

use std::future::IntoFuture;
use std::net::{Ipv4Addr, SocketAddr};

use foundations::telemetry::settings::{TelemetryServerSettings, TelemetrySettings};
use foundations::telemetry::{TelemetryConfig, TelemetryContext};

/// Delimited protobuf preferred, text still acceptable.
const PROTOBUF_PREFERRED: &str = "application/vnd.google.protobuf;\
                                  proto=io.prometheus.client.MetricFamily;\
                                  encoding=delimited;q=0.9,\
                                  application/openmetrics-text;version=1.0.0;q=0.1";

/// Delimited protobuf and nothing else: no text range, no `*/*`.
const PROTOBUF_ONLY: &str = "application/vnd.google.protobuf;\
                             proto=io.prometheus.client.MetricFamily;encoding=delimited";

const PRODUCER_SERIES: &str = "extra_producer_witness_total";

#[tokio::test]
async fn extra_producers_withhold_protobuf() {
    let _ctx = TelemetryContext::test();
    let server_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 1357));

    // Shaped like a service's cache metrics: a self-contained document with its
    // own terminator, which foundations strips before adding the final one.
    #[allow(deprecated)]
    foundations::telemetry::metrics::add_extra_producer(|buffer: &mut Vec<u8>| {
        buffer.extend_from_slice(
            b"# TYPE extra_producer_witness counter\n\
              # HELP extra_producer_witness Emitted only by a text-only extra producer.\n\
              extra_producer_witness_total 7\n\
              # EOF\n",
        );
    });

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

            let status = response.status();
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .expect("/metrics should declare a content type")
                .to_str()
                .unwrap()
                .to_owned();

            (status, content_type, response.bytes().await.unwrap())
        }
    };

    // Text is acceptable to this scraper, so it gets text with the producer
    // output intact rather than protobuf without it.
    let (status, content_type, body) = scrape(PROTOBUF_PREFERRED).await;

    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(
        content_type.starts_with("application/openmetrics-text"),
        "protobuf was served despite registered extra producers: {content_type}"
    );

    let text = String::from_utf8(body.to_vec()).expect("response should be text");

    assert!(
        text.contains(PRODUCER_SERIES),
        "extra producer output missing, body was: {text}"
    );
    assert!(
        text.contains("# TYPE"),
        "registered metrics missing, body was: {text}"
    );
    assert!(
        text.ends_with("# EOF\n"),
        "response was not terminated, body was: {text}"
    );
    assert_eq!(
        text.matches("# EOF").count(),
        1,
        "expected exactly one terminator, body was: {text}"
    );

    // Nothing this scraper accepts can be served: protobuf is withheld and it
    // offers no text range, so it receives the fallback text format.
    let (status, content_type, ..) = scrape(PROTOBUF_ONLY).await;

    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(
        content_type.starts_with("application/openmetrics-text"),
        "unsatisfiable Accept should fall back to text, got: {content_type}"
    );

    // Collecting directly bypasses negotiation, so the encoder refuses instead
    // of silently dropping the producer's series.
    #[allow(deprecated)]
    let allow_protobuf = foundations::telemetry::metrics::allow_protobuf();

    assert!(!allow_protobuf);
    assert!(
        foundations::telemetry::metrics::collect_format(
            foundations::telemetry::metrics::ScrapeFormat::Protobuf,
            &settings.metrics,
        )
        .is_err(),
        "protobuf collection should fail while an extra producer is registered"
    );
}
