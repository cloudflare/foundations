use super::internal::{FinishedSpan, SharedSpan, Tracer};
use super::settings::TracingSettings;
use crate::telemetry::scope::ScopeStack;
use crate::{BootstrapResult, ServiceInfo};
use crossbeam_channel::Receiver;
use once_cell::sync::{Lazy, OnceCell};
use rustracing::sampler::ProbabilisticSampler;
use rustracing::tag::Tag;
use rustracing_jaeger::reporter::JaegerCompactReporter;
use std::thread;
use std::time::Duration;

#[cfg(feature = "testing")]
use std::borrow::Cow;

#[cfg(feature = "logging")]
use crate::telemetry::log;

static HARNESS: OnceCell<TracingHarness> = OnceCell::new();

static NOOP_HARNESS: Lazy<TracingHarness> = Lazy::new(|| {
    let (noop_tracer, _) = Tracer::new(ProbabilisticSampler::new(0.0).unwrap());

    TracingHarness {
        tracer: noop_tracer,
        span_scope_stack: Default::default(),

        #[cfg(feature = "testing")]
        test_tracer_scope_stack: Default::default(),
    }
});

pub(crate) struct TracingHarness {
    tracer: Tracer,

    pub(crate) span_scope_stack: ScopeStack<SharedSpan>,

    #[cfg(feature = "testing")]
    pub(crate) test_tracer_scope_stack: ScopeStack<Tracer>,
}

impl TracingHarness {
    pub(crate) fn get() -> &'static Self {
        HARNESS.get().unwrap_or(&NOOP_HARNESS)
    }

    #[cfg(feature = "testing")]
    pub(crate) fn tracer(&'static self) -> Cow<'static, Tracer> {
        self.test_tracer_scope_stack
            .current()
            .map(Cow::Owned)
            .unwrap_or_else(|| Cow::Borrowed(&self.tracer))
    }

    #[cfg(not(feature = "testing"))]
    pub(crate) fn tracer(&'static self) -> &Tracer {
        &self.tracer
    }
}

pub(crate) fn create_tracer_and_span_rx(
    settings: &TracingSettings,
    with_unbounded_chan: bool,
) -> BootstrapResult<(Tracer, Receiver<FinishedSpan>)> {
    const SPAN_CHANNEL_CAPACITY: usize = 30;

    let (span_tx, span_rx) = if with_unbounded_chan {
        crossbeam_channel::unbounded()
    } else {
        crossbeam_channel::bounded(SPAN_CHANNEL_CAPACITY)
    };

    let tracer = Tracer::with_sender(ProbabilisticSampler::new(settings.sampling_ratio)?, span_tx);

    Ok((tracer, span_rx))
}

// NOTE: does nothing if tracing has already been initialized in this process.
pub(crate) fn init(service_info: ServiceInfo, settings: &TracingSettings) -> BootstrapResult<()> {
    if settings.enabled {
        let (tracer, span_rx) = create_tracer_and_span_rx(settings, false)?;

        start_reporter(service_info, settings, span_rx)?;

        let harness = TracingHarness {
            tracer,
            span_scope_stack: Default::default(),

            #[cfg(feature = "testing")]
            test_tracer_scope_stack: Default::default(),
        };

        let _ = HARNESS.set(harness);
    }

    Ok(())
}

fn start_reporter(
    service_info: ServiceInfo,
    settings: &TracingSettings,
    span_rx: Receiver<FinishedSpan>,
) -> BootstrapResult<()> {
    const REPORTER_COOLDOWN_PERIOD: Duration = Duration::from_secs(2);

    let mut reporter = JaegerCompactReporter::new(service_info.name)?;

    reporter.add_service_tag(Tag::new("app.version", service_info.version));
    reporter.set_agent_addr(settings.jaeger_tracing_server_addr.into());

    thread::spawn(move || {
        while let Ok(span) = span_rx.recv() {
            if let Err(e) = reporter.report(&[span][..]) {
                #[cfg(feature = "logging")]
                log::warn!("failed to send a tracing span to the agent"; "error" => %e);

                #[cfg(not(feature = "logging"))]
                drop(e);

                thread::sleep(REPORTER_COOLDOWN_PERIOD);
            }
        }
    });

    Ok(())
}
