use foundations::telemetry::settings::{TelemetryServerSettings, TelemetrySettings};
use foundations::telemetry::{TelemetryConfig, TelemetryServerRoute};
use futures_util::FutureExt;
use hyper::{Method, Response};
use std::net::{Ipv4Addr, SocketAddr};

#[cfg(target_os = "linux")]
use foundations::telemetry::settings::MemoryProfilerSettings;

#[cfg(target_os = "linux")]
use foundations::telemetry::MemoryProfiler;

#[tokio::test]
async fn telemetry_server() {
    let server_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 1337));

    let settings = TelemetrySettings {
        server: TelemetryServerSettings {
            enabled: true,
            addr: server_addr.into(),
        },
        #[cfg(target_os = "linux")]
        memory_profiler: MemoryProfilerSettings {
            enabled: true,
            ..Default::default()
        },
        ..Default::default()
    };

    #[cfg(target_os = "linux")]
    assert!(
        MemoryProfiler::get_or_init_with(&settings.memory_profiler)
            .unwrap()
            .is_some(),
        "memory profiling should be enabled for tests via `_RJEM_MALLOC_CONF=prof:true` env var"
    );

    tokio::spawn(
        foundations::telemetry::init(TelemetryConfig {
            service_info: &foundations::service_info!(),
            settings: &settings,
            custom_server_routes: vec![TelemetryServerRoute {
                path: "/custom-route".into(),
                methods: vec![Method::GET],
                handler: Box::new(|_, _| {
                    async { Ok(Response::builder().body("Hello".into()).unwrap()) }.boxed()
                }),
            }],
        })
        .unwrap(),
    );

    assert_eq!(
        reqwest::get(format!("http://{server_addr}/health"))
            .await
            .unwrap()
            .status(),
        200
    );

    assert_eq!(
        reqwest::get(format!("http://{server_addr}/custom-route"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "Hello"
    );

    let metrics_res = reqwest::get(format!("http://{server_addr}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(metrics_res.contains("# HELP"));
    assert!(metrics_res.ends_with("# EOF\n"));

    #[cfg(target_os = "linux")]
    assert!(reqwest::get(format!("http://{server_addr}/pprof/heap"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
        .contains("MAPPED_LIBRARIES"));

    #[cfg(target_os = "linux")]
    assert!(
        reqwest::get(format!("http://{server_addr}/pprof/heap_stats"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
            .contains("Allocated")
    );
}
