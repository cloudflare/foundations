use crate::telemetry::metrics::{metrics, Counter, Gauge};
use std::sync::Arc;

#[metrics(crate_path = "crate")]
pub(super) mod tokio_runtime {
    /// Number of worker threads in use by the runtime.
    ///
    /// This number shouldn't change during execution.
    pub fn workers(runtime_name: &Option<Arc<str>>, runtime_id: Option<usize>) -> Gauge;

    /// Current number of blocking threads allocated by the runtime.
    pub fn blocking_threads(runtime_name: &Option<Arc<str>>, runtime_id: Option<usize>) -> Gauge;

    /// Current number of active tasks on the runtime.
    pub fn active_tasks(runtime_name: &Option<Arc<str>>, runtime_id: Option<usize>) -> Gauge;

    /// Current number of idle blocking threads on the runtime which aren't doing anything.
    pub fn idle_blocking_threads(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Gauge;

    /// Counter of schedules not originating from a worker on the runtime.
    pub fn remote_schedules_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    /// Counter of forced yields due to task budgeting.
    pub fn budget_forced_yields_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    pub fn io_driver_fd_registrations_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    pub fn io_driver_fd_deregistrations_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    pub fn io_driver_fd_readies_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    pub fn injection_queue_depth(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Gauge;

    pub fn blocking_queue_depth(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Gauge;
}

#[metrics(crate_path = "crate")]
pub(super) mod tokio_runtime_worker {
    pub fn parks_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub fn noops_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub fn task_steals_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub fn steal_operations_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub fn polls_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub fn busy_duration_micros_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub fn local_schedules_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub fn overflows_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    pub fn local_queue_depth(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Gauge;

    pub fn mean_poll_time_micros(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Gauge;
}
