use super::settings::MemoryProfilerSettings;
use crate::utils::feature_use;
use crate::{BootstrapError, BootstrapResult, Result};
use anyhow::bail;
use once_cell::sync::{Lazy, OnceCell};
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::Read;
use std::os::raw::c_char;
use std::thread;
use tempfile::NamedTempFile;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::spawn_blocking;

feature_use!(cfg(feature = "security"), {
    use crate::security::common_syscall_allow_lists::SERVICE_BASICS;
    use crate::security::{allow_list, enable_syscall_sandboxing, ViolationAction};
});

static PROFILER: OnceCell<Option<MemoryProfiler>> = OnceCell::new();
static PROFILING_IN_PROGRESS_LOCK: Lazy<AsyncMutex<()>> = Lazy::new(Default::default);

mod control {
    use super::*;

    pub(super) const OPT_PROF: &CStr = cstr(b"opt.prof\0");
    pub(super) const PROF_DUMP: &CStr = cstr(b"prof.dump\0");
    pub(super) const PROF_ACTIVE: &CStr = cstr(b"prof.active\0");
    pub(super) const PROF_RESET: &CStr = cstr(b"prof.reset\0");

    #[cfg(target_os = "linux")]
    pub(super) const BACKGROUND_THREAD: &CStr = cstr(b"background_thread\0");

    // NOTE: safe wrappers that also guarantee that control name is a null-terminated C string.
    pub(super) fn write<T>(name: &CStr, value: T) -> tikv_jemalloc_ctl::Result<()> {
        unsafe { tikv_jemalloc_ctl::raw::write(name.to_bytes_with_nul(), value) }
    }

    pub(super) fn profiling_enabled() -> bool {
        unsafe { tikv_jemalloc_ctl::raw::read(OPT_PROF.to_bytes_with_nul()) }
            .expect("jemalloc must be compiled with profiling enabled")
    }

    const fn cstr(bytes: &[u8]) -> &CStr {
        match CStr::from_bytes_until_nul(bytes) {
            Ok(s) => s,
            Err(_) => panic!("control name should be null-terminated"),
        }
    }
}

// NOTE: prevent direct construction by the external code.
#[derive(Copy, Clone)]
struct Seal;

/// A safe interface for [jemalloc]'s memory profiling functionality.
///
/// [jemalloc]: https://github.com/jemalloc/jemalloc
#[derive(Copy, Clone)]
pub struct MemoryProfiler {
    _seal: Seal,

    #[cfg(feature = "security")]
    sandbox_profiling_syscalls: bool,
}

impl MemoryProfiler {
    /// Creates a new profiler with the given settings or returns a previously initialized
    /// profiler ignoring the settings.
    ///
    /// # Enabling profiling
    ///
    /// Note that profiling needs to be explicitly enabled by setting `_RJEM_MALLOC_CONF=prof:true`
    /// environment variable for the binary and with [`MemoryProfilerSettings::enabled`] being set
    /// to `true`. Otherwise, this method will return `None`.
    pub fn get_or_init_with(settings: &MemoryProfilerSettings) -> BootstrapResult<Option<Self>> {
        const MAX_SAMPLE_INTERVAL: u8 = 64;

        // NOTE: https://github.com/jemalloc/jemalloc/blob/3e82f357bb218194df5ba1acee39cd6a7d6fe6f6/src/jemalloc.c#L1589
        if settings.sample_interval > MAX_SAMPLE_INTERVAL {
            bail!("`sample_interval` value should be in the range [0, 64]");
        }

        PROFILER
            .get_or_try_init(|| init_profiler(settings))
            .copied()
    }

    /// Returns a heap profile.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::MemoryProfiler;
    /// use foundations::telemetry::settings::MemoryProfilerSettings;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let settings = MemoryProfilerSettings {
    ///         enabled: true,
    ///         ..Default::default()
    ///     };
    ///
    ///     let profiler = MemoryProfiler::get_or_init_with(&settings)
    ///         .unwrap()
    ///         .expect("profiling should be enabled via `_RJEM_MALLOC_CONF=prof:true` env var");
    ///
    ///     let profile = profiler.heap_profile().await.unwrap();
    ///
    ///     assert!(profile.contains("MAPPED_LIBRARIES"));
    /// }
    /// ```
    pub async fn heap_profile(&self) -> Result<String> {
        // NOTE: we use tokio mutex here, so we can hold the lock across `await` points.
        let Ok(_lock) = PROFILING_IN_PROGRESS_LOCK.try_lock() else {
            return Err("profiling is already in progress".into());
        };

        #[cfg(feature = "security")]
        let sandbox_profiling_syscalls = self.sandbox_profiling_syscalls;

        let collector_thread = thread::spawn(move || {
            #[cfg(feature = "security")]
            if sandbox_profiling_syscalls {
                sandbox_jemalloc_syscalls()?;
            }

            collect_heap_profile()
        });

        spawn_blocking(move || {
            collector_thread
                .join()
                .map_err(|_| "heap profile collector thread panicked")?
        })
        .await?
    }

