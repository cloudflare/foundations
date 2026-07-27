//! Registration glue used by the [`metrics`](super::metrics) macro when the
//! `foundations-metrics` backend is enabled.
//!
//! This module mirrors the surface that the macro expects from the legacy
//! `internal` module, so the macro expands to the same tokens under both
//! backends.

use foundations_metrics::{EncodeMetric, NamedMetric, RegistrationMetadata, register};

/// Registers a metric declared through the [`metrics`](super::metrics) macro.
///
/// `subsystem` and `name` are only meaningful to the legacy backend, which
/// composes the exported name through nested registries. Here the macro
/// supplies the already composed `full_name`, and the service name is applied
/// at collection time.
pub fn register_metric<M>(
    _subsystem: &'static str,
    _name: &'static str,
    full_name: &'static str,
    help: &'static str,
    metric: M,
    optional: bool,
    with_service_prefix: bool,
) where
    NamedMetric<M>: EncodeMetric,
{
    register(
        Box::new(NamedMetric::new(full_name, help, metric)) as Box<dyn EncodeMetric>,
        RegistrationMetadata::default()
            .optional(optional)
            .unprefixed(!with_service_prefix),
    );
}
