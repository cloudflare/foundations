use super::StartTraceOptions;
use super::init::TracingHarness;

use crate::telemetry::tracing::live::LiveReferenceHandle;
use cf_rustracing::sampler::BoxSampler;
#[cfg(feature = "user-tracing")]
use cf_rustracing::span::RoutingMetadata;
use cf_rustracing::tag::Tag;
#[cfg(feature = "user-tracing")]
use cf_rustracing_jaeger::span::TraceId;
use cf_rustracing_jaeger::span::{Span, SpanContext, SpanContextState};
use parking_lot::RwLock;
use rand::RngExt as _;
use std::borrow::Cow;
use std::error::Error;
use std::sync::Arc;
#[cfg(feature = "user-tracing")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub(crate) type Tracer = cf_rustracing::Tracer<BoxSampler<SpanContextState>, SpanContextState>;

/// Shared span with mutability and additional reference tracking for
/// ad-hoc inspection.
///
/// Every handle knows whether its span is sampled without taking the span's lock. Internal spans
/// never change after creation, so the variant implies it, or `Tracked` stores it. User spans can
/// be activated or discarded through any handle, so they share the flag in a [`UserSpanSlot`].
#[derive(Clone, Debug)]
pub(crate) enum SharedSpanHandle {
    /// An internal span registered for liveness tracking. It's only unsampled when all spans are
    /// tracked.
    Tracked {
        span: Arc<LiveReferenceHandle<Arc<RwLock<Span>>>>,
        is_sampled: bool,
    },
    /// A sampled internal span.
    Untracked(Arc<RwLock<Span>>),
    /// A sampled user span.
    #[cfg(feature = "user-tracing")]
    User(Arc<UserSpanSlot>),
    /// A user root that starts inactive and can be activated in place.
    #[cfg(feature = "user-tracing")]
    Deferred(Arc<UserSpanSlot>),
    Inactive,
}

impl SharedSpanHandle {
    pub(crate) fn new(span: Span) -> Self {
        TracingHarness::get().active_roots.track(span)
    }

    #[inline]
    pub(crate) fn is_sampled(&self) -> bool {
        match self {
            SharedSpanHandle::Tracked { is_sampled, .. } => *is_sampled,
            SharedSpanHandle::Untracked(_) => true,
            #[cfg(feature = "user-tracing")]
            SharedSpanHandle::User(slot) | SharedSpanHandle::Deferred(slot) => slot.is_sampled(),
            SharedSpanHandle::Inactive => false,
        }
    }

    pub(crate) fn with_read<R>(&self, f: impl FnOnce(&Span) -> R) -> R {
        static INACTIVE: Span = Span::inactive();

        match self {
            SharedSpanHandle::Tracked { span, .. } => f(&span.read()),
            SharedSpanHandle::Untracked(rw_lock) => f(&rw_lock.read()),
            #[cfg(feature = "user-tracing")]
            SharedSpanHandle::User(slot) | SharedSpanHandle::Deferred(slot) => f(&slot.span.read()),
            SharedSpanHandle::Inactive => f(&INACTIVE),
        }
    }

    /// Runs `f` against the span for mutation, taking a write lock for the duration.
    ///
    /// A no-op for inactive spans: there is nothing to mutate, and unlike [`Self::with_read`] we
    /// can't substitute a shared placeholder.
    pub(crate) fn with_write(&self, f: impl FnOnce(&mut Span)) {
        match self {
            SharedSpanHandle::Tracked { span, .. } => f(&mut span.write()),
            SharedSpanHandle::Untracked(rw_lock) => f(&mut rw_lock.write()),
            #[cfg(feature = "user-tracing")]
            SharedSpanHandle::User(slot) | SharedSpanHandle::Deferred(slot) => {
                f(&mut slot.span.write())
            }
            SharedSpanHandle::Inactive => {}
        }
    }
}

