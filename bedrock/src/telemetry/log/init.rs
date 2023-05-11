use super::field_dedup::FieldDedupFilterFactory;
use super::field_filtering::FieldFilteringDrain;
use super::field_redact::FieldRedactFilterFactory;
use super::internal::SharedLog;
use super::settings::{LogFormat, LogOutput, LoggingSettings};
use crate::telemetry::context_stack::ContextStack;
use crate::BootstrapResult;
use once_cell::sync::{Lazy, OnceCell};
use slog::{Discard, Drain, FnValue, Logger, Never};
use slog_async::Async as AsyncDrain;
use slog_json::Json as JsonDrain;
use slog_term::{Decorator, FullFormat as TextDrain, PlainDecorator, TermDecorator};
use std::fs::File;
use std::io;
use std::sync::Arc;

static HARNESS: OnceCell<LogHarness> = OnceCell::new();

static NOOP_HARNESS: Lazy<LogHarness> = Lazy::new(|| {
    let noop_log = Logger::root(Discard, slog::o!());

    LogHarness {
        root_log: Arc::new(parking_lot::RwLock::new(noop_log)),
        settings: Default::default(),
        log_ctx_stack: Default::default(),
    }
});

pub(crate) struct LogHarness {
    pub(crate) root_log: SharedLog,
    pub(crate) settings: LoggingSettings,
    pub(crate) log_ctx_stack: ContextStack<SharedLog>,
}

impl LogHarness {
    pub(crate) fn get() -> &'static Self {
        HARNESS.get().unwrap_or(&NOOP_HARNESS)
    }
}

// NOTE: Does nothing if logging has already been initialized in this process.
// TODO rename and use in telemetry initializer when <https://jira.cfdata.org/browse/ROCK-9>
// is implemented
pub(crate) fn _init(settings: &LoggingSettings, package_version: String) -> BootstrapResult<()> {
    let root_log = build_log(settings, package_version)?;

    let harness = LogHarness {
        root_log: Arc::new(parking_lot::RwLock::new(root_log)),
        settings: settings.clone(),
        log_ctx_stack: Default::default(),
    };

    let _ = HARNESS.set(harness);

    Ok(())
}

pub(crate) fn build_log(
    settings: &LoggingSettings,
    package_version: String,
) -> BootstrapResult<Logger> {
    Ok(match (&settings.output, settings.format) {
        (LogOutput::Terminal, LogFormat::Text) => build_text_log(
            settings,
            package_version,
            TermDecorator::new().stdout().build(),
        ),
        (LogOutput::Terminal, LogFormat::Json) => {
            build_json_log(settings, package_version, io::stdout())
        }
        (LogOutput::File(file), LogFormat::Text) => build_text_log(
            settings,
            package_version,
            PlainDecorator::new(File::create(file)?),
        ),
        (LogOutput::File(file), LogFormat::Json) => {
            build_json_log(settings, package_version, File::create(file)?)
        }
    })
}

fn build_log_with_drain<D>(settings: &LoggingSettings, package_version: String, drain: D) -> Logger
where
    D: Drain<Ok = (), Err = Never> + Send + 'static,
{
    // NOTE: OXY-178, default is 128 (https://docs.rs/slog-async/2.7.0/src/slog_async/lib.rs.html#251)
    const CHANNEL_SIZE: usize = 1024;

    let drain = FieldFilteringDrain::new(drain, FieldDedupFilterFactory);

    let drain = FieldFilteringDrain::new(
        drain,
        FieldRedactFilterFactory::new(settings.redact_keys.clone()),
    );

    let drain = AsyncDrain::new(drain)
        .chan_size(CHANNEL_SIZE)
        .build()
        .filter_level(*settings.verbosity)
        .fuse();

    Logger::root(
        drain,
        slog::o!(
            "module" => FnValue(|record| {
                format!("{}:{}", record.module(), record.line())
            }),
            "version" => package_version,
        ),
    )
}

fn build_text_log<D>(settings: &LoggingSettings, package_version: String, decorator: D) -> Logger
where
    D: Decorator + Send + 'static,
{
    let drain = TextDrain::new(decorator).build().fuse();

    build_log_with_drain(settings, package_version, drain)
}

fn build_json_log<O>(settings: &LoggingSettings, package_version: String, output: O) -> Logger
where
    O: io::Write + Send + 'static,
{
    let drain = JsonDrain::new(output)
        .add_default_keys()
        .set_pretty(false)
        .build()
        .fuse();

    build_log_with_drain(settings, package_version, drain)
}
