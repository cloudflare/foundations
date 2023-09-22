use super::field_dedup::FieldDedupFilterFactory;
use super::field_filtering::FieldFilteringDrain;
use super::field_redact::FieldRedactFilterFactory;
use super::internal::SharedLog;
use crate::telemetry::log::rate_limit::RateLimitingDrain;
use crate::telemetry::scope::ScopeStack;
use crate::telemetry::settings::{LogFormat, LogOutput, LoggingSettings};
use crate::{BootstrapResult, ServiceInfo};
use once_cell::sync::{Lazy, OnceCell};
use slog::{Discard, Drain, FnValue, LevelFilter, Logger, Never, SendSyncRefUnwindSafeDrain};
use slog_async::Async as AsyncDrain;
use slog_json::Json as JsonDrain;
use slog_term::{Decorator, FullFormat as TextDrain, PlainDecorator, TermDecorator};
use std::fs::File;
use std::io;
use std::sync::Arc;

type FilteredDrain<D> = LevelFilter<
    FieldFilteringDrain<FieldRedactFilterFactory, FieldFilteringDrain<FieldDedupFilterFactory, D>>,
>;

static HARNESS: OnceCell<LogHarness> = OnceCell::new();

static NOOP_HARNESS: Lazy<LogHarness> = Lazy::new(|| {
    let noop_log = Logger::root(Discard, slog::o!());

    LogHarness {
        root_log: Arc::new(parking_lot::RwLock::new(noop_log)),
        settings: Default::default(),
        log_scope_stack: Default::default(),
    }
});

pub(crate) struct LogHarness {
    pub(crate) root_log: SharedLog,
    pub(crate) settings: LoggingSettings,
    pub(crate) log_scope_stack: ScopeStack<SharedLog>,
}

impl LogHarness {
    pub(crate) fn get() -> &'static Self {
        HARNESS.get().unwrap_or(&NOOP_HARNESS)
    }
}

// NOTE: Does nothing if logging has already been initialized in this process.
pub(crate) fn init(service_info: &ServiceInfo, settings: &LoggingSettings) -> BootstrapResult<()> {
    let root_log = build_log(service_info, settings)?;

    let harness = LogHarness {
        root_log: Arc::new(parking_lot::RwLock::new(root_log)),
        settings: settings.clone(),
        log_scope_stack: Default::default(),
    };

    let _ = HARNESS.set(harness);

    Ok(())
}

pub(crate) fn build_log(
    service_info: &ServiceInfo,
    settings: &LoggingSettings,
) -> BootstrapResult<Logger> {
    Ok(match (&settings.output, settings.format) {
        (LogOutput::Terminal, LogFormat::Text) => build_text_log(
            service_info,
            settings,
            TermDecorator::new().stdout().build(),
        ),
        (LogOutput::Terminal, LogFormat::Json) => {
            build_json_log(service_info, settings, io::stdout())
        }
        (LogOutput::File(file), LogFormat::Text) => build_text_log(
            service_info,
            settings,
            PlainDecorator::new(File::create(file)?),
        ),
        (LogOutput::File(file), LogFormat::Json) => {
            build_json_log(service_info, settings, File::create(file)?)
        }
    })
}

pub(crate) fn apply_filters_to_drain<D>(
    drain: D,
    settings: &LoggingSettings,
) -> RateLimitingDrain<FilteredDrain<D>>
where
    D: SendSyncRefUnwindSafeDrain<Ok = (), Err = Never> + Send + 'static,
{
    let drain = FieldFilteringDrain::new(drain, FieldDedupFilterFactory);
    let drain = FieldFilteringDrain::new(
        drain,
        FieldRedactFilterFactory::new(settings.redact_keys.clone()),
    );
    let drain = drain.filter_level(*settings.verbosity);

    RateLimitingDrain::new(drain, settings)
}

fn build_log_with_drain<D>(
    service_info: &ServiceInfo,
    settings: &LoggingSettings,
    drain: D,
) -> Logger
where
    D: Drain<Ok = (), Err = Never> + Send + 'static,
{
    // NOTE: OXY-178, default is 128 (https://docs.rs/slog-async/2.7.0/src/slog_async/lib.rs.html#251)
    const CHANNEL_SIZE: usize = 1024;

    let drain = AsyncDrain::new(drain)
        .chan_size(CHANNEL_SIZE)
        .build()
        .fuse();

    let drain = apply_filters_to_drain(drain, settings);

    Logger::root(
        drain,
        slog::o!(
            "module" => FnValue(|record| {
                format!("{}:{}", record.module(), record.line())
            }),
            "version" => service_info.version,
        ),
    )
}

fn build_text_log<D>(service_info: &ServiceInfo, settings: &LoggingSettings, decorator: D) -> Logger
where
    D: Decorator + Send + 'static,
{
    let drain = TextDrain::new(decorator).build().fuse();

    build_log_with_drain(service_info, settings, drain)
}

fn build_json_log<O>(service_info: &ServiceInfo, settings: &LoggingSettings, output: O) -> Logger
where
    O: io::Write + Send + 'static,
{
    let drain = JsonDrain::new(output)
        .add_default_keys()
        .set_pretty(false)
        .build()
        .fuse();

    build_log_with_drain(service_info, settings, drain)
}
