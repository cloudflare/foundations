use super::internal::{SharedSpan, Tracer};
use super::output_jaeger_thrift_udp;
use crate::telemetry::scope::ScopeStack;
use crate::telemetry::settings::{SamplingStrategy, TracesOutput, TracingSettings};
use crate::{BootstrapResult, ServiceInfo};
use cf_rustracing_jaeger::span::SpanReceiver;
use futures_util::future::BoxFuture;
use once_cell::sync::{Lazy, OnceCell};

#[cfg(feature = "telemetry-otlp-grpc")]
use super::output_otlp_grpc;

use cf_rustracing::sampler::{PassiveSampler, Sampler};
#[cfg(feature = "testing")]
use std::borrow::Cow;

use crate::telemetry::tracing::rate_limit::RateLimitingProbabilisticSampler;

static HARNESS: OnceCell<TracingHarness> = OnceCell::new();

static NOOP_HARNESS: Lazy<TracingHarness> = Lazy::new(|| {
    let (noop_tracer, _) = Tracer::new(RateLimitingProbabilisticSampler::default().boxed());

    TracingHarness {
        tracer: noop_tracer,
        span_scope_stack: Default::default(),

        #[cfg(feature = "testing")]
        test_tracer_scope_stack: Default::default(),

        #[cfg(feature = "telemetry-server")]
        active_roots: Default::default(),
    }
});

pub(crate) struct TracingHarness {
    tracer: Tracer,

    pub(crate) span_scope_stack: ScopeStack<SharedSpan>,

    #[cfg(feature = "testing")]
    pub(crate) test_tracer_scope_stack: ScopeStack<Tracer>,

    #[cfg(feature = "telemetry-server")]
    pub(crate) active_roots: crate::telemetry::tracing::live::ActiveRoots,
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
    pub(crate) fn tracer(&'static self) -> &'static Tracer {
        &self.tracer
    }
}

pub(crate) fn create_tracer_and_span_rx(
    settings: &TracingSettings,
) -> BootstrapResult<(Tracer, SpanReceiver)> {
    let sampler = match &settings.sampling_strategy {
        SamplingStrategy::Passive => PassiveSampler.boxed(),
        SamplingStrategy::Active(settings) => {
            RateLimitingProbabilisticSampler::new(settings)?.boxed()
        }
    };

    Ok(Tracer::new(sampler))
}

// NOTE: does nothing if tracing has already been initialized in this process.
pub(crate) fn init(
    service_info: ServiceInfo,
    settings: &TracingSettings,
) -> BootstrapResult<Option<BoxFuture<'static, BootstrapResult<()>>>> {
    let reporter_fut = if settings.enabled {
        let (tracer, span_rx) = create_tracer_and_span_rx(settings)?;

        let reporter_fut = match &settings.output {
            TracesOutput::JaegerThriftUdp(output_settings) => {
                output_jaeger_thrift_udp::start(service_info, output_settings, span_rx)?
            }
            #[cfg(feature = "telemetry-otlp-grpc")]
            TracesOutput::OpenTelemetryGrpc(output_settings) => {
                output_otlp_grpc::start(service_info, output_settings, span_rx)?
            }
        };

        let harness = TracingHarness {
            tracer,
            span_scope_stack: Default::default(),

            #[cfg(feature = "testing")]
            test_tracer_scope_stack: Default::default(),

            #[cfg(feature = "telemetry-server")]
            active_roots: Default::default(),
        };

        let _ = HARNESS.set(harness);

        Some(reporter_fut)
    } else {
        None
    };

    Ok(reporter_fut)
}
