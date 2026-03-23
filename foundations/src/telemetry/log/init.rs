use super::field_dedup::FieldDedupFilterFactory;
use super::field_filtering::{FieldFilteringDrain, FilterFactory};
use super::field_redact::FieldRedactFilterFactory;
use super::internal::{LoggerWithKvNestingTracking, SharedLog};

#[cfg(feature = "metrics")]
use crate::telemetry::log::log_volume::LogVolumeMetricsDrain;

use crate::telemetry::log::rate_limit::RateLimitingDrain;
use crate::telemetry::log::retry_writer::RetryPipeWriter;
use crate::telemetry::scope::ScopeStack;
use crate::telemetry::settings::{LogFormat, LogOutput, LogVerbosity, LoggingSettings};
use crate::{BootstrapResult, ServiceInfo};
use crossbeam_utils::CachePadded;
use slog::{
    Discard, Drain, FnValue, Fuse, Logger, Never, OwnedKV, SendSyncRefUnwindSafeDrain,
    SendSyncRefUnwindSafeKV,
};
use slog_async::{Async as AsyncDrain, AsyncGuard};
use slog_json::{Json as JsonDrain, Json};
use slog_term::{FullFormat as TextDrain, PlainDecorator, TermDecorator};
use std::fs::File;
use std::io;
use std::io::BufWriter;
use std::sync::{Arc, LazyLock, OnceLock};

type SharedDrain = Arc<dyn SendSyncRefUnwindSafeDrain<Ok = (), Err = Never>>;

// These singletons are accessed _very often_, and each access requires an atomic load to
// ensure initialization. Make sure nobody else invalidates our cache lines.
static HARNESS: CachePadded<OnceLock<LogHarness>> = CachePadded::new(OnceLock::new());

static NOOP_HARNESS: CachePadded<LazyLock<LogHarness>> = CachePadded::new(LazyLock::new(|| {
    let root_drain = Arc::new(Discard) as Arc<_>;
    let noop_log =
        LoggerWithKvNestingTracking::new(Logger::root_typed(Arc::clone(&root_drain), slog::o!()));

    LogHarness {
        root_drain,
        root_log: Arc::new(parking_lot::RwLock::new(noop_log)),
        settings: Default::default(),
        log_scope_stack: Default::default(),
    }
}));

pub(crate) struct LogHarness {
    pub(crate) root_log: SharedLog,
    pub(crate) root_drain: SharedDrain,
    pub(crate) settings: LoggingSettings,
    pub(crate) log_scope_stack: ScopeStack<SharedLog>,
}

impl LogHarness {
    pub(crate) fn get() -> &'static Self {
        HARNESS.get().unwrap_or_else(|| &**NOOP_HARNESS)
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn override_for_testing(override_harness: Self) -> Result<(), ()> {
        HARNESS.set(override_harness).map_err(|_| ())
    }
}

// Buffer output up to 4KiB. For JSON output, flush will be called for each record,
// even if the buffer isn't full.
const BUF_SIZE: usize = 4096;

// NOTE: Does nothing if logging has already been initialized in this process.
pub(crate) fn init(
    service_info: &ServiceInfo,
    settings: &LoggingSettings,
) -> BootstrapResult<Option<AsyncGuard>> {
    // Already initialized
    if HARNESS.get().is_some() {
        return Ok(None);
    }

    // NOTE: OXY-178, default is 128 (https://docs.rs/slog-async/2.7.0/src/slog_async/lib.rs.html#251)
    const CHANNEL_SIZE: usize = 1024;

    let (async_drain, async_guard) = match (&settings.output, &settings.format) {
        (output @ (LogOutput::Terminal | LogOutput::Stderr), LogFormat::Text) => {
            let decorator = if matches!(output, LogOutput::Terminal) {
                TermDecorator::new().stdout().build()
            } else {
                TermDecorator::new().stderr().build()
            };

            let drain = TextDrain::new(decorator).build().fuse();
            AsyncDrain::new(drain)
                .chan_size(CHANNEL_SIZE)
                .build_with_guard()
        }
        (output @ (LogOutput::Terminal | LogOutput::Stderr), LogFormat::Json) => {
            let writer = if matches!(output, LogOutput::Terminal) {
                stdout_writer_without_line_buffering()
            } else {
                stderr_writer_without_line_buffering()
            };
            let drain = build_json_log_drain(writer);
            AsyncDrain::new(drain)
                .chan_size(CHANNEL_SIZE)
                .build_with_guard()
        }
        (LogOutput::File(file_path), LogFormat::Text) => {
            let file = RetryPipeWriter::new(file_path.into())?;
            let drain = TextDrain::new(PlainDecorator::new(file)).build().fuse();
            AsyncDrain::new(drain)
                .chan_size(CHANNEL_SIZE)
                .build_with_guard()
        }
        (LogOutput::File(file_path), LogFormat::Json) => {
            let file = RetryPipeWriter::new(file_path.into())?;
            let buf = BufWriter::with_capacity(BUF_SIZE, file);
            let drain = build_json_log_drain(buf);
            AsyncDrain::new(drain)
                .chan_size(CHANNEL_SIZE)
                .build_with_guard()
        }
        #[cfg(feature = "tracing-rs-compat")]
        (LogOutput::TracingRsCompat, _) => AsyncDrain::new(tracing_slog::TracingSlogDrain {})
            .chan_size(CHANNEL_SIZE)
            .build_with_guard(),
    };

    let root_drain = wrap_root_drain(settings, async_drain.fuse());
    let root_kv = slog::o!(
        "module" => FnValue(|record| {
            format!("{}:{}", record.module(), record.line())
        }),
        "version" => service_info.version,
        "pid" => std::process::id(),
    );

    let root_log = build_log_with_drain(settings.verbosity, root_kv, Arc::clone(&root_drain));
    let harness = LogHarness {
        root_drain,
        root_log: Arc::new(parking_lot::RwLock::new(LoggerWithKvNestingTracking::new(
            root_log,
        ))),
        settings: settings.clone(),
        log_scope_stack: Default::default(),
    };

    let _ = HARNESS.set(harness);

    Ok(Some(async_guard))
}

