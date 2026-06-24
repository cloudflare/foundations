//! Distributed tracing-related functionality.

#[doc(hidden)]
pub mod internal;

pub(crate) mod init;
#[cfg(any(test, feature = "testing"))]
pub(crate) mod testing;

#[cfg(feature = "metrics")]
pub mod metrics;

mod channel;
mod live;
mod output_jaeger_thrift_udp;
mod rate_limit;

#[cfg(feature = "telemetry-otlp-grpc")]
mod output_otlp_grpc;

#[cfg(feature = "user-tracing")]
mod output_otlp_uds;

use self::init::TracingHarness;
use self::internal::{SharedSpan, create_span, current_span, shared_span, span_trace_id};
#[cfg(feature = "user-tracing")]
use self::internal::{create_user_span, current_user_span, user_shared_span};
use super::TelemetryContext;
use super::scope::Scope;
#[cfg(feature = "user-tracing")]
use cf_rustracing::span::InspectableSpan;
use std::borrow::Cow;
use std::sync::Arc;

#[cfg(any(test, feature = "testing"))]
pub use self::testing::{TestSpan, TestTrace, TestTraceIterator, TestTraceOptions};

pub use cf_rustracing::tag::TagValue;
pub use cf_rustracing_jaeger::span::{Span, SpanContextState as SerializableTraceState, TraceId};

#[cfg(feature = "user-tracing")]
pub use cf_rustracing::span::RoutingMetadata;

/// Returns active traces as a JSON dump.
///
/// The model for this functionality is <https://pkg.go.dev/golang.org/x/net/trace>
/// although there is no built-in viewer here, we expect you to use Chrome's
/// `about:tracing` or anything that supports the same JSON log format.
///
/// The same output is also available through the telemetry server at `/debug/traces`.
pub fn get_active_traces() -> String {
    TracingHarness::get().active_roots.get_active_traces()
}

/// A macro that wraps function body with a tracing span that is active as long as the function
/// call lasts.
///
/// The macro works both for sync and async methods and also for the [async_trait] method
/// implementations.
///
/// # Example
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::tracing::{self, test_trace};
///
/// #[tracing::span_fn("foo")]
/// fn foo() {
///     // Does something...
/// }
///
/// // Test context is used for demonstration purposes to show the resulting traces.
/// let ctx = TelemetryContext::test();
/// let _scope = ctx.scope();
///
/// foo();
///
/// assert_eq!(
///     ctx.traces(Default::default()),
///     vec![
///         test_trace! {
///             "foo"
///         },
///     ]
/// );
/// ```
///
/// # Using constants for span names
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::tracing::{self, test_trace};
///
/// const FOO: &str = "foo";
///
/// #[tracing::span_fn(FOO)]
/// fn foo() {
///     // Does something...
/// }
///
/// // Test context is used for demonstration purposes to show the resulting traces.
/// let ctx = TelemetryContext::test();
/// let _scope = ctx.scope();
///
/// foo();
///
/// assert_eq!(
///     ctx.traces(Default::default()),
///     vec![
///         test_trace! {
///             "foo"
///         },
///     ]
/// );
/// ```
///
/// # Using with `async fn`'s that produce `!Send` futures.
/// ```
/// use foundations::telemetry::tracing;
///
/// #[tracing::span_fn("foo", async_local = true)]
/// async fn foo() {
///     // Does something that produces `!Send`` future...
/// }
/// ```
///
/// # Renamed or reexported crate
///
/// The macro will fail to compile if `foundations` crate is reexported. However, the crate path
/// can be explicitly specified for the macro to workaround that:
///
/// ```
/// mod reexport {
///     pub use foundations::*;
/// }
///
/// use reexport::telemetry::TelemetryContext;
/// use reexport::telemetry::tracing::{self, test_trace};
///
/// #[tracing::span_fn("foo", crate_path = "reexport")]
/// fn foo() {
///     // Does something...
/// }
///
/// // Test context is used for demonstration purposes to show the resulting traces.
/// let ctx = TelemetryContext::test();
/// let _scope = ctx.scope();
///
/// foo();
///
/// assert_eq!(
///     ctx.traces(Default::default()),
///     vec![
///         test_trace! {
///             "foo"
///         },
///     ]
/// );
/// ```
///
/// [async_trait]: https://crates.io/crates/async-trait
pub use foundations_macros::span_fn;

/// A handle for the scope in which tracing span is active.
///
/// Scope ends when the handle is dropped.
#[must_use]
pub struct SpanScope {
    span: SharedSpan,
    _inner: Scope<SharedSpan>,

    #[cfg(feature = "user-tracing")]
    user_span: Option<SharedSpan>,
    #[cfg(feature = "user-tracing")]
    _user_inner: Option<Scope<SharedSpan>>,
}

