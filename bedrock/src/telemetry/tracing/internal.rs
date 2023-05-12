use super::init::TracingHarness;
use crate::telemetry::scope::Scope;
use rustracing::sampler::ProbabilisticSampler;
use rustracing_jaeger::span::SpanContextState;
use std::sync::Arc;

pub(crate) type Span = rustracing::span::Span<SpanContextState>;
pub(crate) type FinishedSpan = rustracing::span::FinishedSpan<SpanContextState>;
pub(crate) type Tracer = rustracing::Tracer<ProbabilisticSampler, SpanContextState>;

#[must_use]
pub struct SpanScope(Scope<SharedSpan>);

impl SpanScope {
    #[inline]
    pub(crate) fn new(span: SharedSpan) -> Self {
        Self(Scope::new(&TracingHarness::get().span_scope_stack, span))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SharedSpan {
    // NOTE: we intentionally use a lock without poisoning here to not
    // panic the threads if they just share telemetry with failed thread.
    pub(crate) inner: Arc<parking_lot::RwLock<Span>>,
    // NOTE: store sampling flag separately, so we don't need to acquire lock
    // every time we need to check the flag.
    is_sampled: bool,
}

impl From<Span> for SharedSpan {
    fn from(inner: Span) -> Self {
        let is_sampled = inner.is_sampled();

        Self {
            inner: Arc::new(parking_lot::RwLock::new(inner)),
            is_sampled,
        }
    }
}

pub fn write_current_span(write_fn: impl FnOnce(&mut Span)) {
    if let Some(span) = current_span() {
        if span.is_sampled {
            write_fn(&mut span.inner.write());
        }
    }
}

pub(crate) fn create_span(name: &'static str) -> SharedSpan {
    let harness = TracingHarness::get();

    let span = match current_span() {
        Some(parent) => parent.inner.read().child(name, |o| o.start()),
        None => harness.tracer().span(name).start(),
    };

    span.into()
}

pub(crate) fn current_span() -> Option<SharedSpan> {
    TracingHarness::get().span_scope_stack.current()
}
