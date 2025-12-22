//! These tests assume a separate process is used. Make sure you run with `cargo
//! nextest run`.

fn simulate_panic() {
    let _ = std::panic::catch_unwind(|| panic!("oh no! 😱"));
}

#[cfg(feature = "metrics")]
mod with_metrics {
    use std::{
        panic::PanicHookInfo,
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc,
        },
    };

    use foundations::{
        panic::metrics,
        service_info,
        telemetry::{TelemetryConfig, TestTelemetryContext},
    };
    use foundations_macros::with_test_telemetry;
    use slog::Level;

    use super::simulate_panic;

    #[test]
    fn panic_hook_init_returns_true_on_first_call() {
        let is_installed = foundations::panic::hook().init();
        assert!(is_installed);

        simulate_panic();
        assert_eq!(metrics::panics::total().get(), 1)
    }

    #[test]
    fn panic_hook_metrics_are_well_formed() {
        let is_installed = foundations::panic::hook().init();
        assert!(is_installed);

        simulate_panic();
        assert_eq!(metrics::panics::total().get(), 1);

        let metrics = foundations::telemetry::metrics::collect(&Default::default()).unwrap();
        let has_metric = metrics.lines().any(|line| line == "panics_total 1");
        assert!(has_metric);
    }

    #[test]
    fn panic_hook_init_is_idempotent() {
        let first = foundations::panic::hook().init();
        let second = foundations::panic::hook().init();

        assert!(first);
        assert!(!second);

        simulate_panic();
        assert_eq!(metrics::panics::total().get(), 1)
    }

    #[test]
    fn panic_hook_works_across_threads() {
        foundations::panic::hook().init();

        // simulate two panics, one in another thread:
        simulate_panic();
        let handle = std::thread::spawn(simulate_panic);
        handle.join().unwrap();

        assert_eq!(metrics::panics::total().get(), 2)
    }

    #[test]
    fn panic_hook_works_in_tokio_tasks() {
        foundations::panic::hook().init();

        // panic before tokio is initialized:
        simulate_panic();

        let rt = tokio::runtime::Builder::new_multi_thread().build().unwrap();
        // panic in two tasks:
        let handle_1 = rt.spawn(async {
            simulate_panic();
        });
        let handle_2 = rt.spawn(async {
            simulate_panic();
        });

        rt.block_on(async move {
            handle_1.await.unwrap();
            handle_2.await.unwrap();
        });

        // three panics total:
        assert_eq!(metrics::panics::total().get(), 3)
    }

    #[test]
    fn panic_hook_works_in_tokio_tasks_after_runtime_is_initialized() {
        let rt = tokio::runtime::Builder::new_multi_thread().build().unwrap();

        // install the hook after the runtime has started
        foundations::panic::hook().init();

        // panic in two tasks:
        let handle_1 = rt.spawn(async {
            simulate_panic();
        });
        let handle_2 = rt.spawn(async {
            simulate_panic();
        });

        rt.block_on(async move {
            handle_1.await.unwrap();
            handle_2.await.unwrap();
        });

        // panic outside of the runtime
        simulate_panic();

        assert_eq!(metrics::panics::total().get(), 3)
    }

    #[test]
    fn panic_hook_does_not_override_current_hook() {
        let create_hook =
            |count: Arc<AtomicU64>| -> Box<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static> {
                Box::new(move |_| {
                    count.fetch_add(1, Ordering::Relaxed);
                })
            };

        // install a hook before foundations
        let count = Arc::new(AtomicU64::new(0));
        std::panic::set_hook(create_hook(Arc::clone(&count)));
        simulate_panic();

        foundations::panic::hook().init();
        simulate_panic();

        // Make sure the previous hook saw two total panics:
        assert_eq!(count.load(Ordering::Relaxed), 2);

        // foundations saw only one panic:
        assert_eq!(metrics::panics::total().get(), 1);
    }

    #[with_test_telemetry(test)]
    fn error_log_is_emitted(ctx: TestTelemetryContext) {
        foundations::panic::hook().init();

        simulate_panic();
        assert_eq!(metrics::panics::total().get(), 1);

        let panic_log = {
            let logs = ctx.log_records();
            logs.first().unwrap().clone()
        };

        assert_eq!(panic_log.level, Level::Error);
        assert_eq!(panic_log.message, "panic occurred");
        let has_panic_payload = panic_log
            .fields
            .iter()
            .any(|(key, value)| key == "payload" && value == "oh no! 😱");
        assert!(has_panic_payload);
    }

    #[tokio::test]
    async fn hook_is_auto_initialized() {
        foundations::telemetry::init(TelemetryConfig {
            service_info: &service_info!(),
            settings: &Default::default(),
            custom_server_routes: Default::default(),
        })
        .unwrap();

        simulate_panic();
        assert_eq!(metrics::panics::total().get(), 1);
    }
}

#[cfg(not(feature = "metrics"))]
mod no_metrics {
    use super::simulate_panic;
    use foundations::panic::PanicsMetricsRegistry;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };

    #[derive(Clone)]
    struct TestRegistry {
        panics: Arc<AtomicU64>,
    }

    impl TestRegistry {
        fn new() -> Self {
            Self {
                panics: Arc::new(AtomicU64::new(0)),
            }
        }

        fn panics(&self) -> u64 {
            self.panics.load(Ordering::Relaxed)
        }
    }

    impl PanicsMetricsRegistry for TestRegistry {
        fn inc_panics_total(&self, by: u64) {
            self.panics.fetch_add(by, Ordering::Relaxed);
        }
    }

    #[test]
    fn custom_registry() {
        let registry = TestRegistry::new();
        let first_install = foundations::panic::hook()
            .with_registry(registry.clone())
            .init();

        assert!(first_install);

        simulate_panic();
        assert_eq!(registry.panics(), 1);
    }
}
