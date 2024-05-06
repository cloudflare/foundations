use crate::telemetry::otlp_conversion::tracing::convert_span;
use crate::telemetry::settings::OpenTelemetryGrpcOutputSettings;
use crate::{BootstrapResult, ServiceInfo};
use anyhow::Context as _;
use cf_rustracing_jaeger::span::SpanReceiver;
use futures_util::future::{BoxFuture, FutureExt as _};
use std::time::Duration;
use tonic::client::Grpc;
use tonic::transport::Channel;

pub(super) fn start(
    service_info: ServiceInfo,
    settings: &OpenTelemetryGrpcOutputSettings,
    span_rx: SpanReceiver,
) -> BootstrapResult<BoxFuture<'static, BootstrapResult<()>>> {
    let grpc_channel = Channel::from_shared(format!("{}/v1/traces", settings.endpoint_url))?
        .timeout(Duration::from_secs(settings.request_timeout_seconds));

    // NOTE: don't do any IO or tokio stuff yet - it should be driven by the telemetry driver
    Ok(async move {
        let grpc_channel = grpc_channel
            .connect()
            .await
            .context("failed to connect gRPC channel for traces")?;

        let client = Grpc::new(grpc_channel);

        do_export(client, service_info, span_rx).await
    }
    .boxed())
}

async fn do_export(
    client: Grpc<Channel>,
    service_info: ServiceInfo,
    span_rx: SpanReceiver,
) -> BootstrapResult<()> {
    todo!();

    Ok(())
}