impl From<SharedSpanHandle> for Arc<RwLock<Span>> {
    fn from(value: SharedSpanHandle) -> Self {
        match value {
            SharedSpanHandle::Tracked { span, .. } => Arc::clone(&span),
            SharedSpanHandle::Untracked(rw_lock) => rw_lock,
            // This is only used in `rustracing_span()`, which reads the internal span and should
            // rarely need to be called. Allocating a fresh Arc every time is thus fine.
            #[cfg(feature = "user-tracing")]
            SharedSpanHandle::User(_) | SharedSpanHandle::Deferred(_) => {
                Arc::new(RwLock::new(Span::inactive()))
            }
            SharedSpanHandle::Inactive => Arc::new(RwLock::new(Span::inactive())),
        }
    }
}

/// A user span and whether it's sampled, shared by every handle to it.
///
/// Keeping the flag next to the span, rather than in each handle, keeps it accurate in every handle
/// when the span is activated or discarded.
#[cfg(feature = "user-tracing")]
#[derive(Debug)]
pub(crate) struct UserSpanSlot {
    span: RwLock<Span>,
    /// Mirrors `span.is_sampled()`, so it can be read without the lock. Only written while holding
    /// the write lock.
    is_sampled: AtomicBool,
}

#[cfg(feature = "user-tracing")]
impl UserSpanSlot {
    fn new(span: Span) -> Self {
        Self {
            is_sampled: AtomicBool::new(span.is_sampled()),
            span: RwLock::new(span),
        }
    }

    #[inline]
    fn is_sampled(&self) -> bool {
        self.is_sampled.load(Ordering::Relaxed)
    }