impl SpanScope {
    #[inline]
    pub(crate) fn new(span: SharedSpan) -> Self {
        Self {
            span: span.clone(),
            _inner: Scope::new(&TracingHarness::get().span_scope_stack, span),

            #[cfg(feature = "user-tracing")]
            user_span: None,
            #[cfg(feature = "user-tracing")]
            _user_inner: None,
        }
    }

    /// Opens a parallel user span (child of the current user span, named after this span) when a
    /// user trace is active; otherwise a no-op. The user span shares this scope's lifetime.
    #[cfg(feature = "user-tracing")]
    pub fn with_user_span(mut self) -> Self {
        if current_user_span().is_some() {
            let name = self
                .span
                .inner
                .with_read(|s| s.operation_name().to_string());
            let user_span = create_user_span(name);

            self._user_inner = Some(Scope::new(
                &TracingHarness::get_user().span_scope_stack,
                user_span.clone(),
            ));
            self.user_span = Some(user_span);
        }

        self
    }

    /// Converts the span scope to [`TelemetryContext`] that can be a applied to a future.
    ///
    /// This is effectively a shorthand for calling [`TelemetryContext::current`] with the span
    /// being in scope.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::TelemetryContext;
    /// use foundations::telemetry::tracing::{self, test_trace};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // Test context is used for demonstration purposes to show the resulting traces.
    ///     let ctx = TelemetryContext::test();
    ///
    ///     {
    ///         let _scope = ctx.scope();
    ///         let _root = tracing::span("root");
    ///
    ///         let handle = tokio::spawn(
    ///             tracing::span("future").into_context().apply(async {
    ///                 let _child = tracing::span("child");
    ///             })
    ///         );
    ///
    ///         handle.await;
    ///     }
    ///
    ///     assert_eq!(
    ///         ctx.traces(Default::default()),
    ///         vec![
    ///             test_trace! {
    ///                 "root" => {
    ///                     "future" => {
    ///                         "child"
    ///                     }
    ///                 }
    ///             }
    ///         ]
    ///     );
    /// }
    /// ```
    pub fn into_context(self) -> TelemetryContext {
        let mut ctx = TelemetryContext::current();

        ctx.span = Some(self.span);

        #[cfg(feature = "user-tracing")]
        if let Some(user_span) = self.user_span {
            ctx.user_span = Some(user_span);
        }

        ctx
    }
}

/// A handle for the scope in which a user-tracing span is active.
///
/// Scope ends when the handle is dropped.
#[cfg(feature = "user-tracing")]
#[must_use]
pub struct UserSpanScope {
    span: SharedSpan,
    _inner: Scope<SharedSpan>,
}

#[cfg(feature = "user-tracing")]
impl UserSpanScope {
    #[inline]
    pub(crate) fn new(span: SharedSpan) -> Self {
        Self {
            span: span.clone(),
            _inner: Scope::new(&TracingHarness::get_user().span_scope_stack, span),
        }
    }

    /// Converts the user span scope to a [`TelemetryContext`] that can be applied to a future.
    pub fn into_context(self) -> TelemetryContext {
        let mut ctx = TelemetryContext::current();
        ctx.user_span = Some(self.span);

        ctx
    }
}

/// Options for a new trace.
#[derive(Default, Debug)]
pub struct StartTraceOptions {
    /// Links the new trace with the existing one whose state is provided in the serialized form.
    ///
    /// Usually used to stitch traces between multiple services. The serialized state can be
    /// obtained by using [`state_for_trace_stitching`] function.
    pub stitch_with_trace: Option<SerializableTraceState>,

    /// Overrides the [sampling ratio] specified on [tracing initializaion].
    ///
    /// Can be used to enforce trace sampling by providing `Some(1.0)` value.
    ///
    /// [sampling ratio]: crate::telemetry::settings::ActiveSamplingSettings::sampling_ratio
    /// [tracing initializaion]: crate::telemetry::init
    pub override_sampling_ratio: Option<f64>,
}

/// Determines whether the current span is sampled or not.
///
/// This is useful to do a cheap check before commencing more expensive work,
/// for example to set up span tags.
pub fn span_is_sampled() -> bool {
    matches!(current_span(), Some(span) if span.is_sampled)
}

/// Returns a trace ID of the current span.
///
/// Returns `None` if the span is not sampled and doesn't have associated trace.
pub fn trace_id() -> Option<String> {
    current_span()?.inner.with_read(span_trace_id)
}

