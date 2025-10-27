use prometheus_client::encoding::text::{EncodeMetric, Encoder};
use prometheus_client::metrics::{MetricType, TypedMetric};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

mod sys;

static CPU_COUNT: LazyLock<usize> = LazyLock::new(|| {
    // SAFETY: `get_nprocs_conf` is always safe to call
    let cpus = unsafe { sys::get_nprocs_conf() };
    assert!(cpus > 0, "get_nprocs_conf returned `{cpus}`");
    cpus as _
});

fn sched_getcpu() -> std::io::Result<u32> {
    // SAFETY: `sched_getcpu` is always safe to call
    let res = unsafe { libc::sched_getcpu() };
    if res < 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(res as _)
}

#[repr(C, align(128))]
#[derive(Default)]
struct Padded<T>(T);

pub struct PerCpuCounter {
    // Ideally we would have a per-cpu allocator like librseq's mempool.
    // In lieu of that, we have to make do with cacheline padding.
    counters: Box<[Padded<AtomicU64>]>,
}

impl PerCpuCounter {
    /// Create a new [`PerCpuCounter`] instance.
    pub fn new() -> Self {
        let counters = (0..*CPU_COUNT).map(|_| Default::default()).collect();
        Self { counters }
    }

    /// Increase the [`PerCpuCounter`] by 1.
    #[inline]
    pub fn inc(&self) {
        self.inc_by(1)
    }

    /// Increase the [`PerCpuCounter`] by `v`.
    pub fn inc_by(&self, mut v: u64) {
        let Some(rseq_ptr) = sys::rseq_self() else {
            self.rmw_inc_by(v);
            return;
        };

        let mut cs = sys::rseq_cs {
            version: 0,
            flags: 0,
            start_ip: 0,
            post_commit_offset: 0,
            abort_ip: 0,
        };

        while v != 0 {
            // Optimistically calculate a pointer to the current CPU's counter
            // SAFETY: `sys::rseq_self` returns a valid, aligned pointer to the
            // thread-local `rseq` instance and we never create an `&mut rseq`.
            let cpu = unsafe { rseq_ptr.as_ref().cpu_id_start.load(Ordering::Relaxed) };
            let counter = &self.counters[cpu as usize].0;

            // SAFETY: We follow all rules established for `asm!` in the reference.
            unsafe {
                #[cfg(target_arch = "aarch64")]
                std::arch::asm!(
                    // Setup `cs`
                    "adr {tmp}, 5f",
                    "adr {tmp2}, 6f",
                    "sub {tmp2}, {tmp2}, {tmp}",
                    "stp {tmp}, {tmp2}, [{cs_ptr}, #8]",
                    "adr {tmp}, 7f",
                    "str {tmp}, [{cs_ptr}, #24]",
                    "str {cs_ptr}, [{rseq_ptr}, #8]",

                    // Restartable sequence
                    // Verify that our optimistic CPU lookup is valid
                    "5: ldr {tmp2:w}, [{rseq_ptr}, #4]",
                    "cmp {cpu:w}, {tmp2:w}",
                    "b.ne 7f",
                    // Increment the counter
                    "ldr {tmp}, [{counter}]",
                    "add {tmp}, {tmp}, {v}",
                    // Commit
                    "str {tmp}, [{counter}]",
                    "6: mov {v}, #0",
                    "b 7f",

                    // Abort handler is a noop, but we always need to clean up `rseq_cs`
                    ".int {RSEQ_SIG}",
                    "7: str xzr, [{rseq_ptr}, #8]",

                    rseq_ptr = in(reg) rseq_ptr.as_ptr(),
                    cs_ptr = in(reg) &mut cs,
                    cpu = in(reg) cpu,
                    counter = in(reg) counter,
                    v = inout(reg) v,
                    tmp = out(reg) _,
                    tmp2 = out(reg) _,
                    RSEQ_SIG = const sys::RSEQ_SIG,
                    options(nostack),
                );
                #[cfg(target_arch = "x86_64")]
                std::arch::asm!(
                    // Setup `cs`
                    "lea {tmp}, [rip + 5f]",
                    "mov qword ptr [{cs_ptr} + 8], {tmp}",
                    "neg {tmp}",
                    "mov qword ptr [{cs_ptr} + 16], {tmp}",
                    "lea {tmp}, [rip + 6f]",
                    "add qword ptr [{cs_ptr} + 16], {tmp}",
                    "lea {tmp}, [rip + 7f]",
                    "mov qword ptr [{cs_ptr} + 24], {tmp}",
                    "mov qword ptr [{rseq_ptr} + 8], {cs_ptr}",

                    // Restartable sequence
                    // Verify that our optimistic CPU lookup is valid
                    "5: cmp {cpu:e}, dword ptr [{rseq_ptr} + 4]",
                    "jne 7f",
                    // Increment the counter + commit
                    "add qword ptr [{counter}], {v}",
                    "6: xor {v:e}, {v:e}",
                    "jmp 7f",

                    // Abort handler is a noop, but we always need to clean up `rseq_cs`
                    "ud1 edi, [rip + {RSEQ_SIG}]",
                    "7: mov qword ptr [{rseq_ptr} + 8], 0",

                    rseq_ptr = in(reg) rseq_ptr.as_ptr(),
                    cs_ptr = in(reg) &mut cs,
                    cpu = in(reg) cpu,
                    counter = in(reg) counter,
                    v = inout(reg) v,
                    tmp = out(reg) _,
                    RSEQ_SIG = const sys::RSEQ_SIG,
                    options(nostack),
                );
            }
        }
    }

