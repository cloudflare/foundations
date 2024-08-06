use crate::telemetry::metrics::{metrics, Counter, Gauge};
use std::sync::Arc;

#[metrics(crate_path = "crate")]
pub(super) mod tokio_runtime_core {
    /// Number of worker threads in use by the runtime.
    ///
    /// This number shouldn't change during execution.
    pub fn workers(runtime_name: &Option<Arc<str>>, runtime_id: Option<usize>) -> Gauge;

    /// Current number of blocking threads allocated by the runtime.
    ///
    /// This should ideally be less than the blocking threads limit, otherwise you may be experiencing
    /// resource saturation at least some proportion of the time.
    pub fn blocking_threads(runtime_name: &Option<Arc<str>>, runtime_id: Option<usize>) -> Gauge;

    /// Current number of active tasks on the runtime.
    pub fn num_alive_tasks(runtime_name: &Option<Arc<str>>, runtime_id: Option<usize>) -> Gauge;

    /// Current number of idle blocking threads on the runtime which aren't doing anything.
    ///
    /// This can give a good idea of how much of the thread pool is being utilized.
    ///
    /// If this is a very low number relative to the number of allocated blocking threads,
    /// and we are reaching the limit for blocking thread allocations,
    /// then we may be experiencing saturation of the thread pool.
    pub fn idle_blocking_threads(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Gauge;

    /// Counter of schedules not originating from a worker on the runtime.
    ///
    /// Remote schedules tend to be slower than local ones, and occur when a wake or spawn happens
    /// off of a worker (e.g. on a background thread or in the block_on call).
    pub fn remote_schedules_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    /// Counter of forced yields due to task budgeting.
    pub fn budget_forced_yields_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    /// Counter of file descriptors registered with the IO driver.
    pub fn io_driver_fd_registrations_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    /// Counter of file descriptors deregistered with the IO driver.
    pub fn io_driver_fd_deregistrations_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    /// Counter of readiness events received via the IO driver.
    pub fn io_driver_fd_readies_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Counter;

    /// Current depth of the tokio runtime injection queue.
    pub fn injection_queue_depth(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Gauge;

    /// Current depth of the tokio runtime blocking queue.
    ///
    /// If this is growing, then we have saturated our blocking pool and either need more threads
    /// or cgroups cpu time allotment.
    pub fn blocking_queue_depth(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
    ) -> Gauge;
}

#[metrics(crate_path = "crate")]
pub(super) mod tokio_runtime_worker {
    /// Total number of times this worker has parked.
    pub fn parks_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    /// Total number of spurious noop parks this worker has experienced.
    ///
    /// If this is happening a lot, it might be worth investigating what is happening in tokio and
    /// potentially your kernel as well.
    pub fn noops_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    /// Total number of tasks stolen due to work-stealing by this worker.
    pub fn task_steals_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    /// Total number of times that this worker has stolen one or more tasks.
    pub fn steal_operations_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    /// Total number of times that this worker has polled a task.
    pub fn polls_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    /// Total amount of time that this worker has been polling tasks.
    ///
    /// Ideally, workers should be incrementing this threshold relatively evenly,
    /// otherwise you are experiencing load balancing issues for some reason.
    pub fn busy_duration_micros_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    /// Total number of local schedules.
    ///
    /// Cumulatively, this should generally be high relative to remote schedules.
    ///
    /// Otherwise, you are seeing a high proportion of off-runtime wakes, which can be slower.
    pub fn local_schedules_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    /// Total number of times that this worker has overflown its local queue, pushing excess tasks
    /// to the injector queue.
    pub fn overflows_total(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Counter;

    /// Current depth of this worker's local run queue.
    pub fn local_queue_depth(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Gauge;

    /// Moving average of task poll times for this worker.
    pub fn mean_poll_time_micros(
        runtime_name: &Option<Arc<str>>,
        runtime_id: Option<usize>,
        worker_idx: usize,
    ) -> Gauge;
}
