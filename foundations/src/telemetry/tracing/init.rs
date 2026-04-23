use super::channel::SharedSpanReceiver;
use super::internal::{SharedSpan, Tracer};
use super::live::ActiveRoots;
use super::output_jaeger_thrift_udp;
use super::rate_limit::RateLimitingProbabilisticSampler;
use crate::telemetry::scope::ScopeStack;
use crate::telemetry::settings::{SamplingStrategy, TracesOutput, TracingSettings};
use crate::{BootstrapResult, ServiceInfo};
use cf_rustracing::sampler::{NullSampler, PassiveSampler, Sampler};
use crossbeam_utils::CachePadded;
use futures_util::future::BoxFuture;
use std::sync::{LazyLock, OnceLock};

#[cfg(feature = "telemetry-otlp-grpc")]
use super::output_otlp_grpc;

#[cfg(feature = "testing")]
use std::borrow::Cow;

// These singletons are accessed _very often_, and each access requires an atomic load to
// ensure initialization. Make sure nobody else invalidates our cache lines.
static HARNESS: CachePadded<OnceLock<TracingHarness>> = CachePadded::new(OnceLock::new());

static NOOP_HARNESS: CachePadded<LazyLock<TracingHarness>> =
    CachePadded::new(LazyLock::new(|| {
        let (noop_tracer, _) = Tracer::new(NullSampler.boxed());

        TracingHarness {
            tracer: noop_tracer,
            span_scope_stack: Default::default(),

            #[cfg(feature = "testing")]
            test_tracer_scope_stack: Default::default(),

            active_roots: Default::default(),
        }
    }));

pub(crate) struct TracingHarness {
    tracer: Tracer,

    pub(crate) span_scope_stack: ScopeStack<SharedSpan>,

    #[cfg(feature = "testing")]
    pub(crate) test_tracer_scope_stack: ScopeStack<Tracer>,

    pub(crate) active_roots: crate::telemetry::tracing::live::ActiveRoots,
}

impl TracingHarness {
    pub(crate) fn get() -> &'static Self {
        HARNESS.get().unwrap_or_else(|| &**NOOP_HARNESS)
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

pub(super) fn create_tracer_and_span_rx(
    settings: &TracingSettings,
) -> BootstrapResult<(Tracer, SharedSpanReceiver)> {
    let sampler = match &settings.sampling_strategy {
        SamplingStrategy::Passive => PassiveSampler.boxed(),
        SamplingStrategy::Active(settings) => {
            RateLimitingProbabilisticSampler::new(settings)?.boxed()
        }
    };

    if let Some(cap) = settings.max_queue_size {
        let (consumer, span_rx) = super::channel::channel(cap);
        let tracer = Tracer::with_consumer(sampler, consumer);
        Ok((tracer, span_rx))
    } else {
        let (consumer, span_rx) = super::channel::unbounded_channel();
        let tracer = Tracer::with_consumer(sampler, consumer);
        Ok((tracer, span_rx))
    }
}

pub(super) struct TraceOutputFutures {
    pub initializer: Option<BoxFuture<'static, BootstrapResult<()>>>,
    pub workers: Vec<BoxFuture<'static, ()>>,
}

// NOTE: does nothing if tracing has already been initialized in this process.
pub(crate) fn init(
    service_info: &ServiceInfo,
    settings: &TracingSettings,
) -> BootstrapResult<Option<BoxFuture<'static, BootstrapResult<()>>>> {
    if !settings.enabled || HARNESS.get().is_some() {
        return Ok(None);
    }

    let (tracer, span_rx) = create_tracer_and_span_rx(settings)?;

    let futs = match &settings.output {
        TracesOutput::JaegerThriftUdp(output_settings) => {
            output_jaeger_thrift_udp::start(service_info, output_settings, span_rx)?
        }
        #[cfg(feature = "telemetry-otlp-grpc")]
        TracesOutput::OpenTelemetryGrpc(output_settings) => {
            output_otlp_grpc::start(service_info, output_settings, span_rx)?
        }
    };

    // Only spawn the futures if we are actually initializing the harness
    let mut res = Ok(None);
    HARNESS.get_or_init(|| {
        #[cfg(feature = "metrics")]
        {
            let max_queue_size = settings
                .max_queue_size
                .map(std::num::NonZeroUsize::get)
                .unwrap_or(usize::MAX);
            super::metrics::tracing::max_queue_size().set(max_queue_size as u64);
        }

        res = Ok(futs.initializer);
        for f in futs.workers {
            tokio::spawn(f);
        }

        TracingHarness {
            tracer,
            span_scope_stack: Default::default(),

            #[cfg(feature = "testing")]
            test_tracer_scope_stack: Default::default(),

            active_roots: ActiveRoots::new(settings.liveness_tracking.clone()),
        }
    });
    res
}
