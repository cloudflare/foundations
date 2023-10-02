use super::field_dedup::FieldDedupFilterFactory;
use super::field_filtering::FieldFilteringDrain;
use super::field_redact::FieldRedactFilterFactory;
use super::internal::SharedLog;
use crate::telemetry::log::rate_limit::RateLimitingDrain;
use crate::telemetry::scope::ScopeStack;
use crate::telemetry::settings::{LogFormat, LogOutput, LoggingSettings};
use crate::{BootstrapResult, ServiceInfo};
use once_cell::sync::{Lazy, OnceCell};
use slog::{
    Discard, Drain, FnValue, Fuse, LevelFilter, Logger, Never, OwnedKV, SendSyncRefUnwindSafeDrain,
    SendSyncRefUnwindSafeKV, SendSyncUnwindSafeDrain,
};
use slog_async::Async as AsyncDrain;
use slog_json::{Json as JsonDrain, Json};
use slog_term::{FullFormat as TextDrain, PlainDecorator, TermDecorator};
use std::fs::File;
use std::io;
use std::panic::RefUnwindSafe;
use std::sync::Arc;

type FilteredDrain<D> = LevelFilter<
    FieldFilteringDrain<FieldRedactFilterFactory, FieldFilteringDrain<FieldDedupFilterFactory, D>>,
>;

static HARNESS: OnceCell<LogHarness> = OnceCell::new();

static NOOP_HARNESS: Lazy<LogHarness> = Lazy::new(|| {
    let root_drain = Arc::new(Discard);
    let noop_log = Logger::root(Arc::clone(&root_drain), slog::o!());

    LogHarness {
        root_drain,
        root_log: Arc::new(parking_lot::RwLock::new(noop_log)),
        settings: Default::default(),
        log_scope_stack: Default::default(),
    }
});

pub(crate) struct LogHarness {
    pub(crate) root_log: SharedLog,
    pub(crate) root_drain: Arc<dyn SendSyncRefUnwindSafeDrain<Ok = (), Err = Never>>,
    pub(crate) settings: LoggingSettings,
    pub(crate) log_scope_stack: ScopeStack<SharedLog>,
}

impl LogHarness {
    pub(crate) fn get() -> &'static Self {
        HARNESS.get().unwrap_or(&NOOP_HARNESS)
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn override_for_testing(override_harness: Self) -> Result<(), ()> {
        HARNESS.set(override_harness).map_err(|_| ())
    }
}

// NOTE: Does nothing if logging has already been initialized in this process.
pub(crate) fn init(service_info: &ServiceInfo, settings: &LoggingSettings) -> BootstrapResult<()> {
    // Already initialized
    if HARNESS.get().is_some() {
        return Ok(());
    }

    // NOTE: OXY-178, default is 128 (https://docs.rs/slog-async/2.7.0/src/slog_async/lib.rs.html#251)
    const CHANNEL_SIZE: usize = 1024;

    let drain = match (&settings.output, &settings.format) {
        (LogOutput::Terminal, LogFormat::Text) => {
            let drain = TextDrain::new(TermDecorator::new().stdout().build())
                .build()
                .fuse();
            AsyncDrain::new(drain).chan_size(CHANNEL_SIZE).build()
        }
        (LogOutput::Terminal, LogFormat::Json) => {
            let drain = build_json_log_drain(io::stdout());
            AsyncDrain::new(drain).chan_size(CHANNEL_SIZE).build()
        }
        (LogOutput::File(file), LogFormat::Text) => {
            let drain = TextDrain::new(PlainDecorator::new(File::create(file)?))
                .build()
                .fuse();
            AsyncDrain::new(drain).chan_size(CHANNEL_SIZE).build()
        }
        (LogOutput::File(file), LogFormat::Json) => {
            let drain = build_json_log_drain(File::create(file)?);
            AsyncDrain::new(drain).chan_size(CHANNEL_SIZE).build()
        }
    };

    let root_drain = Arc::new(drain.fuse());
    let root_kv = slog::o!(
        "module" => FnValue(|record| {
            format!("{}:{}", record.module(), record.line())
        }),
        "version" => service_info.version,
    );

    let root_log = build_log_with_drain(settings, root_kv, Arc::clone(&root_drain));
    let harness = LogHarness {
        root_drain,
        root_log: Arc::new(parking_lot::RwLock::new(root_log)),
        settings: settings.clone(),
        log_scope_stack: Default::default(),
    };

    let _ = HARNESS.set(harness);

    Ok(())
}

pub(crate) fn apply_filters_to_drain<D>(
    drain: D,
    settings: &LoggingSettings,
) -> RateLimitingDrain<FilteredDrain<D>>
where
    D: Drain<Ok = (), Err = Never> + 'static,
{
    let drain = FieldFilteringDrain::new(drain, FieldDedupFilterFactory);
    let drain = FieldFilteringDrain::new(
        drain,
        FieldRedactFilterFactory::new(settings.redact_keys.clone()),
    );
    let drain = drain.filter_level(*settings.verbosity);

    RateLimitingDrain::new(drain, settings)
}

pub(crate) fn build_log_with_drain<D, K>(
    settings: &LoggingSettings,
    kv: OwnedKV<K>,
    drain: D,
) -> Logger
where
    D: SendSyncUnwindSafeDrain<Ok = (), Err = Never> + RefUnwindSafe + 'static,
    K: SendSyncRefUnwindSafeKV + 'static,
{
    let drain = apply_filters_to_drain(drain, settings);
    Logger::root(drain, kv)
}

fn build_json_log_drain<O>(output: O) -> Fuse<Json<O>>
where
    O: io::Write + Send + 'static,
{
    JsonDrain::new(output)
        .add_default_keys()
        .set_pretty(false)
        .build()
        .fuse()
}
