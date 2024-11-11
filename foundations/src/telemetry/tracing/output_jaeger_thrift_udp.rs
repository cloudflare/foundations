use super::internal::reporter_error;
use crate::telemetry::settings::JaegerThriftUdpOutputSettings;
use crate::{BootstrapResult, ServiceInfo};
use anyhow::bail;
use cf_rustracing::tag::Tag;
use cf_rustracing_jaeger::reporter::JaegerCompactReporter;
use cf_rustracing_jaeger::span::SpanReceiver;
use futures_util::future::{BoxFuture, FutureExt as _};
use std::net::SocketAddr;
use std::net::{Ipv4Addr, Ipv6Addr};

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
    while let Some(span) = span_rx.recv().await {
        // NOTE: we are limited with a UDP dgram size here, so doing batching is risky.
        let spans = [span];
        if let Err(err) = reporter.report(&spans).await {
            #[cfg(feature = "logging")]
            if self::logging::is_msgsize_error(&err) {
                self::logging::log_span_too_large_err(&err, &spans[0]);
                continue;
            }

            reporter_error(err);
        }
    }
}

#[cfg(feature = "logging")]
mod logging {
    use cf_rustracing::tag::Tag;
    use cf_rustracing_jaeger::span::FinishedSpan;
    use std::io;

    pub(super) fn is_msgsize_error(err: &cf_rustracing::Error) -> bool {
        err.concrete_cause::<io::Error>()
            .and_then(io::Error::raw_os_error)
            == Some(libc::EMSGSIZE)
    }

    pub(super) fn log_span_too_large_err(err: &cf_rustracing::Error, span: &FinishedSpan) {
        let tag_count = span.tags().len();
        let tag_total: usize = span.tags().iter().map(tag_size).sum();
        let top_tag = span.tags().iter().max_by_key(|t| tag_size(t));

        let log_count = span.logs().len();
        let log_total: usize = span.logs().iter().map(log_size).sum();

        let top_tag = top_tag
            .map(|t| format!(", top: {} @ approx {}", t.name(), tag_size(t)))
            .unwrap_or_default();

        crate::telemetry::log::error!(
            "trace span exceeded thrift UDP message size limits";
            "error" => %err,
            "operation" => span.operation_name(),
            "tags" => format!("count: {tag_count}, size: approx {tag_total}{top_tag}"),
            "logs" => format!("count: {log_count}, size: approx {log_total}"),
        );
    }

    /// Approximates the wire size of a span `Tag`. This is not exact and
    /// more closely resembles the non-compact thrift encoding, but should be
    /// sufficient to determine what causes the span to trigger EMSGSIZE.
    fn tag_size(tag: &Tag) -> usize {
        use cf_rustracing::tag::TagValue;
        let val_size = match tag.value() {
            TagValue::String(s) => s.len(),
            TagValue::Boolean(_) => 1,
            TagValue::Integer(_) => 8,
            TagValue::Float(_) => 8,
        };
        tag.name().len() + val_size
    }

    /// Approximates the wire size of a span `Log`, which is always stringified.
    fn log_size(log: &cf_rustracing::log::Log) -> usize {
        log.fields()
            .iter()
            .map(|f| f.name().len() + f.value().len())
            .sum()
    }
}
