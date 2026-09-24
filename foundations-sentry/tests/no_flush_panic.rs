use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use foundations_sentry::backtrace::UnresolvedStacktraceIntegration;
use foundations_sentry::panic::NoFlushPanicIntegration;
use sentry_core::protocol::Event;
use sentry_core::{ClientOptions, Envelope, Hub, Level, Transport};

const TEST_DSN: &str = "https://example@sentry.io/123";

struct CountingTransport {
    envelopes: Mutex<Vec<Envelope>>,
    flushes: AtomicU64,
}

impl CountingTransport {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            envelopes: Default::default(),
            flushes: Default::default(),
        })
    }

    fn fetch_and_clear_envelopes(&self) -> Vec<Envelope> {
        let mut guard = self.envelopes.lock().unwrap();
        std::mem::take(&mut *guard)
    }

    fn flushes(&self) -> u64 {
        self.flushes.load(Ordering::Relaxed)
    }
}

impl Transport for CountingTransport {
    fn send_envelope(&self, envelope: Envelope) {
        self.envelopes.lock().unwrap().push(envelope);
    }

    fn flush(&self, _timeout: Duration) -> bool {
        self.flushes.fetch_add(1, Ordering::Relaxed);
        true
    }
}

fn capture_panic(
    integration: NoFlushPanicIntegration,
    panic: impl FnOnce() + std::panic::UnwindSafe,
) -> Event<'static> {
    let transport = CountingTransport::new();
    let options = ClientOptions {
        dsn: Some(TEST_DSN.parse().unwrap()),
        transport: Some(Arc::new(Arc::clone(&transport))),
        attach_stacktrace: true,
        default_integrations: false,
        integrations: vec![
            Arc::new(UnresolvedStacktraceIntegration),
            Arc::new(integration),
        ],
        ..Default::default()
    };

    let client = sentry_core::Client::with_options(options);
    let hub = Hub::new(Some(Arc::new(client)), Default::default());

    Hub::run(Arc::new(hub), || {
        assert!(std::panic::catch_unwind(panic).is_err());
    });

    let envelopes = transport.fetch_and_clear_envelopes();
    assert_eq!(envelopes.len(), 1);
    let event = envelopes[0]
        .event()
        .expect("Transport should have received exactly 1 event");

    assert_eq!(transport.flushes(), 0);
    assert_eq!(event.level, Level::Fatal);
    assert_eq!(event.exception.len(), 1);
    assert_eq!(event.exception[0].ty, "panic");
    let mechanism = event.exception[0].mechanism.as_ref().unwrap();
    assert_eq!(mechanism.ty, "panic");
    assert_eq!(mechanism.handled, Some(false));
    assert!(event.threads.is_empty());
    event.clone()
}

fn assert_unresolved(event: &Event<'_>) {
    let stacktrace = event.exception[0].stacktrace.as_ref().unwrap();
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
fn no_flush_panic_doesnt_flush() {
    let event = capture_panic(NoFlushPanicIntegration::default(), || {
        panic!("captured panic")
    });
    assert_eq!(event.exception[0].value.as_deref(), Some("captured panic"));
    let stacktrace = event.exception[0].stacktrace.as_ref().unwrap();
    assert!(stacktrace.frames.iter().any(|frame| {
        frame
            .function
            .as_deref()
            .is_some_and(|function| function != "<unknown>")
    }));
}

#[test]
fn unresolved_panic_doesnt_resolve_or_flush() {
    let event = capture_panic(
        NoFlushPanicIntegration::new().with_unresolved_stacktraces(),
        || panic!("captured panic"),
    );
    assert_eq!(event.exception[0].value.as_deref(), Some("captured panic"));
    assert_unresolved(&event);
}

#[test]
fn unresolved_panic_preserves_string_payload() {
    let event = capture_panic(
        NoFlushPanicIntegration::new().with_unresolved_stacktraces(),
        || std::panic::panic_any(String::from("owned panic")),
    );
    assert_eq!(event.exception[0].value.as_deref(), Some("owned panic"));
    assert_unresolved(&event);
}

#[test]
fn unresolved_panic_preserves_unknown_payload() {
    let event = capture_panic(
        NoFlushPanicIntegration::new().with_unresolved_stacktraces(),
        || std::panic::panic_any(42_u32),
    );
    assert_eq!(event.exception[0].value.as_deref(), Some("Box<Any>"));
    assert_unresolved(&event);
}
