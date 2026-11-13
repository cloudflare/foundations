//! These tests assume a separate process is used. Make sure you run with `cargo
//! nextest run`.

use sentry_core::{ClientOptions, Hub, Level};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use foundations_sentry::{EventRateLimitSettings, SentrySettings, metrics};

const TEST_DSN: &str = "https://example@sentry.io/123";

fn simulate_sentry_event(hub: &Hub) {
    hub.capture_message("test event", Level::Error);
}

fn hub_with_settings(settings: &SentrySettings) -> Hub {
    let mut options = ClientOptions::default();
    foundations_sentry::install_hook_with_settings(&mut options, settings);
    hub_with_options(options)
}

fn hub_with_options(mut options: ClientOptions) -> Hub {
    options.dsn = Some(TEST_DSN.parse().unwrap());
    options.transport = Some(Arc::new(sentry_core::test::TestTransport::new()));

    let client = sentry_core::Client::with_options(options);
    Hub::new(Some(Arc::new(client)), Default::default())
}

#[test]
fn sentry_hook_increments_metric_on_event() {
    let hub = hub_with_settings(&Default::default());
    simulate_sentry_event(&hub);
    assert_eq!(metrics::sentry::events_total(Level::Error).get(), 1);

    const SERIES: &str = "sentry_events_total{level=\"error\"} ";

    #[allow(deprecated)]
    let metrics = foundations::telemetry::metrics::collect(&Default::default()).unwrap();
    let has_metric = metrics.lines().any(|line| {
        line.strip_prefix(SERIES)
            .and_then(|value| value.parse::<f64>().ok())
            == Some(1.0)
    });
    assert!(has_metric);
}

#[test]
fn sentry_hook_increments_metric_on_multiple_events() {
    let hub = hub_with_settings(&Default::default());

    simulate_sentry_event(&hub);
    simulate_sentry_event(&hub);
    simulate_sentry_event(&hub);

    assert_eq!(metrics::sentry::events_total(Level::Error).get(), 3);
}

#[test]
fn sentry_hook_rate_limits_events() {
    let settings = SentrySettings {
        max_events_per_second: Some(NonZeroU32::new(1).unwrap()),
        ..Default::default()
    };
    let hub = hub_with_settings(&settings);

    for _ in 0..3 {
        simulate_sentry_event(&hub);
    }

    let num_events = metrics::sentry::events_total(Level::Error).get();
    assert!(num_events >= 1);
    assert!(num_events < 3);
}

#[test]
fn sentry_hook_preserves_previous_before_send_hook() {
    let previous_hook_count = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&previous_hook_count);

    let mut options = ClientOptions {
        // Install a custom before_send hook first
        before_send: Some(Arc::new(move |event| {
            counter.fetch_add(1, Ordering::Relaxed);
            Some(event)
        })),
        ..Default::default()
    };

    // Now install foundations hook
    foundations_sentry::install_hook_with_settings(&mut options, &Default::default());

    let hub = hub_with_options(options);

    simulate_sentry_event(&hub);
    simulate_sentry_event(&hub);

    // Both hooks should have been called
    assert_eq!(previous_hook_count.load(Ordering::Relaxed), 2);
    assert_eq!(metrics::sentry::events_total(Level::Error).get(), 2);
}

#[test]
fn sentry_hook_works_across_threads() {
    let hub = Arc::new(hub_with_settings(&Default::default()));

    // Simulate events from multiple threads
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let hub = Arc::clone(&hub);
            std::thread::spawn(move || simulate_sentry_event(&hub))
        })
        .collect();

    simulate_sentry_event(&hub);

    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(metrics::sentry::events_total(Level::Error).get(), 3);
}

#[test]
fn cloned_client_options_have_hook_installed() {
    // Initialize the first hub
    let hub1 = hub_with_settings(&Default::default());

    // Get the first hub's client and clone its options
    let client1 = hub1.client().expect("client should be bound");
    let cloned_options = client1.options().clone();

    // Create a second hub from the cloned options
    let new_client = Arc::new(sentry_core::Client::with_options(cloned_options));
    let hub2 = Hub::new(Some(new_client), Default::default());

    // Capture an event with the second hub/client
    simulate_sentry_event(&hub2);

    // The hook should have been called via the cloned options
    assert_eq!(metrics::sentry::events_total(Level::Error).get(), 1);
}

fn rate_limit(max_events: u32, period_secs: u64) -> EventRateLimitSettings {
    EventRateLimitSettings {
        max_events,
        period_secs,
    }
}

#[test]
fn sentry_hook_drops_events_over_the_global_rate_limit_and_counts_them() {
    let settings = SentrySettings {
        global_rate_limit: rate_limit(2, 3600),
        ..Default::default()
    };
    let hub = hub_with_settings(&settings);

    for message in ["a", "b", "c"] {
        hub.capture_message(message, Level::Error);
    }

    assert_eq!(metrics::sentry::events_total(Level::Error).get(), 2);
    assert_eq!(
        metrics::sentry::events_rate_limited_total(Level::Error, "global").get(),
        1
    );

    const SERIES: &str = "sentry_events_rate_limited_total{level=\"error\",limit=\"global\"} ";

    #[allow(deprecated)]
    let metrics = foundations::telemetry::metrics::collect(&Default::default()).unwrap();
    let has_metric = metrics.lines().any(|line| {
        line.strip_prefix(SERIES)
            .and_then(|value| value.parse::<f64>().ok())
            == Some(1.0)
    });
    assert!(has_metric, "{metrics}");
}

