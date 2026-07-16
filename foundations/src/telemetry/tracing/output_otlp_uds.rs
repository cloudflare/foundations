//! [OTLP-over-UDS] output for the user tracing pipeline.
//!
//! Sends protobuf-encoded OTLP trace data over HTTP/1.1 to a Unix domain
//! socket served by a local OTLP endpoint.

use super::channel::SharedSpanReceiver;
use super::init::TraceOutputFutures;
use super::internal::reporter_error;
use crate::telemetry::otlp_conversion::tracing::convert_span;
use crate::telemetry::settings::OtlpUdsOutputSettings;
use crate::{BootstrapResult, ServiceInfo};
use anyhow::ensure;
use cf_rustracing_jaeger::span::FinishedSpan;
use futures_util::future::FutureExt as _;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::header::{CONTENT_TYPE, HOST};
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::trace::v1::ResourceSpans;
use prost::Message as _;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::UnixStream;

const TRACES_PATH: &str = "/v1/traces";
const CONTENT_TYPE_PROTOBUF: &str = "application/x-protobuf";
const HOST_HEADER_VALUE: &str = "localhost";

/// A failure exporting a single OTLP request over the Unix domain socket.
///
/// This is the concrete error type for the UDS client, mirroring how the
/// existing exporters surface a concrete library error (`cf_rustracing::Error`,
/// `tonic::Status`) to [`reporter_error`].
#[derive(Debug)]
enum OtlpUdsExportError {
    Connect(std::io::Error),
    Handshake(hyper::Error),
    BuildRequest(http::Error),
    Send(hyper::Error),
    Status(StatusCode),
}

impl std::fmt::Display for OtlpUdsExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(err) => write!(f, "failed to connect to UDS socket: {err}"),
            Self::Handshake(err) => write!(f, "HTTP/1 handshake over UDS failed: {err}"),
            Self::BuildRequest(err) => write!(f, "failed to build OTLP UDS request: {err}"),
            Self::Send(err) => write!(f, "failed to send OTLP UDS request: {err}"),
            Self::Status(status) => {
                write!(f, "OTLP UDS receptor returned non-success status: {status}")
            }
        }
    }
}

impl std::error::Error for OtlpUdsExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Connect(err) => Some(err),
            Self::Handshake(err) => Some(err),
            Self::BuildRequest(err) => Some(err),
            Self::Send(err) => Some(err),
            Self::Status(_) => None,
        }
    }
}

/// Exports user tracing spans as OTLP over a Unix domain socket.
#[derive(Debug)]
pub(super) struct OtlpUdsClient {
    socket_path: String,
    routing_header: String,
}

impl OtlpUdsClient {
    pub(super) fn new(settings: &OtlpUdsOutputSettings) -> BootstrapResult<Self> {
        ensure!(
            !settings.socket_path.is_empty(),
            "user tracing OTLP UDS `socket_path` must be set"
        );
        ensure!(
            !settings.routing_header_name.is_empty(),
            "user tracing OTLP UDS `routing_header_name` must be set"
        );

        Ok(Self {
            socket_path: settings.socket_path.clone(),
            routing_header: settings.routing_header_name.clone(),
        })
    }

    /// Processes a single drained batch of spans: groups them by routing,
    /// converts each to OTLP, and POSTs one request per group. Errors are
    /// reported and do not abort the batch.
    async fn process_batch(&self, service_info: &ServiceInfo, spans: &mut Vec<FinishedSpan>) {
        // Group spans by routing so each request carries a single routing value
        // in its header, encoded once per group.
        let mut groups: HashMap<String, (String, Vec<ResourceSpans>)> = HashMap::new();

        for span in spans.drain(..) {
            // Spans without routing aren't user-traced spans we can route, so
            // drop them. Borrow routing before `convert_span` consumes the span.
            let Some(routing) = span.routing() else {
                continue;
            };
            let entry = groups
                .entry(routing.group_key())
                .or_insert_with(|| (routing.encode(), Vec::new()));

            entry.1.push(convert_span(span, service_info));
        }

        for (_group_key, (header_value, resource_spans)) in groups {
            let body = ExportTraceServiceRequest { resource_spans }.encode_to_vec();

            if let Err(err) = self.send(body, header_value).await {
                reporter_error(err);
            }
        }
    }

