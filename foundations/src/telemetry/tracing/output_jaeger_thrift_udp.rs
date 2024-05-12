use crate::telemetry::settings::JaegerThriftUdpOutputSettings;
use crate::{BootstrapResult, ServiceInfo};
use anyhow::bail;
use cf_rustracing::tag::Tag;
use cf_rustracing_jaeger::reporter::JaegerCompactReporter;
use cf_rustracing_jaeger::span::SpanReceiver;
use futures_util::future::{BoxFuture, FutureExt as _};
use std::net::SocketAddr;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

#[cfg(feature = "logging")]
use crate::telemetry::log;

pub(super) fn start(
    service_info: ServiceInfo,
    settings: &JaegerThriftUdpOutputSettings,
    span_rx: SpanReceiver,
) -> BootstrapResult<BoxFuture<'static, BootstrapResult<()>>> {
    let server_addr = settings.server_addr.into();
    let reporter_bind_addr = get_reporter_bind_addr(settings)?;

    // NOTE: do socket binding as early as possible. It's a good practice to disable binding
    // with seccomp after the service initialisaion.
    let socket = std::net::UdpSocket::bind(reporter_bind_addr)?;

    socket.set_nonblocking(true)?;

    Ok(async move {
        let mut reporter = JaegerCompactReporter::new_with_transport(
            service_info.name,
            server_addr,
            tokio::net::UdpSocket::from_std(socket)?,
        )?;

        reporter.add_service_tag(Tag::new("app.version", service_info.version));

        do_export(reporter, span_rx).await;

        Ok(())
    }
    .boxed())
}

fn get_reporter_bind_addr(settings: &JaegerThriftUdpOutputSettings) -> BootstrapResult<SocketAddr> {
    Ok(match settings.reporter_bind_addr {
        Some(addr) => {
            // the reporter socket will attempt to send traffic to the
            // agent address, so they have to use the same address family
            if settings.server_addr.is_ipv6() == addr.is_ipv6() {
                addr.into()
            } else {
                bail!("`jaeger_tracing_server_addr` and `jaeger_reporter_bind_addr` must have the same address family");
            }
        }
        // caused by https://github.com/sile/rustracing_jaeger/blob/bc7d03f2f6ac6bc0269542089c8907279706ecb7/src/reporter.rs#L34,
        // we need to also set the reporter to an ipv6 when agent is ipv6
        None if settings.server_addr.is_ipv6() => (Ipv6Addr::LOCALHOST, 0).into(),
        None => (Ipv4Addr::LOCALHOST, 0).into(),
    })
}

async fn do_export(reporter: JaegerCompactReporter, mut span_rx: SpanReceiver) {
    let reporter = Arc::new(reporter);

    while let Some(span) = span_rx.recv().await {
        // NOTE: we are limited with a UDP dgram size here, so doing batching is risky.
        tokio::spawn({
            let reporter = Arc::clone(&reporter);

            async move {
                if let Err(e) = reporter.report(&[span][..]).await {
                    #[cfg(feature = "logging")]
                    log::error!("failed to send a tracing span to the agent"; "error" => %e);

                    #[cfg(not(feature = "logging"))]
                    drop(e);
                }
            }
        });
    }
}
