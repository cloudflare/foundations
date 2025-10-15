use super::internal::reporter_error;
use crate::telemetry::otlp_conversion::tracing::convert_span;
use crate::telemetry::settings::OpenTelemetryGrpcOutputSettings;
use crate::{BootstrapResult, ServiceInfo};
use anyhow::Context as _;
use cf_rustracing_jaeger::span::SpanReceiver;
use futures_util::future::{BoxFuture, FutureExt as _};
use http::uri::PathAndQuery;
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use opentelemetry_proto::tonic::trace::v1::ResourceSpans;
use std::time::Duration;
use tonic::client::Grpc;
use tonic::transport::Channel;
use tonic::{GrpcMethod, Request};
use tonic_prost::ProstCodec;

static COLLECTOR_PATH: &str = "/opentelemetry.proto.collector.trace.v1.TraceService/Export";
static TRACE_SERVICE: &str = "opentelemetry.proto.collector.trace.v1.TraceService";

pub(super) fn start(
    service_info: ServiceInfo,
    settings: &OpenTelemetryGrpcOutputSettings,
    span_rx: SpanReceiver,
) -> BootstrapResult<BoxFuture<'static, BootstrapResult<()>>> {
    let max_batch_size = settings.max_batch_size;

    let grpc_channel = Channel::from_shared(format!("{}/v1/traces", settings.endpoint_url))?
        .timeout(Duration::from_secs(settings.request_timeout_seconds));

    // NOTE: don't do any IO or tokio stuff yet - it should be driven by the telemetry driver
    Ok(async move {
        let grpc_channel = grpc_channel
            .connect()
            .await
            .context("failed to connect gRPC channel for traces")?;

        let client = Grpc::new(grpc_channel);

        do_export(client, service_info, span_rx, max_batch_size).await;

        Ok(())
    }
    .boxed())
}

async fn do_export(
    mut client: Grpc<Channel>,
    service_info: ServiceInfo,
    mut span_rx: SpanReceiver,
    max_batch_size: usize,
) {
    let mut batch = Vec::with_capacity(max_batch_size);

    loop {
        let received = span_rx.recv_many(&mut batch, max_batch_size).await;

        if received == 0 {
            break;
        }

        let resource_spans = batch
            .drain(..)
            .map(|span| convert_span(span, &service_info))
            .collect();

        if let Err(err) = client.ready().await {
            reporter_error(err);
            continue;
        }

        let send_res = client
            .unary::<_, ExportTraceServiceResponse, _>(
                create_request(resource_spans),
                PathAndQuery::from_static(COLLECTOR_PATH),
                ProstCodec::default(),
            )
            .await;

        if let Err(err) = send_res {
            reporter_error(err);
        }
    }
}

fn create_request(resource_spans: Vec<ResourceSpans>) -> Request<ExportTraceServiceRequest> {
    let mut request = Request::new(ExportTraceServiceRequest { resource_spans });

    request
        .extensions_mut()
        .insert(GrpcMethod::new(TRACE_SERVICE, "Export"));

    request
}
