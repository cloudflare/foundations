//! A simple HTTP server that serves routes specified in the config.
//!
//! Run
//!
//! ```sh
//! cargo run --example http_server -- --help
//! ```
//!
//! from the repo root for the usage info.

mod metrics;
mod settings;

use self::settings::{EndpointSettings, HttpServerSettings, ResponseSettings};
use anyhow::anyhow;
use foundations::addr::ListenAddr;
use foundations::cli::{Arg, ArgAction, Cli};
use foundations::settings::collections::Map;
use foundations::telemetry::{self, log, tracing, TelemetryConfig, TelemetryContext};
use foundations::BootstrapResult;
use futures_util::stream::{FuturesUnordered, StreamExt};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::convert::Infallible;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};

#[tokio::main]
async fn main() -> BootstrapResult<()> {
    // Obtain service information from Cargo.toml
    let service_info = foundations::service_info!();

    // Parse command line arguments. Add additional command line option that allows checking
    // the config without running the server.
    let cli = Cli::<HttpServerSettings>::new(
        &service_info,
        vec![Arg::new("dry-run")
            .long("dry-run")
            .action(ArgAction::SetTrue)
            .help("Validate or generate config without running the server")],
    )?;

    // Exit if we just want to check the config.
    if cli.arg_matches.get_flag("dry-run") {
        return Ok(());
    }

    // Initialize telemetry with the settings obtained from the config. Don't drive the telemetry
    // yet - we have some extra security-related steps to do.
    let tele_driver = telemetry::init(TelemetryConfig {
        service_info: &service_info,
        settings: &cli.settings.telemetry,
        custom_server_routes: vec![],
    })?;

    if let Some(addr) = tele_driver.server_addr() {
        match addr {
            ListenAddr::Tcp(addr) => log::info!("Telemetry server is listening on http://{addr}"),
            ListenAddr::Unix(path) => log::info!("Telemetry server is listening on {path:?}"),
        }
    }

    // Spawn TCP listeners for each endpoint. Note that `Map<EndpointsSettings>` is ordered, so
    // we can just return a `Vec<TcpListener>` from `spawn_tcp_listeners` and correspond returned
    // listener to the settings just by index.
    let listeners = spawn_tcp_listeners(&cli.settings.endpoints)?;

    // Now, when we have listeners ready, we can add extra security layer to serve user traffic.
    // Allowing syscalls only from the allow list we significantly reduce the attack surface in
    // case of software vulnerability that allows remote code execution. For example, the allow list
    // doesn't include `listen` syscall that we used before to spawn `TcpListener`'s, so if malicious
    // actor attempt to create a new server endpoint the process will terminate.
    #[cfg(target_os = "linux")]
    sandbox_syscalls()?;

    // Start serving endpoints.
    let mut endpoint_futures = FuturesUnordered::new();

    for ((name, settings), listener) in cli.settings.endpoints.into_iter().zip(listeners) {
        // Each endpoint has it's own independent log.
        let endpoint_fut = TelemetryContext::current()
            .with_forked_log()
            .apply(async move { run_endpoint(name, settings.routes, listener).await });

        endpoint_futures.push(endpoint_fut)
    }

    // Drive all the server futures.
    tokio::select! {
        r = endpoint_futures.next() => {
            r.ok_or_else(|| anyhow!("server should have at least one endpoint specified"))??
        },
        r = tele_driver => { r? }
    }

    unreachable!("server should never terminate without an error");
}

fn spawn_tcp_listeners(
    endpoints_settings: &Map<String, EndpointSettings>,
) -> BootstrapResult<Vec<StdTcpListener>> {
    let mut listeners = vec![];

    for (name, settings) in endpoints_settings {
        let listener = StdTcpListener::bind(settings.addr)?;

        log::info!(
            "`{}` endpoint is listening on http://{}",
            name,
            listener.local_addr()?
        );

        listeners.push(listener);
    }

    Ok(listeners)
}

