use crate::telemetry::tokio_runtime_metrics::metrics::{tokio_runtime_core, tokio_runtime_worker};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::runtime::{Handle, RuntimeMetrics};

pub(super) struct RuntimeHandle {
    pub(super) runtime_name: Option<Arc<str>>,
    pub(super) runtime_id: Option<usize>,
    pub(super) handle: Handle,
}

impl RuntimeHandle {
    pub(super) fn record_sample(&mut self) {
        let metrics = self.handle.metrics();

        self.record_global_metrics(&metrics);

        for worker_idx in 0..metrics.num_workers() {
            self.record_worker_metrics(&metrics, worker_idx);
        }
    }

    fn record_global_metrics(&mut self, metrics: &RuntimeMetrics) {
        tokio_runtime_core::workers(&self.runtime_name, self.runtime_id)
            .set(metrics.num_workers() as u64);

        tokio_runtime_core::blocking_threads(&self.runtime_name, self.runtime_id)
            .set(metrics.num_blocking_threads() as u64);

        tokio_runtime_core::num_alive_tasks(&self.runtime_name, self.runtime_id)
            .set(metrics.num_alive_tasks() as u64);

        tokio_runtime_core::idle_blocking_threads(&self.runtime_name, self.runtime_id)
            .set(metrics.num_idle_blocking_threads() as u64);

        tokio_runtime_core::remote_schedules_total(&self.runtime_name, self.runtime_id)
            .inner()
            .store(metrics.remote_schedule_count(), Ordering::SeqCst);

        tokio_runtime_core::budget_forced_yields_total(&self.runtime_name, self.runtime_id)
            .inner()
            .store(metrics.budget_forced_yield_count(), Ordering::SeqCst);

        tokio_runtime_core::io_driver_fd_registrations_total(&self.runtime_name, self.runtime_id)
            .inner()
            .store(metrics.io_driver_fd_registered_count(), Ordering::SeqCst);

        tokio_runtime_core::io_driver_fd_deregistrations_total(&self.runtime_name, self.runtime_id)
            .inner()
            .store(metrics.io_driver_fd_deregistered_count(), Ordering::SeqCst);

        tokio_runtime_core::io_driver_fd_readies_total(&self.runtime_name, self.runtime_id)
            .inner()
            .store(metrics.io_driver_ready_count(), Ordering::SeqCst);

        tokio_runtime_core::global_queue_depth(&self.runtime_name, self.runtime_id)
            .set(metrics.global_queue_depth() as u64);

        tokio_runtime_core::blocking_queue_depth(&self.runtime_name, self.runtime_id)
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
