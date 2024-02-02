//! Tokio runtime metrics helper API.
//!
//! Foundations provides a helper API for monitoring Tokio runtimes using the
//! [`foundations metrics api`](crate::telemetry::metrics).
//! This helper API allows users to track one or more runtimes using a [`RuntimeMonitor`] object and collect their
//! metrics periodically using [`RuntimeMonitor::record_sample`].
//!
//! # Note
//! This is currently an [unstable API](https://docs.rs/foundations/latest/foundations/index.html#features).
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
//! # use foundations::telemetry::tokio_runtime_metrics::RuntimeMonitor;
//! // create and monitor 8 runtimes spawned on other threads, then poll in the background from this one
//! let mut monitor = RuntimeMonitor::new();
//!
//! for i in 0..8 {
//!     let r = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
//!     
//!     monitor.add_runtime(None, Some(i), r.handle());
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
//!     monitor.record_sample();
//!
//!     // record metrics roughly twice a second
//!     std::thread::sleep(Duration::from_secs_f32(0.5));
//! }
//! ```

use crate::telemetry::tokio_runtime_metrics::runtime_handle::RuntimeHandle;
use std::sync::Arc;
use tokio::runtime::Handle;

mod metrics;
mod runtime_handle;

/// Monitors a set of runtimes, allowing metrics to be periodically collected for each tracked runtime.
#[derive(Default)]
pub struct RuntimeMonitor {
    runtimes: Vec<RuntimeHandle>,
}

impl RuntimeMonitor {
    /// Construct a new runtime monitor which initially monitors no runtimes.
    pub fn new() -> Self {
        Self {
            runtimes: Vec::new(),
        }
    }

    /// Add a runtime to the monitor, optionally with a name and/or id in case you are monitoring multiple runtimes.
    ///
    /// Runtimes should be uniquely identifiable by both label and id.
    pub fn add_runtime(
        &mut self,
        runtime_name: Option<Arc<str>>,
        runtime_id: Option<usize>,
        handle: &Handle,
    ) {
        if self.runtimes.iter().any(|handle| {
            (handle.runtime_name.is_some() != runtime_name.is_some())
                || (handle.runtime_id.is_some() != runtime_id.is_some())
        }) {
            panic!("If you specify a runtime name or ID for one runtime, that setting must be specified for all");
        }

        if self.runtimes.iter().any(|handle| {
            (handle.runtime_name.is_some() && handle.runtime_name == runtime_name)
                || (handle.runtime_id.is_some() && handle.runtime_id == runtime_id)
        }) {
            panic!("Any runtime names or IDs provided to the RuntimeMonitor must be unique");
        }

        self.runtimes.push(RuntimeHandle {
            runtime_name,
            runtime_id,
            handle: handle.clone(),
        });
    }

    /// Record a sample of runtime metrics for each tracked runtime.
    pub fn record_sample(&mut self) {
        self.runtimes
            .iter_mut()
            .for_each(RuntimeHandle::record_sample)
    }
}
