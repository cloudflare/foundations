use super::init::{create_tracer_and_span_rx, TracingHarness};
use super::internal::{FinishedSpan, Tracer};
use crate::telemetry::scope::Scope;
use crossbeam_channel::Receiver;
use rustracing::span::SpanReference;
use rustracing::tag::TagValue;
use std::collections::HashMap;
use std::iter::{FusedIterator, Iterator};
use std::sync::Mutex;

type ParentId = Option<u64>;

/// Trace produced in the [test telemetry scope].
///
/// [`test_trace`] macro provides a convenient way to construct this structure in tests for assertions.
///
/// [test telemetry scope]: crate::telemetry::TelemetryContext::test
/// [`test_trace`]: super::test_trace
#[derive(Debug, PartialEq)]
pub struct TestTrace(pub TestSpan);

impl TestTrace {
    /// [`TODO ROCK-13`]
    pub fn iter(&self) -> TestTraceIterator {
        TestTraceIterator {
            stack: vec![&self.0],
        }
    }
}

/// A [depth-first] iterator over [`TestTrace`] spans.
///
/// [depth-first]: https://en.wikipedia.org/wiki/Depth-first_search
pub struct TestTraceIterator<'s> {
    stack: Vec<&'s TestSpan>,
}

impl<'s> Iterator for TestTraceIterator<'s> {
    type Item = &'s TestSpan;

    fn next(&mut self) -> Option<Self::Item> {
        let span = self.stack.pop();

        if let Some(span) = span {
            self.stack.extend(span.children.iter().rev());
        }

        span
    }
}

impl<'s> FusedIterator for TestTraceIterator<'s> {}

/// Trace span produced in the [test telemetry scope].
///
/// [`test_trace`] macro provides a convenient way to construct test spans and traces for assertions.
///
/// [test telemetry scope]: crate::telemetry::TelemetryContext::test
/// [`test_trace`]: super::test_trace
#[derive(Debug, PartialEq)]
pub struct TestSpan {
    /// The name of the span.
    pub name: String,

    /// Children spans.
    pub children: Vec<TestSpan>,

    /// Span's log records.
    pub logs: Vec<(String, String)>,

    /// Span's tags.
    pub tags: Vec<(String, TagValue)>,
}

/// Options for test traces construction.
///
/// Sometimes it's desirable to omit populating certain fields of test spans for the sake of test
/// assertions simplicity. These options provide toggles to disable population of some such span
/// fields.
#[derive(Default, Copy, Clone)]
pub struct TestTraceOptions {
    /// Includes log records in constructed test spans.
    pub include_logs: bool,

    /// Includes tags in constructed test spans.
    pub include_tags: bool,
}

#[must_use]
pub(crate) struct TestTracerScope(Scope<Tracer>);

impl TestTracerScope {
    #[inline]
    pub(crate) fn new(tracer: Tracer) -> Self {
        Self(Scope::new(
            &TracingHarness::get().test_tracer_scope_stack,
            tracer,
        ))
    }
}

pub(crate) struct TestTracesSink {
    span_rx: Receiver<FinishedSpan>,
    raw_spans: Mutex<HashMap<ParentId, Vec<FinishedSpan>>>,
}

impl TestTracesSink {
    pub(crate) fn traces(&self, options: TestTraceOptions) -> Vec<TestTrace> {
        let mut raw_spans = self.raw_spans.lock().unwrap();

        while let Ok(span) = self.span_rx.try_recv() {
            add_raw_span(span, &mut raw_spans);
        }

        for spans in raw_spans.values_mut() {
            spans.sort_by_key(FinishedSpan::start_time);
        }

        match raw_spans.get(&None) {
            Some(roots) => roots
                .iter()
                .map(|root| TestTrace(create_test_span(root, &raw_spans, options)))
                .collect(),
            None => vec![],
        }
    }
}

fn add_raw_span(span: FinishedSpan, raw_spans: &mut HashMap<ParentId, Vec<FinishedSpan>>) {
    let parent_id = span.references().iter().find_map(|r| match r {
        SpanReference::ChildOf(parent) => Some(parent.span_id()),
        _ => None,
    });

    raw_spans.entry(parent_id).or_default().push(span)
}

fn create_test_span(
    raw_span: &FinishedSpan,
    raw_spans: &HashMap<ParentId, Vec<FinishedSpan>>,
    options: TestTraceOptions,
) -> TestSpan {
    let span_id = raw_span.context().state().span_id();

    TestSpan {
        name: raw_span.operation_name().to_string(),
        children: match raw_spans.get(&Some(span_id)) {
            Some(raw_children) => raw_children
                .iter()
                .map(|c| create_test_span(c, raw_spans, options))
                .collect(),
            None => vec![],
        },
        logs: if options.include_logs {
            span_logs(raw_span)
        } else {
            vec![]
        },
        tags: if options.include_tags {
            span_tags(raw_span)
        } else {
            vec![]
        },
    }
}

fn span_logs(raw_span: &FinishedSpan) -> Vec<(String, String)> {
    raw_span
        .logs()
        .iter()
        .flat_map(|l| l.fields().iter())
        .map(|f| (f.name().to_string(), f.value().to_string()))
        .collect()
}

fn span_tags(raw_span: &FinishedSpan) -> Vec<(String, TagValue)> {
    raw_span
        .tags()
        .iter()
        .map(|t| (t.name().to_string(), t.value().clone()))
        .collect()
}

pub(crate) fn current_test_tracer() -> Option<Tracer> {
    TracingHarness::get().test_tracer_scope_stack.current()
}

pub(crate) fn create_test_tracer() -> (Tracer, TestTracesSink) {
    let (tracer, span_rx) = create_tracer_and_span_rx(&Default::default(), true)
        .expect("should create tracer with default settings");

    let sink = TestTracesSink {
        span_rx,
        raw_spans: Default::default(),
    };

    (tracer, sink)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::tracing::test_trace;

    fn make_test_spans(tracer: &Tracer) {
        let root1 = tracer.span("root1").start();
        let root1_child1 = root1.child("root1_child1", |o| o.start());
        let _root1_child1_1 = root1_child1.child("root1_child1_1", |o| o.start());
        let _root1_child1_2 = root1_child1.child("root1_child1_2", |o| o.start());
        let _root1_child2 = root1.child("root1_child2", |o| o.start());
        let root2 = tracer.span("root2").start();
        let _root2_child1 = root2.child("root2_child1", |o| o.start());
    }

    #[test]
    fn span_tree() {
        let (tracer, sink) = create_test_tracer();

        make_test_spans(&tracer);

        assert_eq!(
            sink.traces(Default::default()),
            vec![
                test_trace! {
                    "root1" => {
                        "root1_child1" => {
                            "root1_child1_1",
                            "root1_child1_2"
                        },
                        "root1_child2"
                    }
                },
                test_trace! {
                    "root2" => {
                        "root2_child1"
                    }
                }
            ]
        );
    }

    #[test]
    fn span_iterator() {
        let (tracer, sink) = create_test_tracer();

        make_test_spans(&tracer);

        let root1_spans: Vec<_> = sink.traces(Default::default())[0]
            .iter()
            .map(|s| s.name.clone())
            .collect();

        assert_eq!(
            root1_spans,
            vec![
                "root1",
                "root1_child1",
                "root1_child1_1",
                "root1_child1_2",
                "root1_child2"
            ]
        );
    }
}
