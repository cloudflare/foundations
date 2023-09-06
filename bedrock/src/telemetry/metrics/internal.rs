use super::{info_metric, InfoMetric};
use crate::telemetry::settings::{MetricsSettings, ServiceNameFormat};
use crate::{Result, ServiceInfo};
use once_cell::sync::OnceCell;
use parking_lot::{RwLock, RwLockWriteGuard};
use prometheus_client::encoding::text::{encode, EncodeMetric};
use prometheus_client::registry::Registry;
use prometools::serde::InfoGauge;
use std::any::TypeId;
use std::collections::HashMap;
use std::ops::DerefMut;

static REGISTRIES: OnceCell<Registries> = OnceCell::new();

#[doc(hidden)]
pub struct Registries {
    main: RwLock<Registry>,
    opt: RwLock<Registry>,
    pub(super) info: RwLock<HashMap<TypeId, Box<dyn ErasedInfoMetric>>>,
    extra_label: Option<(String, String)>,
}

impl Registries {
    pub(super) fn init(service_info: &ServiceInfo, settings: &MetricsSettings) {
        let extra_label = match &settings.service_name_format {
            ServiceNameFormat::MetricPrefix => None,
            ServiceNameFormat::LabelWithName(name) => {
                Some((name.clone(), service_info.name_in_metrics.clone()))
            }
        };

        REGISTRIES.get_or_init(|| Registries {
            main: new_registry(service_info, settings),
            opt: new_registry(service_info, settings),
            info: Default::default(),
            extra_label,
        });
    }

    pub(super) fn collect(buffer: &mut Vec<u8>, collect_optional: bool) -> Result<()> {
        let registries = Self::get();

        registries.collect_info_metrics(buffer)?;

        encode_registry(buffer, &registries.main.read())?;

        if collect_optional {
            encode_registry(buffer, &registries.opt.read())?;
        }

        Ok(())
    }

    fn collect_info_metrics(&self, buffer: &mut Vec<u8>) -> Result<()> {
        let info_registry = self.info.read();
        let mut registry = Registry::default();

        for info_metric in info_registry.values() {
            let info_gauge = InfoGauge::new(&**info_metric);

            registry.register(info_metric.name(), info_metric.help(), info_gauge)
        }

        encode_registry(buffer, &registry)
    }

    pub fn get_main_subsystem(subsystem: &str) -> impl DerefMut<Target = Registry> + '_ {
        let registries = Self::get();

        get_subsystem(
            Self::get().main.write(),
            subsystem,
            registries.extra_label.clone(),
        )
    }

    pub fn get_opt_subsystem(subsystem: &str) -> impl DerefMut<Target = Registry> + '_ {
        let registries = Self::get();

        get_subsystem(
            Self::get().opt.write(),
            subsystem,
            registries.extra_label.clone(),
        )
    }

    pub(super) fn get() -> &'static Registries {
        REGISTRIES.get().expect("registries are not initialized")
    }
}

fn new_registry(service_info: &ServiceInfo, settings: &MetricsSettings) -> RwLock<Registry> {
    RwLock::new(match &settings.service_name_format {
        ServiceNameFormat::MetricPrefix => Registry::with_prefix(&service_info.name_in_metrics),
        // FIXME(nox): Due to prometheus-client 0.18 not supporting the creation of
        // registries with specific label values, we use this service identifier
        // format directly in `Registries::get_main` and `Registries::get_optional`.
        ServiceNameFormat::LabelWithName(_) => Registry::default(),
    })
}

fn get_subsystem<'a>(
    registry: RwLockWriteGuard<'a, Registry>,
    subsystem: &str,
    extra_label: Option<(String, String)>,
) -> impl DerefMut<Target = Registry> + 'a {
    RwLockWriteGuard::map(registry, move |mut registry| {
        if let Some((name, value)) = extra_label {
            registry = registry.sub_registry_with_label((name.into(), value.into()));
        }

        registry.sub_registry_with_prefix(subsystem)
    })
}

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

pub(super) fn encode_registry(
    buffer: &mut Vec<u8>,
    registry: &Registry<impl EncodeMetric>,
) -> Result<()> {
    encode(buffer, registry)?;

    if buffer.ends_with(b"# EOF\n") {
        buffer.truncate(buffer.len() - b"# EOF\n".len());
    }

    Ok(())
}