    /// Returns heap statistics.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::MemoryProfiler;
    /// use foundations::telemetry::settings::MemoryProfilerSettings;
    ///
    /// let settings = MemoryProfilerSettings {
    ///     enabled: true,
    ///     ..Default::default()
    /// };
    ///
    /// let profiler = MemoryProfiler::get_or_init_with(&settings)
    ///     .unwrap()
    ///     .expect("profiling should be enabled via `_RJEM_MALLOC_CONF=prof:true` env var");
    ///
    /// let stats = profiler.heap_stats().unwrap();
    ///
    /// assert!(stats.contains("Allocated"));
    /// ```
    pub fn heap_stats(&self) -> Result<String> {
        let mut stats = Vec::new();

        tikv_jemalloc_ctl::stats_print::stats_print(&mut stats, Default::default())?;

        Ok(String::from_utf8(stats)?)
    }
}

fn init_profiler(settings: &MemoryProfilerSettings) -> BootstrapResult<Option<MemoryProfiler>> {
    if !settings.enabled || !control::profiling_enabled() {
        return Ok(None);
    }

    control::write(control::BACKGROUND_THREAD, true).map_err(|e| {
        BootstrapError::new(e).context("failed to activate background thread collection")
    })?;

    control::write(control::PROF_RESET, settings.sample_interval as u64)
        .map_err(|e| BootstrapError::new(e).context("failed to set sample interval"))?;

    control::write(control::PROF_ACTIVE, true)
        .map_err(|e| BootstrapError::new(e).context("failed to activate profiling"))?;

    Ok(Some(MemoryProfiler {
        _seal: Seal,

        #[cfg(feature = "security")]
        sandbox_profiling_syscalls: settings.sandbox_profiling_syscalls,
    }))
}

fn collect_heap_profile() -> Result<String> {
    let out_file = NamedTempFile::new()?;

    let out_file_path = out_file
        .path()
        .to_str()
        .ok_or("failed to obtain heap profile output file path")?;

    let mut out_file_path_c_str = CString::new(out_file_path)?.into_bytes_with_nul();
    let out_file_path_ptr = out_file_path_c_str.as_mut_ptr() as *mut c_char;

    control::write(control::PROF_DUMP, out_file_path_ptr).map_err(|e| {
        format!(
            "failed to dump jemalloc heap profile to {:?}: {}",
            out_file_path, e
        )
    })?;

    let mut profile = Vec::new();

    File::open(out_file_path)?.read_to_end(&mut profile)?;

    Ok(String::from_utf8(profile)?)
}

#[cfg(feature = "security")]
fn sandbox_jemalloc_syscalls() -> Result<()> {
    #[cfg(target_arch = "x86_64")]
    allow_list! {
        static ALLOWED_SYSCALLS = [
            ..SERVICE_BASICS,
            // PXY-41: Required to call Instant::now from parking-lot.
            clock_gettime,
            openat,
            creat,
            unlink
        ]
    }

    #[cfg(target_arch = "aarch64")]
    allow_list! {
        static ALLOWED_SYSCALLS = [
            ..SERVICE_BASICS,
            clock_gettime,
            openat,
            unlinkat
        ]
    }

    enable_syscall_sandboxing(ViolationAction::KillProcess, &ALLOWED_SYSCALLS)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::settings::MemoryProfilerSettings;

    #[test]
    fn sample_interval_out_of_bounds() {
        assert!(MemoryProfiler::get_or_init_with(&MemoryProfilerSettings {
            enabled: true,
            sample_interval: 128,
            ..Default::default()
        })
        .is_err());
    }

    // NOTE: `heap_profile` uses raw pointers, the test ensures that it doesn't affect the returned future
    fn _assert_heap_profile_fut_is_send() {
        fn is_send<T: Send>(_t: T) {}

        is_send(
            MemoryProfiler::get_or_init_with(&Default::default())
                .unwrap()
                .unwrap()
                .heap_profile(),
        );
    }
}