/// Returns tracing state for the current span that can be serialized and passed to other services
/// to stitch it with their traces, so traces can cover the whole service pipeline.
///
/// The serialized trace then can be passed to [`start_trace`] by other service to continue
/// the trace.
///
/// Returns `None` if the current span is not sampled and doesn't have an associated trace.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::tracing::{self, test_trace, SerializableTraceState, StartTraceOptions};
///
/// // Test context is used for demonstration purposes to show the resulting traces.
/// let ctx = TelemetryContext::test();
/// let _scope = ctx.scope();
///
/// fn service1() -> String {
///     let _span = tracing::span("service1_span");
///
///     tracing::state_for_trace_stitching().unwrap().to_string()
/// }
///
/// fn service2(trace_state: String) {
///     let _span = tracing::start_trace(
///         "service2_span",
///         StartTraceOptions {
///             stitch_with_trace: Some(trace_state.parse().unwrap()),
///             ..Default::default()
///         }
///     );
/// }
///
/// let trace_state = service1();
///
/// service2(trace_state);
///
/// assert_eq!(
///     ctx.traces(Default::default()),
///     vec![test_trace! {
///         "service1_span" => {
///             "service2_span"
///         }
///     }]
/// );
/// ```
pub fn state_for_trace_stitching() -> Option<SerializableTraceState> {
    current_span()?
        .inner
        .with_read(|s| Some(s.context()?.state().clone()))
}

/// Returns the value to be used as a W3C traceparent header.
///
/// See: <https://www.w3.org/TR/trace-context/#traceparent-header>
///
/// Returns `None` if the current span is not sampled and doesn't have an associated trace.
pub fn w3c_traceparent() -> Option<String> {
    state_for_trace_stitching().map(|state| {
        format!(
            "00-{:0>16x}{:0>16x}-{:0>16x}-{:0>2x}",
            state.trace_id().high,
            state.trace_id().low,
            state.span_id(),
            state.flags()
        )
    })
}

/// Creates a tracing span.
///
/// If span covers whole function body it's preferable to use [`span_fn`] macro.
///
/// Span ends when returned [`SpanScope`] is dropped. Note that [`SpanScope`] can't be used across
/// `await` points. To span async scopes [`SpanScope::into_context`] should be used.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::tracing::{self, test_trace};
///
/// // Test context is used for demonstration purposes to show the resulting traces.
/// let ctx = TelemetryContext::test();
///
/// {
///     let _scope = ctx.scope();
///     let _root = tracing::span("root");
///
///     {
///         let _span1 = tracing::span("span1");
///     }
///
///     let _span2 = tracing::span("span2");
///     let _span2_1 = tracing::span("span2_1");
/// }
///
/// assert_eq!(
///     ctx.traces(Default::default()),
///     vec![test_trace! {
///         "root" => {
///             "span1",
///             "span2" => {
///                 "span2_1"
///             }
///         }
///     }]
/// );
/// ```
pub fn span(name: impl Into<Cow<'static, str>>) -> SpanScope {
    SpanScope::new(create_span(name))
}

/// Starts a new trace. Ends the current one if it is available and links the new one with it.
///
/// Can also be used to stitch traces with the context received from other services, and can force
/// enable or disable tracing of certain code parts by overriding the sampling ratio.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::tracing::{self, test_trace, StartTraceOptions};
///
/// // Test context is used for demonstration purposes to show the resulting traces.
/// let ctx = TelemetryContext::test();
/// let _scope = ctx.scope();
///
/// {
///     let _root = tracing::span("root");
///
///     {
///         let _span1 = tracing::span("span1");
///     }
///
///     let _new_root_span = tracing::start_trace(
///         "new root",
///         Default::default(),
///     );
///
///     let _span2 = tracing::span("span2");
/// }
///
/// assert_eq!(
///     ctx.traces(Default::default()),
///     vec![
///         test_trace! {
///             "root" => {
///                 "span1",
///                 "[new root ref]"
///             }
///         },
///         test_trace! {
///             "new root" => {
///                 "span2"
///             }
///         }
///     ]
/// );
/// ```
pub fn start_trace(
    root_span_name: impl Into<Cow<'static, str>>,
    options: StartTraceOptions,
) -> SpanScope {
    SpanScope::new(shared_span(internal::start_trace(root_span_name, options)))
}

/// Starts a root user span (per-request activation). `routing` is attached at construction and
/// inherited by child spans.
///
/// Without an active root, `user_span` / `with_user_span` / `add_user_span_tags!` are no-ops.
#[cfg(feature = "user-tracing")]
pub fn start_user_trace(
    name: impl Into<Cow<'static, str>>,
    routing: RoutingMetadata,
) -> UserSpanScope {
    UserSpanScope::new(user_shared_span(internal::start_user_trace(name, routing)))
}

/// Creates a user span as a child of the current user span, or inactive when no user trace is
/// active. Never starts a root — roots come only from [`start_user_trace`].
#[cfg(feature = "user-tracing")]
pub fn user_span(name: impl Into<Cow<'static, str>>) -> UserSpanScope {
    UserSpanScope::new(create_user_span(name))
}