async fn run_endpoint(
    endpoint_name: String,
    routes: Map<String, ResponseSettings>,
    listener: StdTcpListener,
) -> BootstrapResult<()> {
    listener.set_nonblocking(true)?;

    let listener = TcpListener::from_std(listener)?;
    let endpoint_name = Arc::new(endpoint_name);
    let routes = Arc::new(routes);

    // These fields will be included into all the log records produced by this endpoint.
    log::add_fields! {
        "endpoint_name" => Arc::clone(&endpoint_name),
        "endpoint_addr" => listener.local_addr()?
    }

    loop {
        match listener.accept().await {
            Ok((conn, client_addr)) => {
                let routes = Arc::clone(&routes);
                let endpoint_name = Arc::clone(&endpoint_name);

                // Each connection gets its own independent log that inherits fields from the
                // endoint log.
                tokio::spawn(
                    TelemetryContext::current()
                        .with_forked_log()
                        .apply(async move {
                            serve_connection(endpoint_name, conn, client_addr, routes).await
                        }),
                );
            }
            Err(e) => {
                log::error!("failed to accept connection"; "error" => e);
                metrics::http_server::failed_connections_total(&endpoint_name).inc();
            }
        }
    }
}

#[tracing::span_fn("Client connection")]
async fn serve_connection(
    endpoint_name: Arc<String>,
    conn: TcpStream,
    client_addr: SocketAddr,
    routes: Arc<Map<String, ResponseSettings>>,
) {
    metrics::http_server::active_connections(&endpoint_name).inc();

    tracing::add_span_tags! { "client_addr" => client_addr.to_string() }
    log::add_fields! { "client_addr" => client_addr }
    log::info!("accepted client connection");

    // Obtain current telemetry context that's bound to connection and pass it into the closure.
    // In this particular case we don't really need to do that as we pass the closure to hyper's
    // `serve_connection` which we `await` in this function that already has connection-bound
    // telemetry context and hyper doesn't call additional `tokio::spawn`'s to call the closure.
    // But it's a good practice to explicitly pass the desired telemetry context into closures
    // to ensure that they always will have desired telemetry context, disregard of how they are
    // called.
    let conn_tele_ctx = TelemetryContext::current();

    let on_request = service_fn({
        let endpoint_name = Arc::clone(&endpoint_name);

        move |req| {
            let routes = Arc::clone(&routes);
            let endpoint_name = Arc::clone(&endpoint_name);

            // Each request gets independent log inherited from the connection log and separate
            // trace linked to the connection trace.
            conn_tele_ctx
                .with_forked_log()
                .with_forked_trace("request")
                .apply(async move { respond(endpoint_name, req, routes).await })
        }
    });

    if let Err(e) = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
        .serve_connection(TokioIo::new(conn), on_request)
        .await
    {
        log::error!("failed to serve HTTP"; "error" => ?e);
        metrics::http_server::failed_connections_total(&endpoint_name).inc();
    }

    metrics::http_server::active_connections(&endpoint_name).dec();
}

#[tracing::span_fn("respond to request")]
async fn respond(
    endpoint_name: Arc<String>,
    req: Request<Incoming>,
    routes: Arc<Map<String, ResponseSettings>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    log::add_fields! {
        "request_uri" => req.uri().to_string(),
        "method" => req.method().to_string()
    }

    log::info!("received request");
    tracing::add_span_tags! { "request_uri" => req.uri().to_string() }
    metrics::http_server::requests_total(&endpoint_name).inc();

    let route = {
        let _span = tracing::span("route lookup");

        routes.get(req.uri().path())
    };

    Ok(match route {
        Some(route) => {
            log::info!("sending response to the client"; "status_code" => route.status_code);

            Response::builder()
                .status(route.status_code)
                .body(route.response.to_string().into())
                .unwrap()
        }
        None => {
            log::error!("failed to find a route for the request");
            metrics::http_server::requests_failed_total(&endpoint_name, 404).inc();
            Response::builder().status(404).body("".into()).unwrap()
        }
    })
}

#[cfg(target_os = "linux")]
fn sandbox_syscalls() -> BootstrapResult<()> {
    use foundations::security::common_syscall_allow_lists::{
        ASYNC, NET_SOCKET_API, SERVICE_BASICS,
    };
    use foundations::security::{allow_list, enable_syscall_sandboxing, ViolationAction};

    allow_list! {
        static ALLOWED = [
            ..SERVICE_BASICS,
            ..ASYNC,
            ..NET_SOCKET_API
        ]
    }

    enable_syscall_sandboxing(ViolationAction::KillProcess, &ALLOWED)
}
