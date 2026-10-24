// Run profiler initialization in a fresh process so each test controls jemalloc's startup state.
#[cfg(target_os = "linux")]
fn main() {
    use foundations::telemetry::MemoryProfiler;
    use foundations::telemetry::settings::MemoryProfilerSettings;
    use std::ffi::CStr;

    const PROF_ACTIVE: &CStr = cstr(b"prof.active\0");
    const PROF_LG_SAMPLE: &CStr = cstr(b"prof.lg_sample\0");

    let enabled = match std::env::args().nth(1).as_deref() {
        Some("enabled") => true,
        Some("disabled") => false,
        _ => panic!("expected `enabled` or `disabled` argument"),
    };

    let profiler = MemoryProfiler::get_or_init_with(&MemoryProfilerSettings {
        enabled,
        sample_interval: 17,
    })
    .expect("failed to initialize memory profiler");

    let active: bool = read_jemalloc_setting(PROF_ACTIVE);
    let sample_interval: usize = read_jemalloc_setting(PROF_LG_SAMPLE);

    println!(
        "profiler={} active={active} sample_interval={sample_interval}",
        profiler.is_some()
    );
}

#[cfg(target_os = "linux")]
fn read_jemalloc_setting<T: Copy>(name: &std::ffi::CStr) -> T {
    unsafe { tikv_jemalloc_ctl::raw::read(name.to_bytes_with_nul()) }
        .expect("failed to read jemalloc setting")
}

#[cfg(target_os = "linux")]
const fn cstr(bytes: &[u8]) -> &std::ffi::CStr {
    match std::ffi::CStr::from_bytes_until_nul(bytes) {
        Ok(value) => value,
        Err(_) => panic!("control name should be null-terminated"),
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {}