/// Returns the current span as a raw [rustracing] crate's `Span` that is used by Foundations internally.
///
/// Can be used to propagate the tracing context to libraries that don't use Foundations'
/// telemetry.
///
/// [rustracing]: https://crates.io/crates/rustracing
pub fn rustracing_span() -> Option<Arc<parking_lot::RwLock<Span>>> {
    current_span().map(|span| span.inner.into())
}

// NOTE: `#[doc(hidden)]` + `#[doc(inline)]` for `pub use` trick is used to prevent these macros
// to show up in the crate's top level docs.

/// Adds tags to the current tracing span.
///
/// Tags can be either provided in a form of comma-separated `"key" => value` pairs or an
/// [iterable] over `("key", value)` tuples. The later expects that all values have the same
/// type.
///
/// Tag values can be integers, floating point numbers, booleans and strings or string slices.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::tracing::{self, test_trace, TestTraceOptions};
///
/// // Test context is used for demonstration purposes to show the resulting traces.
/// let ctx = TelemetryContext::test();
///
/// {
///     let _scope = ctx.scope();
///     let _root = tracing::span("root");
///
///     tracing::add_span_tags!(
///         "foo" => 42,
///         "bar" => "hello",
///         "baz" => true
///     );
///
///     let _child = tracing::span("child");
///
///     tracing::add_span_tags!(vec![
///         ("qux", 13.37),
///         ("quz", 4.2)
///     ]);
/// }
///
/// let traces = ctx.traces(TestTraceOptions {
///     include_tags: true,
///     ..Default::default()
/// });
///
/// assert_eq!(
///     traces,
///     vec![test_trace! {
///         "root"; {
///             tags: [
///                 ("foo", 42),
///                 ("bar", "hello"),
///                 ("baz", true)
///             ]
///         } => {
///             "child"; {
///                 tags: [
///                     ("qux", 13.37),
///                     ("quz", 4.2)
///                 ]
///             }
///         }
///     }]
/// );
///
/// ```
///
/// [iterable]: std::iter::IntoIterator
#[macro_export]
#[doc(hidden)]
macro_rules! __add_span_tags {
    ( $( $name:expr => $val:expr ),+ ) => {
        $crate::telemetry::tracing::internal::write_current_span(|span| {
            span.set_tags(|| {
                vec![ $($crate::reexports_for_macros::cf_rustracing::tag::Tag::new($name, $val)),+ ]
            });
        });
    };

    ( $tags:expr ) => {
        $crate::telemetry::tracing::internal::write_current_span(|span| {
            span.set_tags(|| {
                $tags
                    .into_iter()
                    .map(|(name, val)| {
                        $crate::reexports_for_macros::cf_rustracing::tag::Tag::new(name, val)
                    })
            });
        });
    };
}

/// Adds log fields to the current span.
///
/// Log entries need to be provided as comma-separated `"field" => "value"` pairs there. Fields and
/// values can be strings or string slices.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::tracing::{self, test_trace, TestTraceOptions};
///
/// // Test context is used for demonstration purposes to show the resulting traces.
/// let ctx = TelemetryContext::test();
///
/// {
///     let _scope = ctx.scope();
///     let _root = tracing::span("root");
///
///     tracing::add_span_log_fields!(
///         "foo" => "hello",
///         "bar" => "world"
///     );
///
///     let _child = tracing::span("child");
///
///     tracing::add_span_log_fields!(
///         "qux" => "beep",
///         "quz" => "boop"
///     );
/// }
///
/// let traces = ctx.traces(TestTraceOptions {
///     include_logs: true,
///     ..Default::default()
/// });
///
/// assert_eq!(
///     traces,
///     vec![test_trace! {
///         "root"; {
///             logs: [
///                 ("foo", "hello"),
///                 ("bar", "world")
///             ]
///         } => {
///             "child"; {
///                 logs: [
///                     ("qux", "beep"),
///                     ("quz", "boop")
///                 ]
///             }
///         }
///     }]
/// );
/// ```
#[macro_export]
#[doc(hidden)]
macro_rules! __add_span_log_fields {
    ( $( $field:expr => $val:expr ),+ ) => {
        $crate::telemetry::tracing::internal::write_current_span(|span| {
            span.log(|builder| {
                $(
                    builder.field(($field, $val));
                )+
            });
        });
    };
}

