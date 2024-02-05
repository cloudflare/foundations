//! Toolkit for monitoring tokio runtimes using the [`foundations metrics api`].
//!
//! Foundations provides a simple toolkit for registering tokio runtimes with a monitor and collecting runtime metrics
//! for registered runtimes.
//!
//! # Note
//! This is currently an [unstable API](https://docs.rs/foundations/latest/foundations/index.html#unstable-features).
//!
//! # Metrics
//! | Metric                                           | Source                                                              | Labels (? indicates optional)          |
//! |:-------------------------------------------------|:--------------------------------------------------------------------|:---------------------------------------|
//! | tokio_runtime_workers                            | [`tokio::runtime::RuntimeMetrics::num_workers`]                     | runtime_name?, runtime_id?             |
//! | tokio_runtime_blocking_threads                   | [`tokio::runtime::RuntimeMetrics::num_blocking_threads`]            | runtime_name?, runtime_id?             |
//! | tokio_runtime_active_tasks                       | [`tokio::runtime::RuntimeMetrics::active_tasks_count`]              | runtime_name?, runtime_id?             |
//! | tokio_runtime_idle_blocking_threads              | [`tokio::runtime::RuntimeMetrics::num_idle_blocking_threads`]       | runtime_name?, runtime_id?             |
//! | tokio_runtime_remote_schedules_total             | [`tokio::runtime::RuntimeMetrics::remote_schedule_count`]           | runtime_name?, runtime_id?             |
//! | tokio_runtime_budget_forced_yields_total         | [`tokio::runtime::RuntimeMetrics::budget_forced_yield_count`]       | runtime_name?, runtime_id?             |
//! | tokio_runtime_io_driver_fd_registrations_total   | [`tokio::runtime::RuntimeMetrics::io_driver_fd_registered_count`]   | runtime_name?, runtime_id?             |
//! | tokio_runtime_io_driver_fd_deregistrations_total | [`tokio::runtime::RuntimeMetrics::io_driver_fd_deregistered_count`] | runtime_name?, runtime_id?             |
//! | tokio_runtime_io_driver_fd_readies_total         | [`tokio::runtime::RuntimeMetrics::io_driver_ready_count`]           | runtime_name?, runtime_id?             |
//! | tokio_runtime_injection_queue_depth              | [`tokio::runtime::RuntimeMetrics::injection_queue_depth`]           | runtime_name?, runtime_id?             |
//! | tokio_runtime_blocking_queue_depth               | [`tokio::runtime::RuntimeMetrics::blocking_queue_depth`]            | runtime_name?, runtime_id?             |
//! | tokio_runtime_worker_parks_total                 | [`tokio::runtime::RuntimeMetrics::worker_park_count`]               | runtime_name?, runtime_id?, worker_idx |
//! | tokio_runtime_worker_noops_total                 | [`tokio::runtime::RuntimeMetrics::worker_noop_count`]               | runtime_name?, runtime_id?, worker_idx |
//! | tokio_runtime_worker_task_steals_total           | [`tokio::runtime::RuntimeMetrics::worker_steal_count`]              | runtime_name?, runtime_id?, worker_idx |
//! | tokio_runtime_worker_steal_operations_total      | [`tokio::runtime::RuntimeMetrics::worker_steal_operations`]         | runtime_name?, runtime_id?, worker_idx |
//! | tokio_runtime_worker_polls_total                 | [`tokio::runtime::RuntimeMetrics::worker_poll_count`]               | runtime_name?, runtime_id?, worker_idx |
//! | tokio_runtime_worker_busy_duration_micros_total  | [`tokio::runtime::RuntimeMetrics::worker_total_busy_duration`]      | runtime_name?, runtime_id?, worker_idx |
//! | tokio_runtime_worker_local_schedules_total       | [`tokio::runtime::RuntimeMetrics::worker_local_schedule_count`]     | runtime_name?, runtime_id?, worker_idx |
//! | tokio_runtime_worker_overflows_total             | [`tokio::runtime::RuntimeMetrics::worker_overflow_count`]           | runtime_name?, runtime_id?, worker_idx |
//! | tokio_runtime_worker_local_queue_depth           | [`tokio::runtime::RuntimeMetrics::worker_local_queue_depth`]        | runtime_name?, runtime_id?, worker_idx |
//! | tokio_runtime_worker_mean_poll_time_micros       | [`tokio::runtime::RuntimeMetrics::worker_mean_poll_time`]           | runtime_name?, runtime_id?, worker_idx |
//!
//! # Example
//! ```no_run
//! # use std::thread;
//! # use std::time::Duration;
//! # use foundations::telemetry::tokio_runtime_metrics::{record_runtime_metrics_sample, register_runtime};
//! // create and monitor 8 runtimes spawned on other threads, then poll in the background from this one
//! for i in 0..8 {
//!     let r = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
//!     
//!     register_runtime(None, Some(i), r.handle());
//!
//!     thread::spawn(move || r.block_on(async {
//!         loop {
//!             println!("Poll");
//!             tokio::time::sleep(Duration::from_secs(1)).await;
//!         }
//!     }));
//! }
//!
//! loop {
//!     record_runtime_metrics_sample();
//!
//!     // record metrics roughly twice a second
//!     std::thread::sleep(Duration::from_secs_f32(0.5));
//! }
//! ```
//!
//! [`foundations metrics api`]: crate::telemetry::metrics

mod metrics;
mod runtime_handle;

use crate::telemetry::tokio_runtime_metrics::runtime_handle::RuntimeHandle;
use parking_lot::Mutex;
use slab::Slab;
use std::sync::Arc;
use tokio::runtime::Handle;

static MONITOR: Mutex<Slab<RuntimeHandle>> = Mutex::new(Slab::new());

pub struct Key(usize);

/// Add a runtime to the global monitor, optionally with a name and/or id in case you are monitoring multiple runtimes.
///
/// Runtimes should be uniquely identifiable by label/id tuple.
///
/// You can use IDs as either unique IDs among all runtimes or unique IDs within a namespace based on the runtime name.
///
/// This function returns a key which can later be used to remove the registered runtime.
pub fn register_runtime(
    runtime_name: Option<Arc<str>>,
    runtime_id: Option<usize>,
    handle: &Handle,
) -> Key {
    let mut monitor = MONITOR.lock();

    assert!(
        monitor.iter().all(
            |(_, handle)| (handle.runtime_name.as_ref(), handle.runtime_id)
                != (runtime_name.as_ref(), runtime_id)
        ),
        "Runtime name and index tuples must be unique"
    );

    Key(monitor.insert(RuntimeHandle {
        runtime_name,
        runtime_id,
        handle: handle.clone(),
    }))
}

/// Try and deregister a runtime, returning true if a runtime is tracked for the specified key, and false otherwise.
pub fn deregister_runtime(key: Key) -> bool {
    MONITOR.lock().try_remove(key.0).is_some()
}

/// Record a sample of runtime metrics for each tracked runtime.
pub fn record_runtime_metrics_sample() {
    MONITOR
        .lock()
        .iter_mut()
        .for_each(|(_, h)| h.record_sample())
}
