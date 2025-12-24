use super::{ExtraProducer, InfoMetric, info_metric};
use crate::telemetry::settings::{MetricsSettings, ServiceNameFormat};
use crate::{Result, ServiceInfo};
use prometheus_client::encoding::text::{EncodeMetric, encode};
use prometheus_client::registry::Registry;
use prometools::serde::InfoGauge;
use std::any::TypeId;
use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::OnceLock;

static REGISTRIES: OnceLock<Registries> = OnceLock::new();

enum MetricsServiceName {
    Prefix(String),
    Label(String, String),
}

impl MetricsServiceName {
    fn new(name: &str, format: ServiceNameFormat) -> Self {
        let name = name.to_owned();
        match format {
            ServiceNameFormat::MetricPrefix => Self::Prefix(name),
            ServiceNameFormat::LabelWithName(label) => Self::Label(label, name),
        }
    }
}

#[doc(hidden)]
pub struct Registries {
    // NOTE: we intentionally use a lock without poisoning here to not
    // panic the threads if they just share telemetry with failed thread.
    main: parking_lot::RwLock<Registry>,
    opt: parking_lot::RwLock<Registry>,
    pub(super) info: parking_lot::RwLock<HashMap<TypeId, Box<dyn ErasedInfoMetric>>>,
    service_name: MetricsServiceName,
    extra_producers: parking_lot::RwLock<Vec<Box<dyn ExtraProducer>>>,
}

impl Registries {
    pub(super) fn init(service_info: &ServiceInfo, settings: &MetricsSettings) {
        let service_name = MetricsServiceName::new(
            &service_info.name_in_metrics,
            settings.service_name_format.clone(),
        );

        // FIXME(nox): Due to prometheus-client 0.18 not supporting the creation of
        // registries with specific label values, we use `MetricsServiceName::Label`
        // directly in `Registries::get_subsystem`.
        REGISTRIES.get_or_init(|| Registries {
            main: Default::default(),
            opt: Default::default(),
            info: Default::default(),
            service_name,
            extra_producers: Default::default(),
        });
    }

    pub(super) fn collect(buffer: &mut Vec<u8>, collect_optional: bool) -> Result<()> {
        let registries = Self::get();

        registries.collect_info_metrics(buffer)?;

        encode_registry(buffer, &registries.main.read())?;

        if collect_optional {
            encode_registry(buffer, &registries.opt.read())?;
        }

        for producer in registries.extra_producers.read().iter() {
            producer.produce(buffer);
            truncate_eof(buffer);
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

    pub fn get_subsystem(
        subsystem: &str,
        optional: bool,
        with_service_prefix: bool,
    ) -> impl DerefMut<Target = Registry> + 'static {
        let registries = Self::get();
        let registry = if optional {
            &registries.opt
        } else {
            &registries.main
        };

        let mut prefix = Cow::Borrowed(subsystem);
        if with_service_prefix && let MetricsServiceName::Prefix(service) = &registries.service_name
        {
            prefix = format!("{service}_{subsystem}").into();
        }

        parking_lot::RwLockWriteGuard::map(registry.write(), move |mut reg| {
            if let MetricsServiceName::Label(name, val) = &registries.service_name {
                reg = reg.sub_registry_with_label((name.into(), val.into()));
            }
            reg.sub_registry_with_prefix(prefix)
        })
    }

    pub fn add_extra_producer(&self, producer: Box<dyn ExtraProducer>) {
        self.extra_producers.write().push(producer);
    }

    pub(super) fn get() -> &'static Registries {
        REGISTRIES.get_or_init(|| Registries {
            main: Default::default(),
            opt: Default::default(),
            info: Default::default(),
            service_name: MetricsServiceName::Prefix("undefined".to_owned()),
            extra_producers: Default::default(),
        })
    }
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

    truncate_eof(buffer);

    Ok(())
}

fn truncate_eof(buffer: &mut Vec<u8>) {
    if buffer.ends_with(b"# EOF\n") {
        buffer.truncate(buffer.len() - b"# EOF\n".len());
    }
}
