//! Distributed tracing-related functionality.

#[doc(hidden)]
pub mod internal;

pub(crate) mod init;
#[cfg(any(test, feature = "testing"))]
pub(crate) mod testing;

#[cfg(feature = "telemetry-server")]
mod live;
mod output_jaeger_thrift_udp;
mod rate_limit;

#[cfg(feature = "telemetry-otlp-grpc")]
mod output_otlp_grpc;

use self::init::TracingHarness;
use self::internal::{create_span, current_span, span_trace_id, SharedSpan};
use super::scope::Scope;
use super::TelemetryContext;
use cf_rustracing_jaeger::Span;
use std::borrow::Cow;
use std::sync::Arc;

#[cfg(any(test, feature = "testing"))]
pub use self::testing::{TestSpan, TestTrace, TestTraceIterator, TestTraceOptions};

pub use cf_rustracing::tag::TagValue;
pub use cf_rustracing_jaeger::span::{SpanContextState as SerializableTraceState, TraceId};

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
}

impl SpanScope {
    #[inline]
    pub(crate) fn new(span: SharedSpan) -> Self {
        Self {
            span: span.clone(),
            _inner: Scope::new(&TracingHarness::get().span_scope_stack, span),
        }
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

/// Returns a trace ID of the current span.
///
/// Returns `None` if the span is not sampled and don't have associated trace.
pub fn trace_id() -> Option<String> {
    span_trace_id(&current_span()?.inner.read())
}

/// Returns tracing state for the current span that can be serialized and passed to other services
/// to stitch it with their traces, so traces can cover the whole service pipeline.
///
/// The serialized trace then can be passed to [`start_trace`] by other service to continue
/// the trace.
///
/// Returns `None` if the current span is not sampled and don't have associated trace.
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
        .read()
        .context()
        .map(|c| c.state().clone())
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
    SpanScope::new(internal::start_trace(root_span_name, options).into())
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
    __set_span_finish_time as set_span_finish_time, __set_span_start_time as set_span_start_time,
};

#[cfg(feature = "testing")]
#[doc(inline)]
pub use __test_trace as test_trace;
