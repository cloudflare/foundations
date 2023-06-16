use super::{info_metric, InfoMetric};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use prometheus_client::encoding::text::{encode, EncodeMetric};
use prometheus_client::registry::Registry;
use prometools::serde::InfoGauge;
use std::any::TypeId;
use std::collections::HashMap;
use std::io;

pub(super) static INFO_REGISTRY: Lazy<RwLock<HashMap<TypeId, Box<dyn ErasedInfoMetric>>>> =
    Lazy::new(Default::default);

#[doc(hidden)]
pub static REGISTRY: Lazy<RwLock<Registry>> =
    Lazy::new(|| RwLock::new(Registry::with_prefix(&**METRIC_PREFIX.read())));

#[doc(hidden)]
pub static OPT_REGISTRY: Lazy<RwLock<Registry>> =
    Lazy::new(|| RwLock::new(Registry::with_prefix(&**METRIC_PREFIX.read())));

pub(super) static METRIC_PREFIX: RwLock<&'static str> = RwLock::new("undefined");

/// Build and version information
#[info_metric(crate_path = "crate")]
pub(super) struct BuildInfo {
    pub(super) version: &'static str,
}

/// Information about the process runtime
#[info_metric(crate_path = "crate")]
pub(super) struct RuntimeInfo {
    pub(super) pid: u32,
}

pub(super) trait ErasedInfoMetric: erased_serde::Serialize + Send + Sync + 'static {
    fn name(&self) -> &'static str;

    fn help(&self) -> &'static str;
}

erased_serde::serialize_trait_object!(ErasedInfoMetric);

impl<M> ErasedInfoMetric for M
where
    M: InfoMetric,
{
    fn name(&self) -> &'static str {
        M::NAME
    }

    fn help(&self) -> &'static str {
        M::HELP
    }
}

pub(super) fn collect_info_metrics(buffer: &mut Vec<u8>) -> io::Result<()> {
    let info_registry = INFO_REGISTRY.read();
    let mut registry = Registry::default();

    for info_metric in info_registry.values() {
        let info_gauge = InfoGauge::new(&**info_metric);

        registry.register(info_metric.name(), info_metric.help(), info_gauge)
    }

    encode_registry(buffer, &registry)
}

pub(super) fn encode_registry(
    buffer: &mut Vec<u8>,
    registry: &Registry<impl EncodeMetric>,
) -> io::Result<()> {
    encode(buffer, registry)?;

    if buffer.ends_with(b"# EOF\n") {
        buffer.truncate(buffer.len() - b"# EOF\n".len());
    }

    Ok(())
}
