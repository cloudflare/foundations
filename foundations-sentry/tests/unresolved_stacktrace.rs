use std::sync::Arc;

use foundations_sentry::backtrace::UnresolvedStacktraceIntegration;
use sentry_core::protocol::{Event, Exception, Frame, Stacktrace, Thread};
use sentry_core::{ClientOptions, Integration};

fn process_event(event: Event<'static>, attach_stacktrace: bool) -> Event<'static> {
    UnresolvedStacktraceIntegration
        .process_event(
            event,
            &ClientOptions {
                attach_stacktrace,
                ..Default::default()
            },
        )
        .unwrap()
}

fn existing_stacktrace() -> Stacktrace {
    Stacktrace {
        frames: vec![Frame {
            function: Some("existing stacktrace".to_owned()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn attaches_instruction_addresses_without_symbols() {
    let event = process_event(Event::default(), true);
    assert_eq!(event.threads.len(), 1);
    let thread = &event.threads[0];
    assert!(thread.current);
    assert!(thread.id.is_some());
    assert_eq!(thread.name.as_deref(), std::thread::current().name());

    let stacktrace = thread.stacktrace.as_ref().unwrap();
    assert!(!stacktrace.frames.is_empty());
    for frame in &stacktrace.frames {
        assert!(frame.instruction_addr.is_some());
        assert_eq!(frame.function.as_deref(), Some("<unknown>"));
        assert!(frame.symbol.is_none());
        assert!(frame.filename.is_none());
        assert!(frame.abs_path.is_none());
        assert!(frame.lineno.is_none());
        assert!(frame.colno.is_none());
    }
}

#[test]
fn respects_attach_stacktrace_option() {
    let event = process_event(Event::default(), false);
    assert!(event.threads.is_empty());
    assert!(event.stacktrace.is_none());
}

#[test]
fn preserves_event_stacktrace() {
    let stacktrace = existing_stacktrace();
    let event = process_event(
        Event {
            stacktrace: Some(stacktrace.clone()),
            ..Default::default()
        },
        true,
    );
    assert_eq!(event.stacktrace, Some(stacktrace));
    assert!(event.threads.is_empty());
}

#[test]
fn preserves_exception_stacktrace() {
    let stacktrace = existing_stacktrace();
    let event = process_event(
        Event {
            exception: vec![
                Exception::default(),
                Exception {
                    stacktrace: Some(stacktrace.clone()),
                    ..Default::default()
                },
            ]
            .into(),
            ..Default::default()
        },
        true,
    );
    assert_eq!(event.exception[1].stacktrace, Some(stacktrace));
    assert!(event.threads.is_empty());
}

#[test]
fn preserves_thread_stacktrace() {
    let stacktrace = existing_stacktrace();
    let event = process_event(
        Event {
            threads: vec![
                Thread::default(),
                Thread {
                    stacktrace: Some(stacktrace.clone()),
                    ..Default::default()
                },
            ]
            .into(),
            ..Default::default()
        },
        true,
    );
    assert_eq!(event.threads.len(), 2);
    assert_eq!(event.threads[1].stacktrace, Some(stacktrace));
}

#[test]
fn attaches_stacktrace_when_existing_exception_and_thread_have_none() {
    let event = process_event(
        Event {
            exception: vec![Exception::default()].into(),
            threads: vec![Thread::default()].into(),
            ..Default::default()
        },
        true,
    );
    assert_eq!(event.exception.len(), 1);
    assert!(event.exception[0].stacktrace.is_none());
    assert_eq!(event.threads.len(), 2);
    assert!(event.threads[0].stacktrace.is_none());
    assert!(event.threads[1].stacktrace.is_some());
}

#[test]
fn preserves_empty_stacktrace() {
    let event = process_event(
        Event {
            stacktrace: Some(Stacktrace::default()),
            ..Default::default()
        },
        true,
    );
    assert!(event.stacktrace.unwrap().frames.is_empty());
    assert!(event.threads.is_empty());
}

#[test]
#[should_panic(expected = "must replace sentry_backtrace::AttachStacktraceIntegration")]
fn rejects_resolving_stacktrace_integration() {
    sentry_core::Client::with_options(ClientOptions {
        default_integrations: false,
        integrations: vec![
            Arc::new(sentry_backtrace::AttachStacktraceIntegration),
            Arc::new(UnresolvedStacktraceIntegration),
        ],
        ..Default::default()
    });
}