/// Overrides the start time of the current span with the provided [`SystemTime`] value.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::tracing::{self, test_trace, TestTraceOptions};
/// use std::time::SystemTime;
///
/// // Test context is used for demonstration purposes to show the resulting traces.
/// let ctx = TelemetryContext::test();
/// let start_time = SystemTime::now();
///
/// {
///     let _scope = ctx.scope();
///     let _span = tracing::span("test span");
///
///     tracing::set_span_start_time!(start_time);
/// }
///
/// let traces = ctx.traces(TestTraceOptions {
///     include_start_time: true,
///     ..Default::default()
/// });
///
/// assert_eq!(traces[0].0.start_time, start_time);
/// ```
///
/// [`SystemTime`]: std::time::SystemTime
#[macro_export]
#[doc(hidden)]
macro_rules! __set_span_start_time {
    ( $time:expr ) => {
        $crate::telemetry::tracing::internal::write_current_span(|span| {
            span.set_start_time(|| $time)
        })
    };
}

/// Overrides the finish time of the current span with the provided [`SystemTime`] value.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::tracing::{self, test_trace, TestTraceOptions};
/// use std::time::SystemTime;
///
/// // Test context is used for demonstration purposes to show the resulting traces.
/// let ctx = TelemetryContext::test();
/// let finish_time = SystemTime::now();
///
/// {
///     let _scope = ctx.scope();
///     let _span = tracing::span("test span");
///
///     tracing::set_span_finish_time!(finish_time);
/// }
///
/// let traces = ctx.traces(TestTraceOptions {
///     include_finish_time: true,
///     ..Default::default()
/// });
///
/// assert_eq!(traces[0].0.finish_time, finish_time);
/// ```
///
/// [`SystemTime`]: std::time::SystemTime
#[macro_export]
#[doc(hidden)]
macro_rules! __set_span_finish_time {
    ( $time:expr ) => {
        $crate::telemetry::tracing::internal::write_current_span(|span| {
            span.set_finish_time(|| $time)
        })
    };
}

/// Sets a new finish callback for the current span. It executes when the span is dropped.
///
/// Each span can only have one callback at a time. Children of a span inherit the
/// callback that is set at the time each child is created. To remove a callback, use
/// `set_span_finish_callback!(None)`.
///
/// The callback has signature `Fn(&mut Span)` and can access all functions available on
/// [`Span`].
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::tracing::{self, Span, test_trace, TestTraceOptions};
///
/// // Test context is used for demonstration purposes to show the resulting traces.
/// let ctx = TelemetryContext::test();
///
/// {
///     let _scope = ctx.scope();
///     let _root = tracing::span("root");
///
///     tracing::set_span_finish_callback!(|span: &mut Span| {
///         use cf_rustracing::tag::Tag;
///         span.set_tag(|| Tag::new("user-id", 92395));
///     });
///
///     let child_with_cb = tracing::span("child_with_cb");
///     drop(child_with_cb);
///
///     // Remove the callback from a newly-created child
///     let _child_without_cb = tracing::span("child_without_cb");
///     tracing::set_span_finish_callback!(None);
/// }
///
/// let traces = ctx.traces(TestTraceOptions {
///     include_tags: true,
///     ..Default::default()
/// });
///
/// assert_eq!(
///     traces,
///     vec![test_trace! {
///         "root"; {
///             tags: [ ("user-id", 92395) ]
///         } => {
///             "child_with_cb"; {
///                 tags: [ ("user-id", 92395) ]
///             },
///             "child_without_cb"
///         }
///     }]
/// );
/// ```
#[macro_export]
#[doc(hidden)]
macro_rules! __set_span_finish_callback {
    ( None ) => {
        $crate::telemetry::tracing::internal::write_current_span(|span| {
            span.take_finish_callback();
        })
    };
    ( $cb:expr ) => {{
        let cb = $cb;
        $crate::telemetry::tracing::internal::write_current_span(move |span| {
            span.set_finish_callback(cb);
        })
    }};
}

/// Adds tags to the current user span. No-op when no user trace is active.
#[cfg(feature = "user-tracing")]
#[macro_export]
#[doc(hidden)]
macro_rules! __add_user_span_tags {
    ( $( $name:expr => $val:expr ),+ ) => {
        $crate::telemetry::tracing::internal::write_current_user_span(|span| {
            span.set_tags(|| {
                vec![ $($crate::reexports_for_macros::cf_rustracing::tag::Tag::new($name, $val)),+ ]
            });
        });
    };

    ( $tags:expr ) => {
        $crate::telemetry::tracing::internal::write_current_user_span(|span| {
            span.set_tags(|| {
                $tags
                    .into_iter()
                    .map(|(name, val)| {
                        $crate::reexports_for_macros::cf_rustracing::tag::Tag::new(name, val)
                    })
            });
        });
    };
}

/// Adds log fields to the current user span. No-op when no user trace is active.
#[cfg(feature = "user-tracing")]
#[macro_export]
#[doc(hidden)]
macro_rules! __add_user_span_log_fields {
    ( $( $field:expr => $val:expr ),+ ) => {
        $crate::telemetry::tracing::internal::write_current_user_span(|span| {
            span.log(|builder| {
                $(
                    builder.field(($field, $val));
                )+
            });
        });
    };
}

