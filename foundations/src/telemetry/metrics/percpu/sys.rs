#![allow(
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    dead_code,
    unreachable_pub
)]

use std::ptr::NonNull;
use std::sync::atomic::{AtomicPtr, AtomicU32};

include!(concat!(env!("OUT_DIR"), "/percpu_sys.rs"));

#[repr(C, align(32))]
#[derive(Debug)]
pub struct rseq {
    pub cpu_id_start: AtomicU32,
    pub cpu_id: AtomicU32,
    pub rseq_cs: AtomicPtr<rseq_cs>,
    pub flags: __u32,
}

#[inline(always)]
pub fn rseq_self() -> Option<NonNull<rseq>> {
    use std::sync::atomic::{AtomicIsize, Ordering};
    /// Cached `__rseq_offset` to avoid libc GOT indirection. There are two special values:
    /// - `isize::MAX`: The static has not been initialized yet.
    /// - `isize::MIN`: glibc's rseq initialization failed or was disabled.
    static OFFSET: AtomicIsize = AtomicIsize::new(isize::MAX);

    #[cold]
    fn initialize_offset() -> isize {
        // SAFETY: any bit pattern is valid for isize
        let mut new_offset = unsafe { __rseq_offset };
        assert!(
            !matches!(new_offset, isize::MIN | isize::MAX),
            "__rseq_offset matches one of our special values",
            // If this was to trigger, static TLS would be >= 2^63 bytes
        );

        // SAFETY: any bit pattern is valid for c_uint
        let size = unsafe { __rseq_size };
        if size < 20 {
            // rseq initialization failed or rseq disabled
            new_offset = isize::MIN
        }

        OFFSET.store(new_offset, Ordering::Relaxed);
        new_offset
    }

    // No need to synchronize with other threads, they will all compute the same value
    let mut offset = OFFSET.load(Ordering::Relaxed);
    if offset == isize::MAX {
        offset = initialize_offset();
    }
    if offset == isize::MIN {
        return None;
    }

    let ptr: *mut rseq;
    // SAFETY: Computes `<thread_pointer> + __rseq_offset` and stores it in `ptr`
    unsafe {
        #[cfg(target_arch = "aarch64")]
        std::arch::asm!(
            "mrs {ptr}, TPIDR_EL0",
            "add {ptr}, {ptr}, {offset}",
            offset = in(reg) offset,
            ptr = out(reg) ptr,
            options(pure, readonly, preserves_flags, nostack)
        );
        #[cfg(target_arch = "x86_64")]
        std::arch::asm!(
            "mov {ptr}, qword ptr fs:[0]",
            "lea {ptr}, [{ptr} + {offset}]",
            offset = in(reg) offset,
            ptr = out(reg) ptr,
            options(pure, readonly, preserves_flags, nostack)
        );
    }

    debug_assert!(
        ptr.is_aligned(),
        "got non-aligned pointer via __rseq_offset"
    );

    // SAFETY: glibc guarantees that `<thread_pointer> + __rseq_offset`
    // points to a valid `rseq` allocation
    unsafe { Some(NonNull::new_unchecked(ptr)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::unnecessary_operation, clippy::identity_op)]
    const _: () = {
        ["Size of rseq"][::std::mem::size_of::<rseq>() - 32usize];
        ["Alignment of rseq"][::std::mem::align_of::<rseq>() - 32usize];
        ["Offset of field: rseq::cpu_id_start"]
            [::std::mem::offset_of!(rseq, cpu_id_start) - 0usize];
        ["Offset of field: rseq::cpu_id"][::std::mem::offset_of!(rseq, cpu_id) - 4usize];
        ["Offset of field: rseq::rseq_cs"][::std::mem::offset_of!(rseq, rseq_cs) - 8usize];
        ["Offset of field: rseq::flags"][::std::mem::offset_of!(rseq, flags) - 16usize];
    };

    #[test]
    fn rseq_valid() {
        let rseq = unsafe {
            rseq_self()
                .expect("Linux thread should have a struct rseq")
                .as_ref()
        };

        println!("{rseq:?}");
    }
}
