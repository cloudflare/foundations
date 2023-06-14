//! Predefined allow lists of syscalls for commonly used operations.

use super::{allow_list, ArgCmp};

const PR_SET_NAME: u64 = 15;

allow_list! {
    /// An allow list for basic tokio and Rust std library operations.
    pub static RUST_BASICS = [
        sigaltstack,
        getrandom,
        clone, // threads/rayon
        clone3, // since Rust 1.56.0
        futex,
        sched_yield,
        set_robust_list,
        sched_getaffinity,
        madvise, // memory allocation
        mprotect,
        prctl if [ ArgCmp::Equal { arg_idx: 0, value: PR_SET_NAME.into() } ] // tokio-runtime thread name
    ]
}

allow_list! {
    /// An allow list for basic service process operations.
    pub static SERVICE_BASICS = [
        ..RUST_BASICS,
        exit,
        exit_group,
        kill if [ ArgCmp::Equal { arg_idx: 0, value: std::process::id().into() } ],
        tgkill if [ ArgCmp::Equal { arg_idx: 0, value: std::process::id().into() } ],
        getpid,
        gettid,
        rt_sigprocmask,
        read,
        write,
        close,
        brk,
        mmap,
        munmap,
        mremap,
        fstat,
        newfstatat,
        lseek,
        rseq
    ]
}

allow_list! {
    /// An allow list for syscalls that are usually required by [epoll]-based async code.
    ///
    /// [epoll]: https://man7.org/linux/man-pages/man7/epoll.7.html
    pub static ASYNC = [
        #[cfg(target_arch = "x86_64")]
        epoll_wait,
        epoll_pwait,
        epoll_ctl,
        #[cfg(target_arch = "x86_64")]
        epoll_create,
        epoll_create1
    ]
}

allow_list! {
    /// An allow list for network socket API.
    ///
    /// Note that this allow list doesn't allow creation of new network endpoints
    /// (e.g. by using [`Syscall::listen`]).
    ///
    /// [`Syscall::listen`]: super::Syscall::listen
    pub static NET_SOCKET_API = [
        socket,
        connect,
        shutdown,
        accept,
        accept4,
        sendto,
        sendmsg,
        sendmmsg,
        recvfrom,
        recvmsg,
        recvmmsg,
        socketpair,
        setsockopt,
        getsockopt,
        bind
    ]
}

allow_list! {
    /// An allow list for [inotify]-based FS watch API.
    ///
    /// [inotify]: https://man7.org/linux/man-pages/man7/inotify.7.html
    pub static FS_WATCH = [
        #[cfg(target_arch = "x86_64")]
        inotify_init,
        inotify_init1,
        inotify_add_watch,
        inotify_rm_watch,
        getdents64
    ]
}
