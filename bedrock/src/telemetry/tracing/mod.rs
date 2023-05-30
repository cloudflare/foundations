//! Distributed tracing-related functionality.

#[doc(hidden)]
pub mod internal;

pub mod settings;

#[cfg(any(test, feature = "testing"))]
pub(crate) mod testing;

pub(crate) mod init;

use self::internal::{create_span, current_span, span_trace_id};
use std::borrow::Cow;

#[cfg(any(test, feature = "testing"))]
pub use self::testing::{TestSpan, TestTrace, TestTraceIterator, TestTraceOptions};

pub use self::internal::SpanScope;
pub use bedrock_macros::span_fn;
pub use rustracing_jaeger::span::SpanContextState as SerializableTraceState;

/// [TODO] ROCK-13
#[derive(Default, Debug)]
pub struct StartTraceOptions {
    /// [TODO] ROCK-13
    pub stitch_with_trace: Option<SerializableTraceState>,

    /// [TODO] ROCK-13
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
/// `await` points. To span async scopes [`TelemetryContext::apply_with_tracing_span`] should be
/// used.
///
/// [`TelemetryContext::apply_with_tracing_span`]: super::TelemetryContext::apply_with_tracing_span
///
/// # Examples
/// ```
/// use bedrock::telemetry::TelemetryContext;
/// use bedrock::telemetry::tracing::{self, test_trace};
///
/// // Test scope is used for demonstration purposes to show the resulting traces.
/// let scope = TelemetryContext::test();
///
/// {
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
///     scope.traces(Default::default()),
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

/// Starts a new root trace. Ends the current one if there is one.
///
/// Can also be used to stitch traces with the context received from
/// other services, and can force enable or disable tracing of
/// certain code parts by overriding the sampling ratio.
///
/// # Examples
/// ```
/// use bedrock::telemetry::TelemetryContext;
/// use bedrock::telemetry::tracing::{self, test_trace, StartTraceOptions, TestTraceOptions};
///
/// let scope = TelemetryContext::test();
///
/// {
///     let _root = tracing::span("root");
///
///     {
///         let _span1 = tracing::span("span1");
///     }
///
///     let _scope = tracing::start_trace(
///         "new root",
///         Default::default(),
///     );
///
///     {
///         let _span2 = tracing::span("span2");
///     }
/// }
///
/// assert_eq!(
///     scope.traces(Default::default()),
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
pub fn start_trace(
    root_span_name: impl Into<Cow<'static, str>>,
    options: StartTraceOptions,
) -> SpanScope {
    SpanScope::new(internal::start_trace(root_span_name, options).into())
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
/// use bedrock::telemetry::TelemetryContext;
/// use bedrock::telemetry::tracing::{self, test_trace, TestTraceOptions};
///
/// // Test scope is used for demonstration purposes to show the resulting traces.
/// let scope = TelemetryContext::test();
///
/// {
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
/// let traces = scope.traces(TestTraceOptions {
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
                vec![ $($crate::reexports_for_macros::rustracing::tag::Tag::new($name, $val)),+ ]
            });
        });
    };

    ( $tags:expr ) => {
        $crate::telemetry::tracing::internal::write_current_span(|span| {
            span.set_tags(|| {
                $tags
                    .into_iter()
                    .map(|(name, val)| {
                        $crate::reexports_for_macros::rustracing::tag::Tag::new(name, val)
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
/// use bedrock::telemetry::TelemetryContext;
/// use bedrock::telemetry::tracing::{self, test_trace, TestTraceOptions};
///
/// // Test scope is used for demonstration purposes to show the resulting traces.
/// let scope = TelemetryContext::test();
///
/// {
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
/// let traces = scope.traces(TestTraceOptions {
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
/// use bedrock::telemetry::TelemetryContext;
/// use bedrock::telemetry::tracing::{self, test_trace, TestTraceOptions};
/// use std::time::SystemTime;
///
/// // Test scope is used for demonstration purposes to show the resulting traces.
/// let scope = TelemetryContext::test();
/// let start_time = SystemTime::now();
///
/// {
///     let _span = tracing::span("test span");
///
///     tracing::set_span_start_time!(start_time);
/// }
///
/// let traces = scope.traces(TestTraceOptions {
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
/// use bedrock::telemetry::TelemetryContext;
/// use bedrock::telemetry::tracing::{self, test_trace, TestTraceOptions};
/// use std::time::SystemTime;
///
/// // Test scope is used for demonstration purposes to show the resulting traces.
/// let scope = TelemetryContext::test();
/// let finish_time = SystemTime::now();
///
/// {
///     let _span = tracing::span("test span");
///
///     tracing::set_span_finish_time!(finish_time);
/// }
///
/// let traces = scope.traces(TestTraceOptions {
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
/// use bedrock::telemetry::tracing::{test_trace, TestSpan, TestTrace};
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
/// use bedrock::telemetry::tracing::{test_trace, TestSpan, TestTrace};
/// use rustracing::tag::TagValue;
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
