use foundations::telemetry::settings::{
    LivenessTrackingSettings, TelemetryServerSettings, TelemetrySettings, TracingSettings,
};
use foundations::telemetry::{
    TelemetryConfig, TelemetryContext, TelemetryRouteBody, TelemetryServerRoute,
    reexports::{
        http_body_util::{BodyExt, Full},
        hyper::{Method, Response},
    },
};
use futures_util::FutureExt;
use std::future::IntoFuture;
use std::net::{Ipv4Addr, SocketAddr};

#[cfg(target_os = "linux")]
use foundations::telemetry::settings::MemoryProfilerSettings;

#[cfg(target_os = "linux")]
use foundations::telemetry::MemoryProfiler;

/// Captures its own instruction pointer via [`backtrace::trace`] and returns it
/// along with the symbol name that the `backtrace` crate resolves it to. This
/// gives us a (pc, expected_name) pair that the `/pprof/symbol` endpoint must
/// also be able to resolve, since it uses the same crate.
#[inline(never)]
fn capture_self_pc() -> (usize, String) {
    let mut result = None;
    backtrace::trace(|frame| {
        if result.is_some() {
            return false;
        }
        let ip = frame.ip() as usize;
        backtrace::resolve(ip as *mut std::ffi::c_void, |symbol| {
            if result.is_none()
                && let Some(name) = symbol.name()
            {
                let name = name.to_string();
                if name.contains("capture_self_pc") {
                    result = Some((ip, name));
                }
            }
        });
        result.is_none()
    });
    result.expect("should find capture_self_pc frame")
}

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
        tracing: TracingSettings {
            liveness_tracking: LivenessTrackingSettings {
                enabled: true,
                track_all_spans: true,
            },
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
                    async {
                        Ok(Response::new(TelemetryRouteBody::new(
                            Full::from("Hello").map_err(Into::into),
                        )))
                    }
                    .boxed()
                }),
            }],
        })
        .unwrap()
        .into_future(),
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
    assert!(
        reqwest::get(format!("http://{server_addr}/pprof/heap"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
            .contains("MAPPED_LIBRARIES")
    );

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

    // Capture a real PC and the symbol the backtrace crate resolves it to, so
    // we can verify both GET and POST resolve to the same name.
    let (known_pc, expected_symbol) = capture_self_pc();
    let addr_hex = format!("{:#x}", known_pc);

    // Test symbol resolution via GET (addresses in query string), GET without
    // addresses (just checks availability), and POST (addresses in body).
    let symbol_requests: Vec<(&str, String)> = vec![
        (
            "GET",
            format!("http://{server_addr}/pprof/symbol?{addr_hex}"),
        ),
        (
            "GET (no addr)",
            format!("http://{server_addr}/pprof/symbol"),
        ),
        ("POST", format!("http://{server_addr}/pprof/symbol")),
    ];

    for (method, url) in &symbol_requests {
        let res = if *method == "POST" {
            reqwest::Client::new()
                .post(url)
                .body(addr_hex.clone())
                .send()
                .await
        } else {
            reqwest::get(url).await
        }
        .unwrap()
        .text()
        .await
        .unwrap();

        eprintln!("pprof/symbol {method}: url={url}, expecting='{expected_symbol}'");
        eprintln!("pprof/symbol {method}: response:\n{res}");

        assert!(
            res.contains("num_symbols: 1"),
            "pprof/symbol {method} should contain 'num_symbols: 1', got: {res}"
        );

        if !method.contains("no addr") {
            assert!(
                res.contains(&expected_symbol),
                "pprof/symbol {method} should resolve to '{expected_symbol}', got: {res}"
            );
            let known_pc_str = format!("{:#x}", known_pc);
            assert!(
                res.contains(&known_pc_str),
                "pprof/symbol {method} should resolve to '{known_pc_str}', got: {res}"
            );
        }
    }

    let telemetry_ctx = TelemetryContext::current();
    let _scope = telemetry_ctx.scope();

    // Create a broadcast channel used to keep tasks active until we fetch traces.
    let (keep_trace_active, mut trace_waiter) = tokio::sync::broadcast::channel(2);

    // Create a span with a detached child.
    // The parent span will end before the child does.
    let mut trace_waiter1 = keep_trace_active.subscribe();
    #[allow(clippy::async_yields_async)]
    let child_span_handle = foundations::telemetry::tracing::span("parent_span")
        .into_context()
        .apply(async move {
            // return the JoinHandle for this task
            tokio::spawn(
                foundations::telemetry::tracing::span("child_span_outliving_parent")
                    .into_context()
                    .apply(async move {
                        let _ = trace_waiter1.recv().await;
                    }),
            )
        })
        .await;

    // Create a span that stays active
    let traced_task = {
        let _scope = telemetry_ctx.scope();
        let _root = foundations::telemetry::tracing::span("my_root_span");

        tokio::spawn(TelemetryContext::current().apply(async move {
            let _ = trace_waiter.recv().await;
        }))
    };

    let trace_output = reqwest::get(format!("http://{server_addr}/debug/traces"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    keep_trace_active.send(()).unwrap();
    let _ = traced_task.await;
    let _ = child_span_handle.await;

    assert!(!trace_output.contains("parent_span"));
    assert!(trace_output.contains("child_span_outliving_parent"));
    assert!(trace_output.contains("my_root_span"));
}