/// Sets (`$cb`) or clears (`None`) the finish callback on the current user span. No-op when no
/// user trace is active. Routing is set at construction by `start_user_trace`, so this is a
/// general escape hatch — not used for routing.
#[cfg(feature = "user-tracing")]
#[macro_export]
#[doc(hidden)]
macro_rules! __set_user_span_finish_callback {
    ( None ) => {
        $crate::telemetry::tracing::internal::write_current_user_span(|span| {
            span.take_finish_callback();
        })
    };
    ( $cb:expr ) => {{
        let cb = $cb;
        $crate::telemetry::tracing::internal::write_current_user_span(move |span| {
            span.set_finish_callback(cb);
        })
    }};
}

/// A convenience macro to construct [`TestTrace`] for test assertions.
///
/// Note that for span timings the macro always generates default
/// [`std::time::SystemTime::UNIX_EPOCH`] values (as with [`TestTraceOptions::include_start_time`]
/// and [`TestTraceOptions::include_start_time`] being set to `false`).
///
/// # Examples
/// ```
/// use foundations::telemetry::tracing::{test_trace, TestSpan, TestTrace};
/// use std::time::SystemTime;
///
/// let trace = test_trace! {
///     "root" => {
///         "child1" => {
///             "child1_1",
///             "child1_2"
///         },
///         "child2"
///     }
/// };
///
/// let expanded = TestTrace(TestSpan {
///     name: "root".into(),
///     logs: vec![],
///     tags: vec![],
///     start_time: SystemTime::UNIX_EPOCH,
///     finish_time: SystemTime::UNIX_EPOCH,
///     children: vec![
///         TestSpan {
///             name: "child1".into(),
///             logs: vec![],
///             tags: vec![],
///             start_time: SystemTime::UNIX_EPOCH,
///             finish_time: SystemTime::UNIX_EPOCH,
///             children: vec![
///                 TestSpan {
///                     name: "child1_1".into(),
///                     logs: vec![],
///                     tags: vec![],
///                     start_time: SystemTime::UNIX_EPOCH,
///                     finish_time: SystemTime::UNIX_EPOCH,
///                     children: vec![],
///                 },
///                 TestSpan {
///                     name: "child1_2".into(),
///                     logs: vec![],
///                     tags: vec![],
///                     start_time: SystemTime::UNIX_EPOCH,
///                     finish_time: SystemTime::UNIX_EPOCH,
///                     children: vec![],
///                 },
///             ],
///         },
///         TestSpan {
///             name: "child2".into(),
///             logs: vec![],
///             tags: vec![],
///             start_time: SystemTime::UNIX_EPOCH,
///             finish_time: SystemTime::UNIX_EPOCH,
///             children: vec![],
///         },
///     ],
/// });
///
/// assert_eq!(trace, expanded);
/// ```
///
/// Tags and log records can optionally be included in the generated [`TestSpan`] as a list of
/// `("key", value)` pairs.
///
/// Note that span's log records are always lexicographically sorted by the field name, so macro
/// sorts the provided log records this way during expansion.
///
/// ```
/// use foundations::telemetry::tracing::{test_trace, TagValue, TestSpan, TestTrace};
/// use std::time::SystemTime;
///
/// let trace = test_trace! {
///     "root"; {
///         logs: [
///             ("hello", "world"),
///             ("foo", "bar")
///         ]
///     } => {
///         "child1"; {
///             tags: [
///                 ("tag1", 42),
///                 ("tag2", "hi")
///             ]
///         },
///         "child2"; {
///             logs: [
///                 ("answer", "42")
///             ]
///
///             tags: [
///                 ("more_tags", true)
///             ]
///         }
///     }
/// };
///
/// let expanded = TestTrace(TestSpan {
///     name: "root".into(),
///     // NOTE: log records are lexicographically sorted by the field name.
///     logs: vec![
///         ("foo".into(), "bar".into()),
///         ("hello".into(), "world".into()),
///     ],
///     tags: vec![],
///     start_time: SystemTime::UNIX_EPOCH,
///     finish_time: SystemTime::UNIX_EPOCH,
///     children: vec![
///         TestSpan {
///             name: "child1".into(),
///             logs: vec![],
///             tags: vec![
///                 ("tag1".into(), TagValue::Integer(42)),
///                 ("tag2".into(), TagValue::String("hi".into())),
///             ],
///             start_time: SystemTime::UNIX_EPOCH,
///             finish_time: SystemTime::UNIX_EPOCH,
///             children: vec![],
///         },
///         TestSpan {
///             name: "child2".into(),
///             logs: vec![("answer".into(), "42".into())],
///             tags: vec![("more_tags".into(), TagValue::Boolean(true))],
///             start_time: SystemTime::UNIX_EPOCH,
///             finish_time: SystemTime::UNIX_EPOCH,
///             children: vec![],
///         },
///     ],
/// });
///
/// assert_eq!(trace, expanded);
/// ```
#[macro_export]
#[doc(hidden)]
#[cfg(feature = "testing")]
macro_rules! __test_trace {
    ( $name:expr $( ; $logs_tags:tt )? $( => $children:tt )? ) => {
        $crate::telemetry::tracing::TestTrace(
            $crate::telemetry::tracing::test_trace!(
                @span $name $(; $logs_tags)? $( => $children )?
            )
        )
    };

    ( @span $name:expr $( ; {
        $( logs: [ $( ( $log_field:expr, $log_value:expr ) ),* ] )?
        $( tags: [ $( ( $tag_name:expr, $tag_value:expr ) ),* ] )?
    })? $( => $children:tt )? ) => {{
        // NOTE: resulting logs are lexicographically sorted, so we sort provided fields for
        // conveience, so macro users won't need to bother.
        let mut logs = vec![ $( $( $( ( $log_field.into(), $log_value.into() ) ),* )? )? ];

        logs.sort_by(|(f1, _), (f2, _)| std::cmp::Ord::cmp(f1, f2));

        $crate::telemetry::tracing::TestSpan {
            name: $name.to_string(),
            children: $crate::telemetry::tracing::test_trace!( @children $( $children )? ),
            logs,
            tags: vec![ $( $( $( ( $tag_name.into(), $tag_value.into() ) ),* )? )? ],
            start_time: std::time::SystemTime::UNIX_EPOCH,
            finish_time: std::time::SystemTime::UNIX_EPOCH,
        }}
    };

    ( @children { $( $name:expr $( ; $logs_tags:tt )? $( => $children:tt )? ),* } ) => {
        vec![
            $(
                $crate::telemetry::tracing::test_trace!(
                    @span $name $(; $logs_tags)? $( => $children )?
                )
            ),*
        ]
    };

    ( @children ) => { vec![] };
}