    /// POSTs a single OTLP request body to the receptor, tagged with the routing
    /// header.
    async fn send(&self, body: Vec<u8>, header_value: String) -> Result<(), OtlpUdsExportError> {
        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(OtlpUdsExportError::Connect)?;

        let (mut send_request, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(OtlpUdsExportError::Handshake)?;

        // Drive the connection in the background; request/response errors are
        // surfaced via `send_request` below, and the driver completes once the
        // exchange is done.
        tokio::spawn(async move {
            let _ = conn.await;
        });

        let request = Request::builder()
            .method(Method::POST)
            .uri(TRACES_PATH)
            .header(HOST, HOST_HEADER_VALUE)
            .header(CONTENT_TYPE, CONTENT_TYPE_PROTOBUF)
            .header(self.routing_header.as_str(), header_value)
            .body(Full::new(Bytes::from(body)))
            .map_err(OtlpUdsExportError::BuildRequest)?;

        let response = send_request
            .send_request(request)
            .await
            .map_err(OtlpUdsExportError::Send)?;

        let status = response.status();
        if !status.is_success() {
            return Err(OtlpUdsExportError::Status(status));
        }

        Ok(())
    }
}

pub(super) fn start(
    service_info: &ServiceInfo,
    settings: &OtlpUdsOutputSettings,
    span_rx: SharedSpanReceiver,
) -> BootstrapResult<TraceOutputFutures> {
    let client = Arc::new(OtlpUdsClient::new(settings)?);
    let max_batch_size = settings.max_batch_size;

    let workers = (0..settings.num_tasks)
        .map(|_| {
            let client = Arc::clone(&client);
            let service_info = service_info.clone();
            let span_rx = span_rx.clone();

            async move { do_export(client, service_info, span_rx, max_batch_size).await }.boxed()
        })
        .collect();

    Ok(TraceOutputFutures {
        initializer: None,
        workers,
    })
}

/// Drains the span channel and hands each batch to the client for export.
async fn do_export(
    client: Arc<OtlpUdsClient>,
    service_info: ServiceInfo,
    span_rx: SharedSpanReceiver,
    max_batch_size: usize,
) {
    let mut batch = Vec::with_capacity(max_batch_size);

    while span_rx.recv_many(&mut batch, max_batch_size).await > 0 {
        client.process_batch(&service_info, &mut batch).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cf_rustracing::span::RoutingMetadata;
    use http_body_util::BodyExt as _;
    use hyper::Response;
    use hyper::body::Incoming;
    use hyper::service::service_fn;
    use std::convert::Infallible;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;
    use tokio::net::UnixListener;
    use tokio::sync::mpsc;

    const TEST_ROUTING_HEADER: &str = "cf-trace-config";

    #[derive(Debug)]
    struct TestRouting {
        zone_id: u64,
        account_id: u64,
    }

    impl RoutingMetadata for TestRouting {
        fn group_key(&self) -> String {
            format!("{}|{}", self.zone_id, self.account_id)
        }

        fn encode(&self) -> String {
            serde_json::json!({
                "zoneId": self.zone_id,
                "accountId": self.account_id,
            })
            .to_string()
        }
    }

    struct CapturedRequest {
        method: String,
        path: String,
        host: Option<String>,
        content_type: Option<String>,
        trace_config: Option<String>,
        body: Vec<u8>,
    }

    /// Binds a UDS "receptor" that captures the first request it receives and
    /// replies with `status`. The returned `TempDir` must be kept alive for the
    /// socket file to remain valid.
    fn spawn_receptor(
        status: StatusCode,
    ) -> (PathBuf, TempDir, mpsc::UnboundedReceiver<CapturedRequest>) {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("otlp.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();

        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();

            let service = service_fn(move |req: Request<Incoming>| {
                let tx = tx.clone();

                async move {
                    let (parts, body) = req.into_parts();
                    let headers = &parts.headers;
                    let get = |name: &str| {
                        headers
                            .get(name)
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned)
                    };

                    let captured = CapturedRequest {
                        method: parts.method.to_string(),
                        path: parts.uri.path().to_string(),
                        host: get("host"),
                        content_type: get("content-type"),
                        trace_config: get(TEST_ROUTING_HEADER),
                        body: body.collect().await.unwrap().to_bytes().to_vec(),
                    };

                    tx.send(captured).ok();

                    Ok::<_, Infallible>(
                        Response::builder()
                            .status(status)
                            .body(Full::new(Bytes::new()))
                            .unwrap(),
                    )
                }
            });

            hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
                .ok();
        });

        (socket_path, dir, rx)
    }

    fn settings_for(socket_path: &Path) -> OtlpUdsOutputSettings {
        OtlpUdsOutputSettings {
            socket_path: socket_path.to_string_lossy().into_owned(),
            routing_header_name: TEST_ROUTING_HEADER.to_string(),
            num_tasks: 1,
            max_batch_size: 8,
        }
    }

    #[tokio::test]
    async fn new_rejects_empty_socket_path() {
        let err = OtlpUdsClient::new(&OtlpUdsOutputSettings {
            socket_path: String::new(),
            routing_header_name: TEST_ROUTING_HEADER.to_string(),
            num_tasks: 1,
            max_batch_size: 8,
        })
        .unwrap_err();

        assert!(err.to_string().contains("socket_path"));
    }