/// Opens fd 1 directly and wraps with a [`BufWriter`] with [`BUF_SIZE`] capacity.
///
/// [`io::Stdout`] uses a [`io::LineWriter`] which may cause unnecessary flushing.
#[cfg(not(target_family = "windows"))]
fn stdout_writer_without_line_buffering() -> BufWriter<File> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let stdout = std::io::stdout();
    let stdout = unsafe { File::from_raw_fd(stdout.as_raw_fd()) };
    BufWriter::with_capacity(BUF_SIZE, stdout)
}

/// Opens fd 1 directly and wraps with a [`BufWriter`] with [`BUF_SIZE`] capacity.
///
/// [`io::Stdout`] uses a [`io::LineWriter`] which may cause unnecessary flushing.
#[cfg(target_family = "windows")]
fn stdout_writer_without_line_buffering() -> BufWriter<File> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle};

    let stdout = std::io::stdout();
    let stdout = unsafe { File::from_raw_handle(stdout.as_raw_handle()) };
    BufWriter::with_capacity(BUF_SIZE, stdout)
}

/// Opens fd 2 directly and wraps with a [`BufWriter`] with [`BUF_SIZE`] capacity.
#[cfg(not(target_family = "windows"))]
fn stderr_writer_without_line_buffering() -> BufWriter<File> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let stderr = std::io::stderr();
    let stderr = unsafe { File::from_raw_fd(stderr.as_raw_fd()) };
    BufWriter::with_capacity(BUF_SIZE, stderr)
}

/// Opens fd 2 directly and wraps with a [`BufWriter`] with [`BUF_SIZE`] capacity.
#[cfg(target_family = "windows")]
fn stderr_writer_without_line_buffering() -> BufWriter<File> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle};

    let stderr = std::io::stderr();
    let stderr = unsafe { File::from_raw_handle(stderr.as_raw_handle()) };
    BufWriter::with_capacity(BUF_SIZE, stderr)
}

pub(crate) fn wrap_root_drain<D>(settings: &LoggingSettings, drain: D) -> SharedDrain
where
    D: SendSyncRefUnwindSafeDrain<Ok = (), Err = Never> + 'static,
{
    let drain = drain
        .field_filter(FieldDedupFilterFactory)
        .field_filter(FieldRedactFilterFactory::new(settings.redact_keys.clone()));

    #[cfg(feature = "metrics")]
    if settings.log_volume_metrics.enabled {
        return drain.volume_metrics().rate_limit(settings).shared();
    }

    drain.rate_limit(settings).shared()
}

pub(crate) fn build_log_with_drain<K>(
    verbosity: LogVerbosity,
    kv: OwnedKV<K>,
    drain: SharedDrain,
) -> Logger
where
    K: SendSyncRefUnwindSafeKV + 'static,
{
    let drain = drain.filter_level(verbosity.into()).ignore_res();
    Logger::root(drain, kv)
}

fn build_json_log_drain<O>(output: O) -> Fuse<Json<O>>
where
    O: io::Write + Send + 'static,
{
    JsonDrain::new(output)
        .add_default_keys()
        .set_pretty(false)
        .set_flush(true)
        .build()
        .fuse()
}

/// [`Drain`] extension trait for easier layering.
trait DrainExt: Drain + Sized {
    /// Layers a [`FieldFilteringDrain`] on top of the current drain.
    fn field_filter<F: FilterFactory>(self, filter_factory: F) -> FieldFilteringDrain<F, Self> {
        FieldFilteringDrain::new(self, filter_factory)
    }

    /// Layers a [`LogVolumeMetricsDrain`] on top of the current drain.
    #[cfg(feature = "metrics")]
    fn volume_metrics(self) -> LogVolumeMetricsDrain<Self> {
        LogVolumeMetricsDrain::new(self)
    }

    /// Layers a [`RateLimitingDrain`] on top of the current drain.
    fn rate_limit(self, settings: &LoggingSettings) -> RateLimitingDrain<Self> {
        RateLimitingDrain::new(self, &settings.rate_limit)
    }

    /// Converts the current drain into an `Arc<dyn _>` for sharing between
    /// multiple loggers.
    fn shared(self) -> Arc<dyn SendSyncRefUnwindSafeDrain<Ok = Self::Ok, Err = Self::Err>>
    where
        Self: SendSyncRefUnwindSafeDrain + 'static,
    {
        Arc::new(self)
    }
}

impl<D: Drain + Sized> DrainExt for D {}