    /// Fallback for `inc_by` using atomic RMW. This is used if `rseq` is unavailable.
    #[cold]
    fn rmw_inc_by(&self, v: u64) {
        let cpu = sched_getcpu().expect("sched_getcpu failed");
        let counter = &self.counters[cpu as usize].0;
        counter.fetch_add(v, Ordering::Relaxed);
    }

    /// Get the current value of the [`PerCpuCounter`].
    pub fn get(&self) -> u64 {
        // Use wrapping arithmetic to emulate a single counter
        self.counters
            .iter()
            .map(|c| c.0.load(Ordering::Relaxed))
            .fold(0, u64::wrapping_add)
    }
}

impl fmt::Debug for PerCpuCounter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let total = self.get();
        f.debug_struct("PerCpuCounter")
            .field("value", &total)
            .finish()
    }
}

impl Default for PerCpuCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedMetric for PerCpuCounter {
    const TYPE: MetricType = MetricType::Counter;
}

impl EncodeMetric for PerCpuCounter {
    fn encode(&self, mut encoder: Encoder) -> std::io::Result<()> {
        let total = self.get();
        encoder
            .encode_suffix("total")?
            .no_bucket()?
            .encode_value(total)?
            .no_exemplar()
    }

    fn metric_type(&self) -> MetricType {
        Self::TYPE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn inc_and_get() {
        let counter = PerCpuCounter::new();
        assert_eq!(0, counter.get());

        counter.inc();
        assert_eq!(1, counter.get());

        counter.inc_by(199);
        assert_eq!(200, counter.get());
    }

    #[test]
    fn threaded_inc() {
        let counter = Arc::new(PerCpuCounter::new());
        assert_eq!(0, counter.get());

        let handles: Vec<_> = (0..5)
            .map(|_| {
                let counter = Arc::clone(&counter);
                std::thread::spawn(move || {
                    for _ in 0..1000 {
                        counter.inc();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("test thread paniced");
        }

        assert_eq!(5000, counter.get());

        // Test wrap-around: adding u64::MAX is equivalent to subtracing 1
        counter.inc_by(u64::MAX);
        assert_eq!(4999, counter.get());
    }
}
