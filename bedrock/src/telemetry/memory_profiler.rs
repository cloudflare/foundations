use crate::{BootstrapError, BootstrapResult, Result};
use anyhow::bail;
use once_cell::sync::{Lazy, OnceCell};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use tempfile::NamedTempFile;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::spawn_blocking;

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
struct Seal;

/// A safe interface for [jemalloc]'s memory profiling functionality.
///
/// [jemalloc]: https://github.com/jemalloc/jemalloc
pub struct MemoryProfiler(Seal);

impl MemoryProfiler {
    /// Creates a new profiler with the given sampling interval or returns a previously initialized
    /// profiler ignoring the passed sampling interval.
    ///
    /// `sample_interval` is a value between 0 and 64 which specifies the number of bytes of
    /// allocation activity between samples as `number_of_bytes = 2 ^ sample_interval`. Increasing
    /// the `sample_interval` decreases profile fidelity, but also decreases the computational
    /// overhead. The recommended default is `19` (2 ^ 19 = 512KiB).
    ///
    /// # Enabling profiling
    ///
    /// Note that profiling needs to be explicitly enabled by setting `_RJEM_MALLOC_CONF=prof:true`
    /// environment variable for the binary. Otherwise, this method will return `None`.
    pub fn get_or_init_with_sample_interval(
        sample_interval: u8,
    ) -> BootstrapResult<Option<&'static Self>> {
        const MAX_SAMPLE_INTERVAL: u8 = 64;

        // NOTE: https://github.com/jemalloc/jemalloc/blob/3e82f357bb218194df5ba1acee39cd6a7d6fe6f6/src/jemalloc.c#L1589
        if sample_interval > MAX_SAMPLE_INTERVAL {
            bail!("`sample_interval` value should be in the range [0, 64]");
        }

        PROFILER
            .get_or_try_init(|| init_profiler(sample_interval))
            .map(Option::as_ref)
    }

    /// Returns a heap profile.
    ///
    /// # Examples
    /// ```
    /// use bedrock::telemetry::MemoryProfiler;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let profiler = MemoryProfiler::get_or_init_with_sample_interval(19)
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

        let out_file = spawn_blocking(NamedTempFile::new).await??;

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

        let mut out_file = File::open(out_file_path).await?;
        let mut profile = Vec::new();

        out_file.read_to_end(&mut profile).await?;

        Ok(String::from_utf8(profile)?)
    }

    /// Returns heap statistics.
    ///
    /// # Examples
    /// ```
    /// use bedrock::telemetry::MemoryProfiler;
    ///
    /// let profiler = MemoryProfiler::get_or_init_with_sample_interval(19)
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

fn init_profiler(sample_interval: u8) -> BootstrapResult<Option<MemoryProfiler>> {
    if !control::profiling_enabled() {
        return Ok(None);
    }

    #[cfg(target_os = "linux")]
    control::write(control::BACKGROUND_THREAD, true).map_err(|e| {
        BootstrapError::new(e).context("failed to activate background thread collection")
    })?;

    control::write(control::PROF_RESET, sample_interval as u64)
        .map_err(|e| BootstrapError::new(e).context("failed to set sample interval"))?;

    control::write(control::PROF_ACTIVE, true)
        .map_err(|e| BootstrapError::new(e).context("failed to activate profiling"))?;

    Ok(Some(MemoryProfiler(Seal)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_interval_out_of_bounds() {
        assert!(MemoryProfiler::get_or_init_with_sample_interval(128).is_err());
    }
}
