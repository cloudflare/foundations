#![cfg(feature = "sentry")]
//! These tests assume a separate process is used. Make sure you run with `cargo
//! nextest run`.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use foundations::sentry::{metrics, Level};

const TEST_DSN: &str = "https://example@sentry.io/123";

fn simulate_sentry_event() {
    sentry::capture_message("test event", sentry::Level::Error);
}

#[test]
fn sentry_hook_increments_metric_on_event() {
    let mut options = sentry::ClientOptions::default();
    foundations::sentry::install_hook(&mut options);

    let _guard = sentry::init((TEST_DSN, options));

    simulate_sentry_event();
    assert_eq!(metrics::sentry_events::total(Level::Error).get(), 1);
}

#[test]
fn sentry_hook_metrics_are_well_formed() {
    let mut options = sentry::ClientOptions::default();
    foundations::sentry::install_hook(&mut options);

    let _guard = sentry::init((TEST_DSN, options));

    simulate_sentry_event();
    assert_eq!(metrics::sentry_events::total(Level::Error).get(), 1);

    let metrics = foundations::telemetry::metrics::collect(&Default::default()).unwrap();
    let has_metric = metrics
        .lines()
        .any(|line| line == "sentry_events_total{level=\"error\"} 1");
    assert!(has_metric);
}

#[test]
fn sentry_hook_increments_metric_on_multiple_events() {
    let mut options = sentry::ClientOptions::default();
    foundations::sentry::install_hook(&mut options);

    let _guard = sentry::init((TEST_DSN, options));

    simulate_sentry_event();
    simulate_sentry_event();
    simulate_sentry_event();

    assert_eq!(metrics::sentry_events::total(Level::Error).get(), 3);
}

#[test]
fn sentry_hook_preserves_previous_before_send_hook() {
    let previous_hook_count = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&previous_hook_count);

    let mut options = sentry::ClientOptions {
        // Install a custom before_send hook first
        before_send: Some(Arc::new(move |event| {
            counter.fetch_add(1, Ordering::Relaxed);
            Some(event)
        })),
        ..Default::default()
    };

    // Now install foundations hook
    foundations::sentry::install_hook(&mut options);

    let _guard = sentry::init((TEST_DSN, options));

    simulate_sentry_event();
    simulate_sentry_event();

    // Both hooks should have been called
    assert_eq!(previous_hook_count.load(Ordering::Relaxed), 2);
    assert_eq!(metrics::sentry_events::total(Level::Error).get(), 2);
}

#[test]
fn sentry_hook_works_across_threads() {
    let mut options = sentry::ClientOptions::default();
    foundations::sentry::install_hook(&mut options);

    let _guard = sentry::init((TEST_DSN, options));

    // Simulate events from multiple threads
    simulate_sentry_event();

    let handle1 = std::thread::spawn(simulate_sentry_event);
    let handle2 = std::thread::spawn(simulate_sentry_event);

    handle1.join().unwrap();
    handle2.join().unwrap();

    assert_eq!(metrics::sentry_events::total(Level::Error).get(), 3);
}

#[test]
fn sentry_hook_works_in_tokio_tasks() {
    let mut options = sentry::ClientOptions::default();
    foundations::sentry::install_hook(&mut options);

    let _guard = sentry::init((TEST_DSN, options));

    // Event before tokio runtime
    simulate_sentry_event();

    let rt = tokio::runtime::Builder::new_multi_thread().build().unwrap();

    let handle1 = rt.spawn(async {
        simulate_sentry_event();
    });
    let handle2 = rt.spawn(async {
        simulate_sentry_event();
    });

    rt.block_on(async move {
        handle1.await.unwrap();
        handle2.await.unwrap();
    });

    assert_eq!(metrics::sentry_events::total(Level::Error).get(), 3);
}

#[test]
fn cloned_client_options_have_hook_installed() {
    use sentry::{Client, Hub, Scope};

    let mut options = sentry::ClientOptions::default();
    foundations::sentry::install_hook(&mut options);

    // Initialize the global client
    let _guard = sentry::init((TEST_DSN, options));

    // Get the global client and clone its options
    let global_client = Hub::current().client().expect("client should be bound");
    let cloned_options = global_client.options().clone();

    // Create a new client from the cloned options
    let new_client = Arc::new(Client::with_options(cloned_options));
    let hub = Arc::new(Hub::new(Some(new_client), Arc::new(Scope::default())));

    // Run with the new hub and capture an event
    Hub::run(hub, || {
        simulate_sentry_event();
    });

    // The hook should have been called via the cloned options
    assert_eq!(metrics::sentry_events::total(Level::Error).get(), 1);
}