#[test]
fn sentry_hook_limits_fatal_events_separately() {
    let settings = SentrySettings {
        global_rate_limit: rate_limit(1, 3600),
        fatal_rate_limit: rate_limit(1, 3600),
        ..Default::default()
    };
    let hub = hub_with_settings(&settings);

    hub.capture_message("error a", Level::Error);
    hub.capture_message("error b", Level::Error);
    hub.capture_message("panic a", Level::Fatal);
    hub.capture_message("panic b", Level::Fatal);

    assert_eq!(metrics::sentry::events_total(Level::Error).get(), 1);
    assert_eq!(metrics::sentry::events_total(Level::Fatal).get(), 1);
    assert_eq!(
        metrics::sentry::events_rate_limited_total(Level::Error, "global").get(),
        1
    );
    assert_eq!(
        metrics::sentry::events_rate_limited_total(Level::Fatal, "fatal").get(),
        1
    );
}

#[test]
fn sentry_hook_does_not_run_previous_hook_for_rate_limited_events() {
    let previous_hook_count = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&previous_hook_count);

    let mut options = ClientOptions {
        before_send: Some(Arc::new(move |event| {
            counter.fetch_add(1, Ordering::Relaxed);
            Some(event)
        })),
        ..Default::default()
    };
    let settings = SentrySettings {
        per_fingerprint_rate_limit: rate_limit(1, 3600),
        ..Default::default()
    };
    foundations_sentry::install_hook_with_settings(&mut options, &settings);
    let hub = hub_with_options(options);

    simulate_sentry_event(&hub);
    simulate_sentry_event(&hub);

    assert_eq!(previous_hook_count.load(Ordering::Relaxed), 1);
    assert_eq!(metrics::sentry::events_total(Level::Error).get(), 1);
    assert_eq!(
        metrics::sentry::events_rate_limited_total(Level::Error, "fingerprint").get(),
        1
    );
}

#[test]
fn events_dropped_by_previous_hook_do_not_use_the_shared_allowance() {
    let mut options = ClientOptions {
        before_send: Some(Arc::new(|event| {
            (event.message.as_deref() != Some("noise")).then_some(event)
        })),
        ..Default::default()
    };
    let settings = SentrySettings {
        global_rate_limit: rate_limit(1, 3600),
        fatal_rate_limit: rate_limit(1, 3600),
        ..Default::default()
    };
    foundations_sentry::install_hook_with_settings(&mut options, &settings);
    let hub = hub_with_options(options);

    for _ in 0..10 {
        hub.capture_message("noise", Level::Error);
        hub.capture_message("noise", Level::Fatal);
    }
    hub.capture_message("error", Level::Error);
    hub.capture_message("panic", Level::Fatal);

    assert_eq!(metrics::sentry::events_total(Level::Error).get(), 1);
    assert_eq!(metrics::sentry::events_total(Level::Fatal).get(), 1);
    assert_eq!(
        metrics::sentry::events_rate_limited_total(Level::Error, "global").get(),
        0
    );
    assert_eq!(
        metrics::sentry::events_rate_limited_total(Level::Fatal, "fatal").get(),
        0
    );
}

#[test]
fn cloned_client_options_share_the_rate_limits() {
    let settings = SentrySettings {
        global_rate_limit: rate_limit(1, 3600),
        ..Default::default()
    };
    let hub1 = hub_with_settings(&settings);
    let cloned_options = hub1.client().unwrap().options().clone();
    let hub2 = Hub::new(
        Some(Arc::new(sentry_core::Client::with_options(cloned_options))),
        Default::default(),
    );

    hub1.capture_message("a", Level::Error);
    hub2.capture_message("b", Level::Error);

    assert_eq!(metrics::sentry::events_total(Level::Error).get(), 1);
    assert_eq!(
        metrics::sentry::events_rate_limited_total(Level::Error, "global").get(),
        1
    );
}

#[test]
fn settings_are_parsed_from_yaml() {
    let settings: SentrySettings = foundations::settings::from_yaml_str(
        r#"
        per_fingerprint_rate_limit: { max_events: 10, period_secs: 60 }
        global_rate_limit: { max_events: 60 }
        "#,
    )
    .unwrap();

    assert_eq!(settings.per_fingerprint_rate_limit.max_events, 10);
    assert_eq!(settings.per_fingerprint_rate_limit.period_secs, 60);
    assert_eq!(settings.global_rate_limit.max_events, 60);
    assert_eq!(settings.global_rate_limit.period_secs, 1);
    assert_eq!(settings.fatal_rate_limit.max_events, 0);
    assert_eq!(settings.max_events_per_second, None);
    assert_eq!(settings.max_tracked_fingerprints, 10_000);
}

#[test]
fn default_settings_keep_rate_limits_disabled() {
    let hub = hub_with_settings(&SentrySettings::default());

    for _ in 0..100 {
        simulate_sentry_event(&hub);
    }

    assert_eq!(metrics::sentry::events_total(Level::Error).get(), 100);
}
