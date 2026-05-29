use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use foundations_sentry::panic::NoFlushPanicIntegration;
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

#[test]
fn no_flush_panic_doesnt_flush() {
    let transport = CountingTransport::new();
    let options = ClientOptions {
        dsn: Some(TEST_DSN.parse().unwrap()),
        transport: Some(Arc::new(Arc::clone(&transport))),
        default_integrations: false,
        integrations: vec![Arc::new(NoFlushPanicIntegration::default())],
        ..Default::default()
    };

    let client = sentry_core::Client::with_options(options);
    let hub = Hub::new(Some(Arc::new(client)), Default::default());

    Hub::run(Arc::new(hub), || {
        let _ = std::panic::catch_unwind(|| panic!("captured panic"));
    });

    let envelopes = transport.fetch_and_clear_envelopes();
    assert_eq!(envelopes.len(), 1);
    let event = envelopes[0]
        .event()
        .expect("Transport should have received exactly 1 event");

    assert_eq!(transport.flushes(), 0);
    assert_eq!(event.level, Level::Fatal);
    assert_eq!(event.exception[0].ty, "panic");
    assert_eq!(event.exception[0].value.as_deref(), Some("captured panic"));
}