    fn discard(&self) {
        let mut span = self.span.write();
        span.discard();
        self.is_sampled.store(false, Ordering::Relaxed);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SharedSpan {
    // NOTE: we intentionally use a lock without poisoning here to not
    // panic the threads if they just share telemetry with failed thread.
    pub(crate) inner: SharedSpanHandle,
    /// USDT span probe state, recorded when the span's probe semaphore is
    /// non-zero (a tracer is attached), regardless of sampling. Shared by all
    /// clones, so the `span_end__*` probe fires exactly once when the last
    /// clone of the span drops.
    pub(crate) probe: Option<Arc<SpanProbe>>,
}

impl SharedSpan {
    /// Creates a [`SharedSpan`] equivalent to [`Span::inactive()`].
    #[cfg(feature = "user-tracing")]
    pub(crate) const fn inactive() -> Self {
        Self {
            inner: SharedSpanHandle::Inactive,
            probe: None,
        }
    }

    /// Creates a [`SharedSpan`] whose root can be activated in place
    /// after contexts have captured it.
    #[cfg(feature = "user-tracing")]
    pub(crate) fn deferred() -> Self {
        Self {
            inner: SharedSpanHandle::Deferred(Arc::new(UserSpanSlot::new(Span::inactive()))),
            probe: None,
        }
    }

    #[inline]
    pub(crate) fn is_sampled(&self) -> bool {
        self.inner.is_sampled()
    }

    /// Discards a user span, so it is never reported.
    ///
    /// Every handle sharing the span stops recording and reports it as unsampled. A deferred root
    /// that wasn't activated yet has nothing to discard, so it can still be activated. Internal
    /// spans can't be discarded.
    #[cfg(feature = "user-tracing")]
    pub(crate) fn discard(&self) {
        if let SharedSpanHandle::User(slot) | SharedSpanHandle::Deferred(slot) = &self.inner {
            slot.discard();
        }
    }
}

/// Maximum number of u64 arguments a span probe can carry (arg0 is always the
/// span duration in nanoseconds; the rest are caller-chosen values).
pub(crate) const MAX_PROBE_ARGS: usize = 4;

/// Probe state for a single span invocation. Dropping it fires the span's
/// `span_end__*` USDT probe, passing `&args`: the span duration in
/// nanoseconds (arg0) followed by the caller-chosen values. The probe function
/// defines how many of those values are exposed/meaningful.
#[derive(Debug)]
pub(crate) struct SpanProbe {
    start: Instant,
    end_probe: fn(&[u64; MAX_PROBE_ARGS]),
    args: [u64; MAX_PROBE_ARGS],
}

impl SpanProbe {
    /// `args[0]` is overwritten at drop time with the span duration; the
    /// remaining `args[1..4]` are the caller-chosen values.
    pub(crate) fn new(end_probe: fn(&[u64; MAX_PROBE_ARGS]), args: [u64; MAX_PROBE_ARGS]) -> Self {
        Self {
            start: Instant::now(),
            end_probe,
            args,
        }
    }
}

impl Drop for SpanProbe {
    fn drop(&mut self) {
        self.args[0] = self.start.elapsed().as_nanos() as u64;
        (self.end_probe)(&self.args);
    }
}

/// Wraps a span and registers it with the internal harness's `active_roots` for live tracking.
pub(crate) fn shared_span(span: Span) -> SharedSpan {
    SharedSpan {
        inner: SharedSpanHandle::new(span),
        probe: None,
    }
}

/// Wraps a user span as `User`/`Inactive`, bypassing `active_roots` so user spans never
/// enter the internal harness's live registry.
#[cfg(feature = "user-tracing")]
pub(crate) fn user_shared_span(span: Span) -> SharedSpan {
    let inner = if span.is_sampled() {
        SharedSpanHandle::User(Arc::new(UserSpanSlot::new(span)))
    } else {
        SharedSpanHandle::Inactive
    };

    SharedSpan { inner, probe: None }
}

pub fn write_current_span(write_fn: impl FnOnce(&mut Span)) {
    // Check the cached flag before touching the lock. Writing to an unsampled span is a no-op
    // anyway, so this only avoids taking a write guard for nothing.
    let span = match current_span() {
        Some(span) if span.is_sampled() => span,
        _ => return,
    };

    span.inner.with_write(write_fn);
}

pub(crate) fn create_span(name: impl Into<Cow<'static, str>>) -> SharedSpan {
    shared_span(match current_span() {
        Some(parent) => parent.inner.with_read(|s| s.child(name, |o| o.start())),
        None => start_trace(name, Default::default()),
    })
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
        Some(current_span) if current_span.is_sampled() => current_span,
        _ => return span_builder.start(),
    };

    // if a prior trace was ongoing (e.g. during stitching, forking), we want to
    // link the new trace with the existing one
    let mut new_trace_root_span = span_builder.start();

    link_new_trace_with_current(&mut current_span, &root_span_name, &mut new_trace_root_span);

    new_trace_root_span
}

#[cfg(feature = "user-tracing")]
pub(crate) fn current_user_span() -> Option<SharedSpan> {
    TracingHarness::get_user().span_scope_stack.current()
}

/// Child of the current user span, or inactive when no user trace is active (never a root).
#[cfg(feature = "user-tracing")]
pub(crate) fn create_user_span(name: impl Into<Cow<'static, str>>) -> SharedSpan {
    match current_user_span() {
        Some(parent) => child_user_span(&parent, name),
        None => user_shared_span(Span::inactive()),
    }
}

/// Child of an explicitly given user span. Inactive when `parent` is, since an inactive span's
/// children are inactive.
#[cfg(feature = "user-tracing")]
pub(crate) fn child_user_span(
    parent: &SharedSpan,
    name: impl Into<Cow<'static, str>>,
) -> SharedSpan {
    user_shared_span(parent.inner.with_read(|s| s.child(name, |o| o.start())))
}

#[cfg(feature = "user-tracing")]
pub fn write_current_user_span(write_fn: impl FnOnce(&mut Span)) {
    let span = match current_user_span() {
        Some(span) if span.is_sampled() => span,
        _ => return,
    };

    span.inner.with_write(write_fn);
}

/// Starts an inactive deferred root in place. Other spans and already-started roots are unchanged.
#[cfg(feature = "user-tracing")]
pub(crate) fn activate_deferred_user_trace(
    span: &SharedSpan,
    name: impl Into<Cow<'static, str>>,
    routing: impl RoutingMetadata + 'static,
    inbound: Option<super::TraceparentContext>,
) {
    let SharedSpanHandle::Deferred(slot) = &span.inner else {
        return;
    };

    if inbound
        .as_ref()
        .is_some_and(|inbound| !inbound.is_sampled())
    {
        return;
    }

    // Keep caller-controlled conversion and destruction outside the span lock.
    let name = name.into();
    let routing: Arc<dyn RoutingMetadata> = Arc::new(routing);
    let mut span = slot.span.write();
    if span.is_sampled() {
        return;
    }

    *span = start_user_trace(name, routing, inbound);
    slot.is_sampled.store(span.is_sampled(), Ordering::Relaxed);
}

/// Starts a root user span on the user harness, optionally continuing the inbound W3C trace.
/// `routing` is set at construction and inherited by child spans.
#[cfg(feature = "user-tracing")]
pub(crate) fn start_user_trace(
    name: impl Into<Cow<'static, str>>,
    routing: Arc<dyn RoutingMetadata>,
    inbound: Option<super::TraceparentContext>,
) -> Span {
    let tracer = TracingHarness::get_user().tracer();
    let mut builder = tracer.span(name).routing(routing);

    if let Some(tp) = inbound {
        let trace_id = TraceId {
            high: u64::from_be_bytes(tp.trace_id[..8].try_into().unwrap()),
            low: u64::from_be_bytes(tp.trace_id[8..].try_into().unwrap()),
        };
        let state = SpanContextState::new(
            trace_id,
            u64::from_be_bytes(tp.parent_id),
            tp.trace_flags,
            String::new(),
        );
        builder = builder.child_of(&SpanContext::new(state, vec![]));
    }

    builder.start()
}

pub(super) fn reporter_error(err: impl Error) {
    #[cfg(feature = "logging")]
    crate::telemetry::log::error!("failed to report traces to the traces sink"; "error" => %err);

    #[cfg(not(feature = "logging"))]
    drop(err);
}

// Link a newly created trace in the current span's ref span and vice-versa
fn link_new_trace_with_current(
    current_span: &mut SharedSpan,
    root_span_name: &str,
    new_trace_root_span: &mut Span,
) {
    let (mut new_trace_ref_span, current_trace_id) = current_span.inner.with_read(|s| {
        let trace_id = span_trace_id(s);
        let ref_span = create_fork_ref_span(root_span_name, s);
        (ref_span, trace_id)
    });

    if let Some(trace_id) = span_trace_id(&*new_trace_root_span) {
        new_trace_ref_span.set_tag(|| {
            Tag::new(
                "note",
                "current trace was forked at this point, see the `trace_id` field to obtain the forked trace",
            )
        });

        new_trace_ref_span.set_tag(|| Tag::new("trace_id", trace_id));
    }

    if let Some(trace_id) = current_trace_id {
        new_trace_root_span.set_tag(|| Tag::new("trace_id", trace_id));
    }

    if let Some(new_trace_ref_ctx) = new_trace_ref_span.context() {
        let new_trace_ref_span_id = format!("{:32x}", new_trace_ref_ctx.state().span_id());

        new_trace_root_span.set_tag(|| Tag::new("fork_of_span_id", new_trace_ref_span_id));
    }
}

pub(crate) fn fork_trace(fork_name: impl Into<Cow<'static, str>>) -> SharedSpan {
    match current_span() {
        Some(span) if span.is_sampled() => span,
        _ => return shared_span(Span::inactive()),
    };

    let fork_name = fork_name.into();

    shared_span(start_trace(
        fork_name,
        StartTraceOptions {
            // NOTE: If the current span is sampled, then forked trace is also forcibly sampled
            override_sampling_ratio: Some(1.0),
            ..Default::default()
        },
    ))
}

fn create_fork_ref_span(fork_name: &str, current_span: &Span) -> Span {
    let fork_ref_span_name = format!("[{fork_name} ref]");
    current_span.child(fork_ref_span_name, |o| o.start())
}

fn should_sample(sampling_ratio: f64) -> bool {
    // NOTE: quick paths first, without rng involved
    if sampling_ratio == 0.0 {
        return false;
    }

    if sampling_ratio == 1.0 {
        return true;
    }

    rand::rng().random_range(0.0..1.0) < sampling_ratio
}
