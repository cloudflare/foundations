use crate::telemetry::tokio_runtime_metrics::metrics::{tokio_runtime_core, tokio_runtime_worker};
use std::sync::Arc;
use std::sync::atomic::Ordering;
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

        let parked_workers = (0..metrics.num_workers())
            .filter(|&worker_idx| metrics.worker_park_unpark_count(worker_idx) % 2 == 1)
            .count();
        tokio_runtime_core::parked_workers(&self.runtime_name, self.runtime_id)
            .set(parked_workers as u64);

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn samples_parked_workers_from_tokio_metrics() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let metrics = runtime.metrics();
        let mut monitor = RuntimeHandle {
            runtime_name: Some("samples_parked_workers_from_tokio_metrics".into()),
            runtime_id: Some(0),
            handle: runtime.handle().clone(),
        };
        let parked_workers =
            tokio_runtime_core::parked_workers(&monitor.runtime_name, monitor.runtime_id);

        monitor.record_sample();
        assert_eq!(parked_workers.get(), 0);

        let (wake, receiver) = tokio::sync::oneshot::channel();
        let worker = thread::spawn(move || {
            runtime.block_on(receiver).unwrap();
            runtime
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while metrics.worker_park_unpark_count(0).is_multiple_of(2) {
            assert!(Instant::now() < deadline, "runtime did not park");
            thread::sleep(Duration::from_millis(1));
        }

        monitor.record_sample();
        assert_eq!(parked_workers.get(), 1);
        monitor.record_sample();
        assert_eq!(parked_workers.get(), 1);

        wake.send(()).unwrap();
        let _runtime = worker.join().unwrap();
        monitor.record_sample();
        assert_eq!(parked_workers.get(), 0);
    }
}