#[doc(inline)]
pub use {
    __add_span_log_fields as add_span_log_fields, __add_span_tags as add_span_tags,
    __set_span_finish_callback as set_span_finish_callback,
    __set_span_finish_time as set_span_finish_time, __set_span_start_time as set_span_start_time,
};

#[cfg(feature = "user-tracing")]
#[doc(inline)]
pub use {
    __add_user_span_log_fields as add_user_span_log_fields,
    __add_user_span_tags as add_user_span_tags,
    __set_user_span_finish_callback as set_user_span_finish_callback,
};

#[cfg(feature = "testing")]
#[doc(inline)]
pub use __test_trace as test_trace;

#[cfg(all(test, feature = "user-tracing", feature = "testing"))]
mod user_tracing_tests {
    use super::{
        RoutingMetadata, add_user_span_log_fields, add_user_span_tags,
        set_user_span_finish_callback, span, start_user_trace, test_trace, user_span,
    };
    use crate::telemetry::TelemetryContext;
    use crate::telemetry::tracing::{Span, TestTraceOptions};
    use cf_rustracing::tag::{Tag, TagValue};

    fn routing() -> RoutingMetadata {
        RoutingMetadata {
            zone_id: 1,
            account_id: 2,
            account_tag: "0123456789abcdef0123456789abcdef".to_string(),
            destinations: vec![],
            persist: false,
        }
    }

    #[test]
    fn creation_and_nesting() {
        let ctx = TelemetryContext::test();
        let _scope = ctx.scope();

        {
            let _root = start_user_trace("request", routing());
            let _child = user_span("child");
            let _grandchild = user_span("grandchild");
        }

        assert_eq!(
            ctx.user_traces(Default::default()),
            vec![test_trace! {
                "request" => {
                    "child" => {
                        "grandchild"
                    }
                }
            }]
        );
        // User spans must not leak into the internal pipeline.
        assert!(ctx.traces(Default::default()).is_empty());
    }

    #[test]
    fn tags_and_logs() {
        let ctx = TelemetryContext::test();
        let _scope = ctx.scope();

        {
            let _root = start_user_trace("request", routing());
            add_user_span_tags!("cache.status" => "HIT");
            add_user_span_log_fields!("event" => "lookup");
        }

        let opts = TestTraceOptions {
            include_tags: true,
            include_logs: true,
            ..Default::default()
        };
        let traces = ctx.user_traces(opts);
        let root = &traces[0].0;

        assert!(
            root.tags
                .contains(&("cache.status".to_string(), TagValue::String("HIT".into())))
        );
        assert!(
            root.logs
                .contains(&("event".to_string(), "lookup".to_string()))
        );
    }

