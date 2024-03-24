use super::init::TracingHarness;
use super::StartTraceOptions;
use rand::{self, Rng};

use crate::telemetry::tracing::rate_limit::RateLimitingProbabilisticSampler;
use rustracing::tag::Tag;
use rustracing_jaeger::span::{SpanContext, SpanContextState};
use std::borrow::Cow;
use std::sync::Arc;

pub(crate) type Span = rustracing::span::Span<SpanContextState>;
pub(crate) type FinishedSpan = rustracing::span::FinishedSpan<SpanContextState>;
pub(crate) type Tracer = rustracing::Tracer<RateLimitingProbabilisticSampler, SpanContextState>;

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
    match current_span() {
        Some(parent) => parent.inner.read().child(name, |o| o.start()),
        None => start_trace(name, Default::default()),
    }
    .into()
}

pub(crate) fn current_span() -> Option<SharedSpan> {
    TracingHarness::get().span_scope_stack.current()
}

pub(crate) fn span_trace_id(span: &Span) -> Option<String> {
    span.context().map(|c| c.state().trace_id().to_string())
}

pub(crate) fn start_trace(
    root_span_name: impl Into<Cow<'static, str>>,
    options: StartTraceOptions,
) -> Span {
    let tracer = TracingHarness::get().tracer();
    let root_span_name = root_span_name.into();
    let mut span_builder = tracer.span(root_span_name.clone());

    if let Some(state) = options.stitch_with_trace {
        let ctx = SpanContext::new(state, vec![]);

        span_builder = span_builder.child_of(&ctx);
    }

    if let Some(ratio) = options.override_sampling_ratio {
        span_builder = span_builder.tag(Tag::new(
            "sampling.priority",
            if should_sample(ratio) { 1 } else { 0 },
        ));
    }

    let mut current_span = match current_span() {
        Some(current_span) if current_span.is_sampled => current_span,
        _ => return span_builder.start(),
    };

    // if a prior trace was ongoing (e.g. during stitching, forking), we want to
    // link the new trace with the existing one
    let mut new_trace_root_span = span_builder.start();

    link_new_trace_with_current(&mut current_span, &root_span_name, &mut new_trace_root_span);

    new_trace_root_span
}

// Link a newly created trace in the current span's ref span and vice-versa
fn link_new_trace_with_current(
    current_span: &mut SharedSpan,
    root_span_name: &str,
    new_trace_root_span: &mut Span,
) {
    let current_span_lock = current_span.inner.read();
    let mut new_trace_ref_span = create_fork_ref_span(root_span_name, &current_span_lock);

    if let Some(trace_id) = span_trace_id(&*new_trace_root_span) {
        new_trace_ref_span.set_tag(|| {
            Tag::new(
                "note",
                "current trace was forked at this point, see the `trace_id` field to obtain the forked trace",
            )
        });

        new_trace_ref_span.set_tag(|| Tag::new("trace_id", trace_id));
    }

    if let Some(trace_id) = span_trace_id(&current_span_lock) {
        new_trace_root_span.set_tag(|| Tag::new("trace_id", trace_id));
    }

    if let Some(new_trace_ref_ctx) = new_trace_ref_span.context() {
        let new_trace_ref_span_id = format!("{:32x}", new_trace_ref_ctx.state().span_id());

        new_trace_root_span.set_tag(|| Tag::new("fork_of_span_id", new_trace_ref_span_id));
    }
}

pub(crate) fn fork_trace(fork_name: impl Into<Cow<'static, str>>) -> SharedSpan {
    match current_span() {
        Some(span) if span.is_sampled => span,
        _ => return Span::inactive().into(),
    };

    let fork_name = fork_name.into();

    start_trace(
        fork_name,
        StartTraceOptions {
            // NOTE: If the current span is sampled, then forked trace is also forcibly sampled
            override_sampling_ratio: Some(1.0),
            ..Default::default()
        },
    )
    .into()
}

fn create_fork_ref_span(
    fork_name: &str,
    current_span_lock: &parking_lot::RwLockReadGuard<Span>,
) -> Span {
    let fork_ref_span_name = format!("[{fork_name} ref]");

    current_span_lock.child(fork_ref_span_name, |o| o.start())
}

fn should_sample(sampling_ratio: f64) -> bool {
    // NOTE: quick paths first, without rng involved
    if sampling_ratio == 0.0 {
        return false;
    }

    if sampling_ratio == 1.0 {
        return true;
    }

    rand::thread_rng().gen_range(0.0..1.0) < sampling_ratio
}
