//! Distributed tracing-related functionality.

#[doc(hidden)]
pub mod internal;

pub mod settings;

#[cfg(any(test, feature = "testing"))]
pub(crate) mod testing;

pub(crate) mod init;

use self::internal::create_span;

#[cfg(any(test, feature = "testing"))]
pub use self::testing::{TestSpan, TestTrace, TestTraceIterator, TestTraceOptions};

pub use self::internal::SpanScope;

/// Creates a tracing span.
///
/// If span covers whole function body it's preferable to use [`tracing::span_fn`] macro.
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
pub fn span(name: &'static str) -> SpanScope {
    SpanScope::new(create_span(name))
}

// NOTE: `#[doc(hidden)]` + `#[doc(inline)]` for `pub use` trick is used to prevent these macros
// to show up in the crate's top level docs.

/// [`TODO ROCK-13`]
#[macro_export]
#[doc(hidden)]
macro_rules! __set_span_tags {
    ( $( $name:expr => $val:expr ),+ ) => {
        $crate::telemetry::tracing::internal::write_current_span(|span| {
            span.set_tags(|| {
                &[ $($crate::reexports_for_macros::rustracing::Tag::new($name, $val)),+ ]
            });
        });
    };

    ( $tags:expr ) => {
        $crate::telemetry::tracing::internal::write_current_span(|span| {
            span.set_tags(|| {
                $tags
                    .into_iter()
                    .map(|(name, val)| {
                        $crate::reexports_for_macros::rustracing::Tag::new(name, val)
                    })
            });
        });
    };
}

/// [`TODO ROCK-13`]
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
        $( logs: [ $( $log:expr ),* ] )?
        $( tags: [ $( $tag_name:expr, $tag_value:expr ),* ] )?
    })? $( => $children:tt )? ) => {
        $crate::telemetry::tracing::TestSpan {
            name: $name.to_string(),
            children: $crate::telemetry::tracing::test_trace!( @children $( $children )? ),
            logs: vec![ $( $( $( $log ),* )? )? ],
            tags: vec![ $( $( $( $tag_value, $tag_value.into() ),* )? )? ]
        }
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
pub use __set_span_tags as set_span_tags;

#[cfg(feature = "testing")]
#[doc(inline)]
pub use __test_trace as test_trace;
