use super::init::TracingHarness;
use crate::telemetry::scope::Scope;
use rustracing::sampler::ProbabilisticSampler;
use rustracing::tag::Tag;
use rustracing_jaeger::span::{SpanContext, SpanContextState};
use std::borrow::Cow;
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

pub(crate) fn create_span(name: impl Into<Cow<'static, str>>) -> SharedSpan {
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

pub(crate) fn span_trace_id(span: &Span) -> Option<String> {
    span.context().map(|c| c.state().trace_id().to_string())
}

pub(crate) fn force_start_trace(
    root_span_name: impl Into<Cow<'static, str>>,
    stitch_with_trace: Option<SpanContextState>,
) -> Span {
    let tracer = TracingHarness::get().tracer();
    let mut span_builder = tracer.span(root_span_name);

    if let Some(trace) = stitch_with_trace {
        let ctx = SpanContext::new(trace, vec![]);

        span_builder = span_builder.child_of(&ctx);
    }

    span_builder.tag(Tag::new("sampling.priority", 1)).start()
}

pub(crate) fn fork_trace(fork_name: impl Into<Cow<'static, str>>) -> SharedSpan {
    let current_span = match current_span() {
        Some(span) if span.is_sampled => span,
        _ => return Span::inactive().into(),
    };

    let fork_name = fork_name.into();
    let current_span_lock = current_span.inner.read();
    let mut fork_ref_span = create_fork_ref_span(&fork_name, &current_span_lock);
    let fork_root_span = create_fork_root_span(fork_name, current_span_lock, &fork_ref_span);

    // Link the newly created trace in the fork ref span
    if let Some(trace_id) = span_trace_id(&fork_root_span) {
        fork_ref_span.set_tag(|| {
            Tag::new(
                "note",
                "current trace was forked at this point, see the `trace_id` field to obtain the forked trace",
            )
        });

        fork_ref_span.set_tag(|| Tag::new("trace_id", trace_id));
    }

    fork_root_span.into()
}

fn create_fork_ref_span(
    fork_name: &str,
    current_span_lock: &parking_lot::RwLockReadGuard<Span>,
) -> Span {
    let fork_ref_span_name = format!("[{fork_name} ref]");

    current_span_lock.child(fork_ref_span_name, |o| o.start())
}

fn create_fork_root_span(
    fork_name: Cow<'static, str>,
    current_span_lock: parking_lot::RwLockReadGuard<Span>,
    fork_ref_span: &Span,
) -> Span {
    // NOTE: If the current span is sampled, then forked trace is also forcibly sampled
    let mut fork_root_span = force_start_trace(fork_name, None);

    if let Some(trace_id) = span_trace_id(&current_span_lock) {
        fork_root_span.set_tag(|| Tag::new("trace_id", trace_id));
    }

    if let Some(fork_ref_ctx) = fork_ref_span.context() {
        let fork_ref_span_id = format!("{:32x}", fork_ref_ctx.state().span_id());

        fork_root_span.set_tag(|| Tag::new("fork_of_span_id", fork_ref_span_id));
    }

    fork_root_span
}
