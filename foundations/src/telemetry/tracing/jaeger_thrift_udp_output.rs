use super::internal::FinishedSpan;
use crate::telemetry::settings::JaegerThriftUdpOutputSettings;
use crate::{BootstrapResult, ServiceInfo};
use anyhow::bail;
use crossbeam_channel::Receiver;
use rustracing::tag::Tag;
use rustracing_jaeger::reporter::JaegerCompactReporter;
use std::net::Ipv6Addr;
use std::thread;
use std::time::Duration;

#[cfg(feature = "logging")]
use crate::telemetry::log;

pub(super) fn start(
    service_info: &ServiceInfo,
    settings: &JaegerThriftUdpOutputSettings,
    span_rx: Receiver<FinishedSpan>,
) -> BootstrapResult<()> {
    const REPORTER_COOLDOWN_PERIOD: Duration = Duration::from_secs(2);

    let mut reporter = JaegerCompactReporter::new(service_info.name)?;

    reporter.add_service_tag(Tag::new("app.version", service_info.version));
    reporter.set_agent_addr(settings.server_addr.into());

    match settings.reporter_bind_addr {
        Some(addr) => {
            // the reporter socket will attempt to send traffic to the
            // agent address, so they have to use the same address family
            if settings.server_addr.is_ipv6() == addr.is_ipv6() {
                reporter.set_reporter_addr(addr.into())?;
            } else {
                bail!("`jaeger_tracing_server_addr` and `jaeger_reporter_bind_addr` must have the same address family");
            }
        }
        None => {
            // caused by https://github.com/sile/rustracing_jaeger/blob/bc7d03f2f6ac6bc0269542089c8907279706ecb7/src/reporter.rs#L34,
            // we need to also set the reporter to an ipv6 when agent is ipv6
            if settings.server_addr.is_ipv6() {
                reporter.set_reporter_addr((Ipv6Addr::LOCALHOST, 0).into())?;
            }
        }
    };

    thread::spawn(move || {
        while let Ok(span) = span_rx.recv() {
            if let Err(e) = reporter.report(&[span][..]) {
                #[cfg(feature = "logging")]
                log::error!("failed to send a tracing span to the agent"; "error" => %e);

                #[cfg(not(feature = "logging"))]
                drop(e);

                thread::sleep(REPORTER_COOLDOWN_PERIOD);
            }
        }
    });

    Ok(())
}
