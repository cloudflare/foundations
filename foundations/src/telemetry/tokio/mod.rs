//! Tokio Runtime Metrics
//!
//! Foundations provides a helper API for monitoring Tokio runtimes using the
//! [`foundations metrics api`](crate::telemetry::metrics).
//! This helper API allows users to track one or more runtimes using a [`RuntimeMonitor`] object and collect their
//! metrics periodically using [`RuntimeMonitor::record_sample`].
//!
//! ## Metrics
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
//! ## Example
//! ```no_run
//! # use std::thread;
//! # use std::time::Duration;
//! # use foundations::telemetry::tokio::RuntimeMonitor;
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

use crate::telemetry::metrics::{metrics, Counter, Gauge};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::runtime::{Handle, RuntimeMetrics};

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

struct RuntimeHandle {
    runtime_name: Option<Arc<str>>,
    runtime_id: Option<usize>,
    handle: Handle,
}

impl RuntimeHandle {
    fn record_sample(&mut self) {
        let metrics = self.handle.metrics();

        self.record_global_metrics(&metrics);

        for worker_idx in 0..metrics.num_workers() {
            self.record_worker_metrics(&metrics, worker_idx);
        }
    }

    fn record_global_metrics(&mut self, metrics: &RuntimeMetrics) {
        tokio_runtime::workers(&self.runtime_name, self.runtime_id)
            .set(metrics.num_workers() as u64);

        tokio_runtime::blocking_threads(&self.runtime_name, self.runtime_id)
            .set(metrics.num_blocking_threads() as u64);

        tokio_runtime::active_tasks(&self.runtime_name, self.runtime_id)
            .set(metrics.active_tasks_count() as u64);

        tokio_runtime::idle_blocking_threads(&self.runtime_name, self.runtime_id)
            .set(metrics.num_idle_blocking_threads() as u64);

        tokio_runtime::remote_schedules_total(&self.runtime_name, self.runtime_id)
            .inner()
            .store(metrics.remote_schedule_count(), Ordering::SeqCst);

        tokio_runtime::budget_forced_yields_total(&self.runtime_name, self.runtime_id)
            .inner()
            .store(metrics.budget_forced_yield_count(), Ordering::SeqCst);

        tokio_runtime::io_driver_fd_registrations_total(&self.runtime_name, self.runtime_id)
            .inner()
            .store(metrics.io_driver_fd_registered_count(), Ordering::SeqCst);

        tokio_runtime::io_driver_fd_deregistrations_total(&self.runtime_name, self.runtime_id)
            .inner()
            .store(metrics.io_driver_fd_deregistered_count(), Ordering::SeqCst);

        tokio_runtime::io_driver_fd_readies_total(&self.runtime_name, self.runtime_id)
            .inner()
            .store(metrics.io_driver_ready_count(), Ordering::SeqCst);

        tokio_runtime::injection_queue_depth(&self.runtime_name, self.runtime_id)
            .set(metrics.injection_queue_depth() as u64);

        tokio_runtime::blocking_queue_depth(&self.runtime_name, self.runtime_id)
            .set(metrics.blocking_queue_depth() as u64);
    }

    fn record_worker_metrics(&mut self, metrics: &RuntimeMetrics, worker_idx: usize) {
        tokio_runtime_worker::parks_total(&self.runtime_name, self.runtime_id, worker_idx)
            .inner()
            .store(metrics.worker_park_count(worker_idx), Ordering::SeqCst);

        tokio_runtime_worker::noops_total(&self.runtime_name, self.runtime_id, worker_idx)
            .inner()
            .store(metrics.worker_noop_count(worker_idx), Ordering::SeqCst);

        tokio_runtime_worker::task_steals_total(&self.runtime_name, self.runtime_id, worker_idx)
            .inner()
            .store(metrics.worker_steal_count(worker_idx), Ordering::SeqCst);

        tokio_runtime_worker::steal_operations_total(
            &self.runtime_name,
            self.runtime_id,
            worker_idx,
        )
        .inner()
        .store(
            metrics.worker_steal_operations(worker_idx),
            Ordering::SeqCst,
        );

        tokio_runtime_worker::polls_total(&self.runtime_name, self.runtime_id, worker_idx)
            .inner()
            .store(metrics.worker_poll_count(worker_idx), Ordering::SeqCst);

        tokio_runtime_worker::busy_duration_micros_total(
            &self.runtime_name,
            self.runtime_id,
            worker_idx,
        )
        .inner()
        .store(
            metrics.worker_total_busy_duration(worker_idx).as_micros() as u64,
            Ordering::SeqCst,
        );

        tokio_runtime_worker::local_schedules_total(
            &self.runtime_name,
            self.runtime_id,
            worker_idx,
        )
        .inner()
        .store(
            metrics.worker_local_schedule_count(worker_idx),
            Ordering::SeqCst,
        );

        tokio_runtime_worker::overflows_total(&self.runtime_name, self.runtime_id, worker_idx)
            .inner()
            .store(metrics.worker_overflow_count(worker_idx), Ordering::SeqCst);

        tokio_runtime_worker::local_queue_depth(&self.runtime_name, self.runtime_id, worker_idx)
            .set(metrics.worker_local_queue_depth(worker_idx) as u64);

        tokio_runtime_worker::mean_poll_time_micros(
            &self.runtime_name,
            self.runtime_id,
            worker_idx,
        )
        .set(metrics.worker_mean_poll_time(worker_idx).as_micros() as u64);
    }
}

#[metrics(crate_path = "crate")]
mod tokio_runtime {
    /// Number of worker threads in use by the runtime.
    ///
    /// This number shouldn't change during execution.
    pub(super) fn workers(runtime_name: &Option<Arc<str>>, runtime_id: Option<usize>) -> Gauge;

    /// Current number of blocking threads allocated by the runtime.
    pub(super) fn blocking_threads(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Gauge;

    /// Current number of active tasks on the runtime.
    pub(super) fn active_tasks(runtime_name: &Option<Arc<str>>, runtime_id: Option<usize>)
        -> Gauge;

    /// Current number of idle blocking threads on the runtime which aren't doing anything.
    pub(super) fn idle_blocking_threads(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Gauge;

    /// Counter of schedules not originating from a worker on the runtime.
    pub(super) fn remote_schedules_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    /// Counter of forced yields due to task budgeting.
    pub(super) fn budget_forced_yields_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    pub(super) fn io_driver_fd_registrations_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    pub(super) fn io_driver_fd_deregistrations_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    pub(super) fn io_driver_fd_readies_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    pub(super) fn injection_queue_depth(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Gauge;

    pub(super) fn blocking_queue_depth(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Gauge;
}

#[metrics(crate_path = "crate")]
mod tokio_runtime_worker {
    pub(super) fn parks_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub(super) fn noops_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub(super) fn task_steals_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub(super) fn steal_operations_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub(super) fn polls_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub(super) fn busy_duration_micros_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub(super) fn local_schedules_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub(super) fn overflows_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub(super) fn local_queue_depth(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Gauge;

    pub(super) fn mean_poll_time_micros(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Gauge;
}
