use super::internal::{FinishedSpan, SharedSpan, Tracer};
use super::jaeger_thrift_udp_output;
use crate::telemetry::scope::ScopeStack;
use crate::telemetry::settings::{TracesOutput, TracingSettings};
use crate::{BootstrapResult, ServiceInfo};
use crossbeam_channel::Receiver;
use once_cell::sync::{Lazy, OnceCell};

#[cfg(feature = "testing")]
use std::borrow::Cow;

use crate::telemetry::tracing::rate_limit::RateLimitingProbabilisticSampler;

static HARNESS: OnceCell<TracingHarness> = OnceCell::new();

static NOOP_HARNESS: Lazy<TracingHarness> = Lazy::new(|| {
    let (noop_tracer, _) = Tracer::new(Default::default());

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

    let tracer = Tracer::with_sender(RateLimitingProbabilisticSampler::new(settings)?, span_tx);

    Ok((tracer, span_rx))
}

// NOTE: does nothing if tracing has already been initialized in this process.
pub(crate) fn init(service_info: &ServiceInfo, settings: &TracingSettings) -> BootstrapResult<()> {
    if settings.enabled {
        let (tracer, span_rx) = create_tracer_and_span_rx(settings, false)?;

        match &settings.output {
            TracesOutput::JaegerThriftUdp(output_settings) => {
                jaeger_thrift_udp_output::start(service_info, output_settings, span_rx)?
            }
        }

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