    #[test]
    fn with_user_span_is_parallel() {
        let ctx = TelemetryContext::test();
        let _scope = ctx.scope();

        {
            let _root = start_user_trace("request", routing());
            let _s = span("op").with_user_span();
        }

        // Internal pipeline: just the internal span.
        assert_eq!(ctx.traces(Default::default()), vec![test_trace! { "op" }]);
        // User pipeline: the parallel user span nested under the user root.
        assert_eq!(
            ctx.user_traces(Default::default()),
            vec![test_trace! { "request" => { "op" } }]
        );
    }

    #[test]
    fn no_op_without_activation() {
        let ctx = TelemetryContext::test();
        let _scope = ctx.scope();

        {
            // No `start_user_trace`, so user tracing isn't active for this scope.
            let _child = user_span("child");
            add_user_span_tags!("k" => "v");
        }

        assert!(ctx.user_traces(Default::default()).is_empty());
    }

    #[test]
    fn finish_callback_runs() {
        let ctx = TelemetryContext::test();
        let _scope = ctx.scope();

        {
            let _root = start_user_trace("request", routing());
            set_user_span_finish_callback!(|span: &mut Span| {
                span.set_tag(|| Tag::new("finished", true));
            });
        }

        let opts = TestTraceOptions {
            include_tags: true,
            ..Default::default()
        };
        let traces = ctx.user_traces(opts);
        assert!(traces[0].0.tags.iter().any(|(k, _)| k == "finished"));
    }

    #[tokio::test]
    async fn propagates_across_await() {
        let ctx = TelemetryContext::test();
        let _scope = ctx.scope();

        {
            let root_ctx = start_user_trace("request", routing()).into_context();
            root_ctx
                .apply(async {
                    let _child = user_span("child");
                })
                .await;
        }

        assert_eq!(
            ctx.user_traces(Default::default()),
            vec![test_trace! { "request" => { "child" } }]
        );
    }

    // The user span rides along on the ambient `TelemetryContext` even when propagation goes
    // through an *internal* span's `into_context()` — no explicit user-span threading needed.
    #[tokio::test]
    async fn user_span_carried_by_internal_context() {
        let ctx = TelemetryContext::test();
        let _scope = ctx.scope();

        {
            let _root = start_user_trace("request", routing());

            // Propagate via an internal span's context; never touch the user scope.
            span("internal")
                .into_context()
                .apply(async {
                    let _user_child = user_span("user_child");
                })
                .await;
        }

        // User pipeline: the user child nested under the user root (the user span was carried).
        assert_eq!(
            ctx.user_traces(Default::default()),
            vec![test_trace! { "request" => { "user_child" } }]
        );
        // Internal pipeline: just the internal span.
        assert_eq!(
            ctx.traces(Default::default()),
            vec![test_trace! { "internal" }]
        );
    }

    // Same property via the `#[span_fn]` macro path (a plain internal-traced async fn).
    #[crate::telemetry::tracing::span_fn("internal_fn", crate_path = "crate")]
    async fn internal_fn() {
        let _user_child = user_span("user_child");
    }

    #[tokio::test]
    async fn user_span_carried_by_span_fn() {
        let ctx = TelemetryContext::test();
        let _scope = ctx.scope();

        {
            let _root = start_user_trace("request", routing());
            internal_fn().await;
        }

        assert_eq!(
            ctx.user_traces(Default::default()),
            vec![test_trace! { "request" => { "user_child" } }]
        );
        assert_eq!(
            ctx.traces(Default::default()),
            vec![test_trace! { "internal_fn" }]
        );
    }

    // `with_user_span()` is a no-op when no user trace is active: the internal span is still
    // created, but no parallel user span is produced.
    #[test]
    fn with_user_span_no_op_when_inactive() {
        let ctx = TelemetryContext::test();
        let _scope = ctx.scope();

        {
            // No `start_user_trace` => user tracing not active for this scope.
            let _s = span("op").with_user_span();
        }

        assert_eq!(ctx.traces(Default::default()), vec![test_trace! { "op" }]);
        assert!(ctx.user_traces(Default::default()).is_empty());
    }

    #[crate::telemetry::tracing::span_fn("user_fn", user = true, crate_path = "crate")]
    async fn user_fn() {}

    // `#[span_fn(user = true)]` is likewise a no-op for the user pipeline when inactive.
    #[tokio::test]
    async fn span_fn_user_no_op_when_inactive() {
        let ctx = TelemetryContext::test();
        let _scope = ctx.scope();

        user_fn().await;

        assert_eq!(
            ctx.traces(Default::default()),
            vec![test_trace! { "user_fn" }]
        );
        assert!(ctx.user_traces(Default::default()).is_empty());
    }
}
