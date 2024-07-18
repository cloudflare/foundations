use super::settings::MemoryProfilerSettings;
use crate::utils::feature_use;
use crate::{BootstrapError, BootstrapResult, Result};
use anyhow::anyhow;
use anyhow::bail;
use once_cell::sync::OnceCell;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::Read;
use std::os::raw::c_char;
use std::sync::mpsc::{self};
use std::sync::{Arc, Mutex};
use std::thread::{spawn, JoinHandle};
use tempfile::NamedTempFile;
use tokio::sync::oneshot;

feature_use!(cfg(feature = "security"), {
    use crate::security::common_syscall_allow_lists::SERVICE_BASICS;
    use crate::security::{allow_list, enable_syscall_sandboxing, ViolationAction};
});

static PROFILER: OnceCell<Option<MemoryProfiler>> = OnceCell::new();

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
#[derive(Clone)]
pub struct MemoryProfiler {
    _seal: Seal,

    request_heap_profile: mpsc::Sender<oneshot::Sender<anyhow::Result<String>>>,
    _heap_profiling_thread_handle: Arc<Mutex<JoinHandle<()>>>,
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
            .cloned()
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
        let (response_sender, response_receiver) = oneshot::channel();
        self.request_heap_profile.send(response_sender)?;

        Ok(response_receiver.await??)
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

    let (request_sender, request_receiver) = mpsc::channel();

    #[cfg(feature = "security")]
    let (setup_complete_sender, setup_complete_receiver) = mpsc::channel();

    let sandbox_profiling_syscalls = settings.sandbox_profiling_syscalls;
    let heap_profile_thread_handle = spawn(move || {
        heap_profile_thread(
            request_receiver,
            #[cfg(feature = "security")]
            setup_complete_sender,
            sandbox_profiling_syscalls,
        )
    });

    #[cfg(feature = "security")]
    receive_profiling_thread_setup_msg(setup_complete_receiver)?;

    control::write(control::BACKGROUND_THREAD, true).map_err(|e| {
        BootstrapError::new(e).context("failed to activate background thread collection")
    })?;

    control::write(control::PROF_RESET, settings.sample_interval as u64)
        .map_err(|e| BootstrapError::new(e).context("failed to set sample interval"))?;

    control::write(control::PROF_ACTIVE, true)
        .map_err(|e| BootstrapError::new(e).context("failed to activate profiling"))?;

    Ok(Some(MemoryProfiler {
        _seal: Seal,

        request_heap_profile: request_sender,
        _heap_profiling_thread_handle: Arc::new(Mutex::new(heap_profile_thread_handle)),
    }))
}

#[cfg(feature = "security")]
fn receive_profiling_thread_setup_msg(
    setup_complete_receiver: mpsc::Receiver<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    match setup_complete_receiver.recv() {
        Ok(Ok(())) => {}
        Ok(Err(setup_err)) => {
            return Err(setup_err);
        }
        Err(std::sync::mpsc::RecvError) => {
            bail!("Profiling thread disconnected before finishing setup")
        }
    }

    Ok(())
}

fn heap_profile_thread(
    receive_request: mpsc::Receiver<oneshot::Sender<anyhow::Result<String>>>,
    #[cfg(feature = "security")] setup_complete: mpsc::Sender<anyhow::Result<()>>,
    sandbox_profiling_syscalls: bool,
) {
    #[cfg(feature = "security")]
    if sandbox_profiling_syscalls {
        if let Err(_) = setup_complete.send(sandbox_jemalloc_syscalls()) {
            return;
        }
    }

    while let Ok(send_response) = receive_request.recv() {
        if let Err(_) = send_response.send(collect_heap_profile()) {
            break;
        }
    }
}

fn collect_heap_profile() -> anyhow::Result<String> {
    let out_file = NamedTempFile::new()?;

    let out_file_path = out_file
        .path()
        .to_str()
        .ok_or(anyhow!("failed to obtain heap profile output file path"))?;

    let mut out_file_path_c_str = CString::new(out_file_path)?.into_bytes_with_nul();
    let out_file_path_ptr = out_file_path_c_str.as_mut_ptr() as *mut c_char;

    control::write(control::PROF_DUMP, out_file_path_ptr).map_err(|e| {
        anyhow!(
            "failed to dump jemalloc heap profile to {:?}: {}",
            out_file_path,
            e
        )
    })?;

    let mut profile = Vec::new();

    File::open(out_file_path)?.read_to_end(&mut profile)?;

    Ok(String::from_utf8(profile)?)
}

#[cfg(feature = "security")]
fn sandbox_jemalloc_syscalls() -> anyhow::Result<()> {
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
    use crate::{
        security::common_syscall_allow_lists::ASYNC, telemetry::settings::MemoryProfilerSettings,
    };

    #[test]
    fn sample_interval_out_of_bounds() {
        assert!(MemoryProfiler::get_or_init_with(&MemoryProfilerSettings {
            enabled: true,
            sample_interval: 128,
            ..Default::default()
        })
        .is_err());
    }

    #[tokio::test]
    async fn profile_heap_with_profiling_sandboxed_after_previous_seccomp_init() {
        let profiler = MemoryProfiler::get_or_init_with(&MemoryProfilerSettings {
            enabled: true,
            sandbox_profiling_syscalls: true,
            ..Default::default()
        })
        .unwrap()
        .unwrap_or_else(|| {
            panic!("profiling should be enabled via `_RJEM_MALLOC_CONF=prof:true` env var");
        });

        allow_list! {
           static ALLOW_PROFILING = [
                ..SERVICE_BASICS,
                ..ASYNC
           ]
        }
        enable_syscall_sandboxing(ViolationAction::KillProcess, &ALLOW_PROFILING).unwrap();

        let profile = profiler.heap_profile().await.unwrap();
        assert!(!profile.is_empty());
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