    #[tokio::test]
    async fn send_posts_otlp_with_headers_and_body() {
        let (socket_path, _dir, mut rx) = spawn_receptor(StatusCode::OK);

        let client = OtlpUdsClient::new(&settings_for(&socket_path)).unwrap();

        let body = b"hello-otlp".to_vec();
        let trace_config = r#"{"zone_id":"z1"}"#.to_string();

        client
            .send(body.clone(), trace_config.clone())
            .await
            .unwrap();

        let captured = rx.recv().await.unwrap();
        assert_eq!(captured.method, "POST");
        assert_eq!(captured.path, TRACES_PATH);
        assert_eq!(captured.host.as_deref(), Some(HOST_HEADER_VALUE));
        assert_eq!(
            captured.content_type.as_deref(),
            Some(CONTENT_TYPE_PROTOBUF)
        );
        assert_eq!(
            captured.trace_config.as_deref(),
            Some(trace_config.as_str())
        );
        assert_eq!(captured.body, body);
    }

    #[tokio::test]
    async fn send_errors_on_non_success_status() {
        let (socket_path, _dir, _rx) = spawn_receptor(StatusCode::INTERNAL_SERVER_ERROR);

        let client = OtlpUdsClient::new(&settings_for(&socket_path)).unwrap();

        let err = client
            .send(b"x".to_vec(), "{}".to_string())
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            OtlpUdsExportError::Status(StatusCode::INTERNAL_SERVER_ERROR)
        ));
        assert!(err.to_string().contains("non-success status"));
    }

    // Drives the full path: a span produced through a tracer (with routing set)
    // flows through the channel, is converted + encoded by `process_batch`, and
    // arrives at the receptor with its routing in the `cf-trace-config` header.
    #[tokio::test]
    async fn process_batch_sends_converted_spans() {
        use super::super::channel::{PipelineType, unbounded_channel};
        use cf_rustracing::Tracer;
        use cf_rustracing::sampler::AllSampler;

        let (socket_path, _dir, mut rx) = spawn_receptor(StatusCode::OK);

        let (sender, span_rx) = unbounded_channel(PipelineType::User);

        // Produce one finished span with routing, then drop the tracer so the
        // channel closes and the worker loop terminates after draining.
        {
            let tracer = Tracer::with_consumer(AllSampler, sender);
            let _span = tracer
                .span("user-root")
                .routing(Arc::new(TestRouting {
                    zone_id: 12345,
                    account_id: 42,
                }))
                .start();
        }

        let service_info = crate::service_info!();
        let futs = start(&service_info, &settings_for(&socket_path), span_rx).unwrap();
        for worker in futs.workers {
            tokio::spawn(worker);
        }

        let captured = rx.recv().await.unwrap();
        assert_eq!(captured.method, "POST");
        assert_eq!(captured.path, TRACES_PATH);
        assert_eq!(
            captured.content_type.as_deref(),
            Some(CONTENT_TYPE_PROTOBUF)
        );
        let trace_config: serde_json::Value =
            serde_json::from_str(captured.trace_config.as_deref().unwrap()).unwrap();
        assert_eq!(trace_config["zoneId"], 12345);
        assert_eq!(trace_config["accountId"], 42);
        // Body is a protobuf-encoded `ExportTraceServiceRequest`.
        assert!(!captured.body.is_empty());
    }

    // Full producer path: `init_user` stands up `USER_HARNESS` + the OTLP/UDS exporter, then
    // `user_tracing::start_trace` + `user_tracing::span` + `user_tracing::add_span_tags!` produce spans that reach the
    // receptor with routing in the configured routing header. (nextest isolates this in its own
    // process, so the one-shot `USER_HARNESS` is fine.)
    #[tokio::test]
    async fn user_pipeline_exports_with_routing() {
        use crate::telemetry::settings::{UserTracesOutput, UserTracingSettings};
        use crate::telemetry::tracing::user_tracing;
        use opentelemetry_proto::tonic::common::v1::any_value::Value;
        use prost::Message as _;

        let (socket_path, _dir, mut rx) = spawn_receptor(StatusCode::OK);

        let settings = UserTracingSettings {
            enabled: true,
            max_queue_size: None,
            output: UserTracesOutput::OtlpUds(settings_for(&socket_path)),
        };

        let service_info = crate::service_info!();
        crate::telemetry::tracing::init::init_user(&service_info, &settings).unwrap();

        {
            let _root = user_tracing::start_trace(
                "request",
                TestRouting {
                    zone_id: 12345,
                    account_id: 42,
                },
                None,
            );

            let _child = user_tracing::span("child");
            user_tracing::add_span_tags!("cache.status" => "HIT");
        }

        let captured = rx.recv().await.unwrap();
        assert_eq!(captured.path, TRACES_PATH);

        let trace_config: serde_json::Value =
            serde_json::from_str(captured.trace_config.as_deref().unwrap()).unwrap();
        assert_eq!(trace_config["zoneId"], 12345);
        assert_eq!(trace_config["accountId"], 42);

        // Decode the OTLP body and verify the producer API actually emitted the expected spans.
        let req = ExportTraceServiceRequest::decode(captured.body.as_slice()).unwrap();
        let spans: Vec<_> = req
            .resource_spans
            .iter()
            .flat_map(|rs| &rs.scope_spans)
            .flat_map(|ss| &ss.spans)
            .collect();

        let root = spans
            .iter()
            .find(|s| s.name == "request")
            .expect("root span exported");
        let child = spans
            .iter()
            .find(|s| s.name == "child")
            .expect("child span exported");

        // `user_tracing::add_span_tags!` wrote to the current user span (the child), not the root.
        let tag = child
            .attributes
            .iter()
            .find(|kv| kv.key == "cache.status")
            .expect("cache.status tag present on child");
        assert!(matches!(
            &tag.value.as_ref().unwrap().value,
            Some(Value::StringValue(v)) if v == "HIT"
        ));
        assert!(!root.attributes.iter().any(|kv| kv.key == "cache.status"));

        // Correct hierarchy: child is a child of root within the same trace.
        assert_eq!(child.trace_id, root.trace_id);
        assert_eq!(child.parent_span_id, root.span_id);
    }

    // Stitching: a root started with an inbound `traceparent` continues that trace on the wire
    // (same 128-bit trace id, and its parent is the inbound parent span id).
    #[tokio::test]
    async fn user_pipeline_continues_inbound_trace() {
        use crate::telemetry::settings::{UserTracesOutput, UserTracingSettings};
        use crate::telemetry::tracing::{TraceparentContext, user_tracing};
        use prost::Message as _;

        let (socket_path, _dir, mut rx) = spawn_receptor(StatusCode::OK);

        let settings = UserTracingSettings {
            enabled: true,
            max_queue_size: None,
            output: UserTracesOutput::OtlpUds(settings_for(&socket_path)),
        };
        crate::telemetry::tracing::init::init_user(&crate::service_info!(), &settings).unwrap();

        let inbound =
            TraceparentContext::parse(b"00-11223344556677889900aabbccddeeff-a1b2c3d4e5f60718-01")
                .unwrap();

        {
            let _root = user_tracing::start_trace(
                "request",
                TestRouting {
                    zone_id: 12345,
                    account_id: 42,
                },
                Some(inbound),
            );
        }

        let captured = rx.recv().await.unwrap();
        let req = ExportTraceServiceRequest::decode(captured.body.as_slice()).unwrap();
        let root = req
            .resource_spans
            .iter()
            .flat_map(|rs| &rs.scope_spans)
            .flat_map(|ss| &ss.spans)
            .find(|s| s.name == "request")
            .expect("root span exported");

        assert_eq!(root.trace_id, inbound.trace_id);
        assert_eq!(root.parent_span_id, inbound.parent_id);
    }

    // Routing set on the root is inherited by all descendants, so a grandchild also exports
    // (the exporter drops spans without routing).
    #[tokio::test]
    async fn user_pipeline_inherits_routing_to_descendants() {
        use crate::telemetry::settings::{UserTracesOutput, UserTracingSettings};
        use crate::telemetry::tracing::user_tracing;
        use prost::Message as _;

        let (socket_path, _dir, mut rx) = spawn_receptor(StatusCode::OK);

        let settings = UserTracingSettings {
            enabled: true,
            max_queue_size: None,
            output: UserTracesOutput::OtlpUds(settings_for(&socket_path)),
        };
        crate::telemetry::tracing::init::init_user(&crate::service_info!(), &settings).unwrap();

        {
            let _root = user_tracing::start_trace(
                "request",
                TestRouting {
                    zone_id: 12345,
                    account_id: 42,
                },
                None,
            );
            let _child = user_tracing::span("child");
            let _grandchild = user_tracing::span("grandchild");
        }

        let captured = rx.recv().await.unwrap();
        let req = ExportTraceServiceRequest::decode(captured.body.as_slice()).unwrap();
        let names: Vec<&str> = req
            .resource_spans
            .iter()
            .flat_map(|rs| &rs.scope_spans)
            .flat_map(|ss| &ss.spans)
            .map(|s| s.name.as_str())
            .collect();

        assert!(names.contains(&"request"));
        assert!(names.contains(&"child"));
        assert!(names.contains(&"grandchild"));
    }
}
