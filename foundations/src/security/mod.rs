//! Security-related features.
//!
//! # Syscall sandboxing
//!
//! [seccomp] is a Linux kernel's syscall sandboxing feature. It allows to set up hooks for the
//! syscalls that application is using and perform certain actions on it, such as blocking or
//! logging. As an effect, providing an additional fence from attacks like [arbitrary code execution].
//!
//! seccomp filtering is applied to a thread in which [`enable_syscall_sandboxing`] was called and
//! all the threads spawned by this thread. Therefore, enabling seccomp early in the `main` function
//! enables it for the whole proccess.
//!
//! All the syscalls are considered to be a security violation by default, with [`ViolationAction`]
//! being performed when syscall is encountered. Application need to provide a list of exception
//! [`Rule`]s to [`enable_syscall_sandboxing`] function for syscalls that it considers safe to use.
//!
//! The crate provides a few [`common_syscall_allow_lists`] to simplify configuration.
//!
//! Foundations compiles and statically links with [libseccomp], so it doesn't require the lib to be
//! installed.
//!
//! # Simple case [Spectre] mitigation for x86_64 processors
//!
//! One of the simplest Spectre attack vectors it to use x86_64's [time stamp counter]. foundations
//! provides [`forbid_x86_64_cpu_cycle_counter`] method that dissallows the usage of the
//! counter in the process, so any attempts to use the counter by malicious code will cause process
//! termination.
//!
//! [seccomp]: https://man7.org/linux/man-pages/man2/seccomp.2.html
//! [arbitrary code execution]: https://en.wikipedia.org/wiki/Arbitrary_code_execution
//! [libseccomp]: https://github.com/seccomp/libseccomp
//! [Spectre]: https://en.wikipedia.org/wiki/Spectre_(security_vulnerability)
//! [time stamp counter]: https://en.wikipedia.org/wiki/Time_Stamp_Counter

pub mod common_syscall_allow_lists;
mod internal;
mod syscalls;

#[allow(
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    dead_code,
    unreachable_pub
)]
mod sys {
    include!(concat!(env!("OUT_DIR"), "/security_sys.rs"));
}

use self::internal::RawRule;
use crate::{BootstrapError, BootstrapResult};
use anyhow::{anyhow, bail};
use std::fmt::Display;
use sys::PR_GET_SECCOMP;

pub use self::syscalls::Syscall;

/// A raw OS error code to be returned by [`Rule::ReturnError`].
pub type RawOsErrorNum = u16;

/// An action to be taken on seccomp sandbox violation.
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ViolationAction {
    /// Kill the process.
    ///
    /// Note that even though seccomp API allows to kill individual threads, Foundations doesn't
    /// expose this action as killing threads without unwinding [can cause UB in Rust].
    ///
    /// [can cause UB in Rust]: https://github.com/rust-lang/unsafe-code-guidelines/issues/211
    KillProcess = sys::SCMP_ACT_KILL_PROCESS,

    /// Allow the syscalls, but also log them in [sysctl] logs.
    ///
    /// The logs can be examined by running:
    /// ```sh
    /// sysctl -n kernel.seccomp.actions_logged
    /// ```
    ///
    /// [sysctl]: https://man7.org/linux/man-pages/man8/sysctl.8.html
    AllowAndLog = sys::SCMP_ACT_LOG,
}

/// A value to compare syscall arguments with in [comparators].
///
/// The value can be either a numeric or a reference `'static` value.
///
/// [comparators]: ArgCmp
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ArgCmpValue(u64);

impl ArgCmpValue {
    /// Constructs a value from a given static reference.
    pub fn from_static<T>(val: &'static T) -> Self {
        Self(val as *const T as u64)
    }
}

impl<T: Into<u64>> From<T> for ArgCmpValue {
    fn from(val: T) -> Self {
        Self(val.into())
    }
}

/// Errors that can be produced when initializing sandboxing using
/// [`enable_syscall_sandboxing`]
#[derive(Debug)]
pub enum SandboxingInitializationError {
    /// Sandboxing has already been initialized on the current thread
    /// with the given [`SeccompMode`]
    AlreadyInitialized(SeccompMode),

    /// Some other error occurred during initialization
    Other(BootstrapError),
}

impl Display for SandboxingInitializationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxingInitializationError::AlreadyInitialized(mode) => write!(
                f,
                "seccomp has already been initialized and is in mode {:?}",
                mode
            ),
            SandboxingInitializationError::Other(e) => e.fmt(f),
        }
    }
}

impl From<BootstrapError> for SandboxingInitializationError {
    fn from(err: BootstrapError) -> Self {
        SandboxingInitializationError::Other(err)
    }
}

impl From<SandboxingInitializationError> for BootstrapError {
    fn from(value: SandboxingInitializationError) -> Self {
        match value {
            SandboxingInitializationError::Other(e) => e,
            e @ SandboxingInitializationError::AlreadyInitialized(_) => anyhow!("{}", e),
        }
    }
}

/// Syscall argument comparators to be used in [`Rule`].
///
/// Argument comparators add additional filtering layer to rules allowing to compare syscall's
/// argument with the provided value and apply the exception rule only if the comparisson is
/// successful.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ArgCmp {
    /// Checks that argument is not equal to the provided value.
    NotEqual {
        /// The index of the argument.
        arg_idx: u32,

        /// Value to compare the argument with (can be a raw pointer).
        value: ArgCmpValue,
    },

    /// Checks that argument is less than the provided value.
    LessThan {
        /// The index of the argument.
        arg_idx: u32,

        /// Value to compare the argument with (can be a raw pointer).
        value: ArgCmpValue,
    },

    /// Checks that argument is less than or equal to the provided value.
    LessThanOrEqual {
        /// The index of the argument.
        arg_idx: u32,

        /// Value to compare the argument with (can be a raw pointer).
        value: ArgCmpValue,
    },

    /// Checks that argument is equal to the provided value.
    Equal {
        /// The index of the argument.
        arg_idx: u32,

        /// Value to compare the argument with (can be a raw pointer).
        value: ArgCmpValue,
    },

    /// Checks that argument is greater than or equal to the provided value.
    GreaterThanOrEqual {
        /// The index of the argument.
        arg_idx: u32,

        /// Value to compare the argument with (can be a raw pointer).
        value: ArgCmpValue,
    },

    /// Checks that argument is greater than the provided value.
    GreaterThan {
        /// The index of the argument.
        arg_idx: u32,

        /// Value to compare the argument with (can be a raw pointer).
        value: ArgCmpValue,
    },

    /// Checks that argument is equal to the provided value after application of the provided
    /// bitmask.
    EqualMasked {
        /// The index of the argument.
        arg_idx: u32,

        /// The bitmask to be applied to the argument before comparison.
        mask: u64,

        /// Value to compare the masked argument with.
        value: ArgCmpValue,
    },
}

/// A syscall exception rule to be provided to [`enable_syscall_sandboxing`].
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    /// Allows a syscall.
    ///
    /// [`allow_list`] macros provides a convenient way of constructing allow rules.
    ///
    /// # Examples
    ///
    /// Allow syscalls, required for [`std::process::exit`] to work, but allow only `0` status code,
    /// so this fails:
    /// ```should_panic
    /// use foundations::security::{
    ///     enable_syscall_sandboxing, ArgCmp, ViolationAction, Rule, Syscall, allow_list
    /// };
    /// use foundations::security::common_syscall_allow_lists::RUST_BASICS;
    /// use std::panic;
    /// use std::thread;
    /// use std::process;
    ///
    /// // Allows process exit only if the status code is 0.
    /// allow_list! {
    ///     static PROCESS_EXIT_ALLOWED = [
    ///         ..RUST_BASICS,
    ///         munmap,
    ///         exit_group
    ///     ]
    /// }
    ///
    /// // NOTE: `Rule::Allow` is used directly here only for the demonstration purposes,
    /// // in most cases it's more convenient to use the `allow_list!` macros as above.
    /// let mut rules = vec![
    ///     Rule::Allow(
    ///         Syscall::exit,
    ///         vec![ArgCmp::Equal { arg_idx: 0, value: 0u64.into() }]
    ///     )
    /// ];
    ///
    /// rules.extend_from_slice(&PROCESS_EXIT_ALLOWED);
    ///
    /// enable_syscall_sandboxing(ViolationAction::KillProcess, &rules).unwrap();
    ///
    /// process::exit(1);
    /// ```
    ///
    /// Same as the above but this time exit with `0` status code, so this works:
    /// ```
    /// use foundations::security::{
    ///     enable_syscall_sandboxing, ArgCmp, ViolationAction, Rule, Syscall, allow_list
    /// };
    /// use foundations::security::common_syscall_allow_lists::RUST_BASICS;
    /// use std::panic;
    /// use std::thread;
    /// use std::process;
    ///
    /// // Allows process exit only if the status code is 0.
    /// allow_list! {
    ///     static PROCESS_EXIT_ALLOWED = [
    ///         ..RUST_BASICS,
    ///         munmap,
    ///         exit_group
    ///     ]
    /// }
    ///
    /// // NOTE: `Rule::Allow` is used directly here only for the demonstration purposes,
    /// // in most cases it's more convenient to use the `allow_list!` macros as above.
    /// let mut rules = vec![
    ///     Rule::Allow(
    ///         Syscall::exit,
    ///         vec![ArgCmp::Equal { arg_idx: 0, value: 0u64.into() }]
    ///     )
    /// ];
    ///
    /// rules.extend_from_slice(&PROCESS_EXIT_ALLOWED);
    ///
    /// enable_syscall_sandboxing(ViolationAction::KillProcess, &PROCESS_EXIT_ALLOWED).unwrap();
    ///
    /// process::exit(0);
    /// ```
    Allow(Syscall, Vec<ArgCmp>),

    /// Same as [`Rule::Allow`], but also logs the syscall in [sysctl] logs.
    ///
    /// The logs can be examined by running:
    /// ```sh
    /// sysctl -n kernel.seccomp.actions_logged
    /// ```
    ///
    /// [sysctl]: https://man7.org/linux/man-pages/man8/sysctl.8.html
    AllowAndLog(Syscall, Vec<ArgCmp>),

    /// Forces syscall to return the provided error code.
    ///
    /// # Examples
    ///
    /// ```
    /// use foundations::security::{
    ///     enable_syscall_sandboxing, ViolationAction, allow_list, Rule, Syscall
    /// };
    /// use foundations::security::common_syscall_allow_lists::{SERVICE_BASICS, NET_SOCKET_API};
    /// use std::net::TcpListener;
    /// use std::panic;
    /// use std::thread;
    /// use std::io;
    ///
    /// const EPERM: u16 = 1;
    ///
    /// let mut rules = vec![Rule::ReturnError(Syscall::listen, EPERM, vec![])];
    ///
    /// rules.extend_from_slice(&SERVICE_BASICS);
    /// rules.extend_from_slice(&NET_SOCKET_API);
    ///
    /// enable_syscall_sandboxing(ViolationAction::KillProcess, &rules).unwrap();
    ///
    /// let err = TcpListener::bind("127.0.0.1:0").unwrap_err();
    ///
    /// assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    /// ```
    ReturnError(Syscall, RawOsErrorNum, Vec<ArgCmp>),
}

/// See [PR_GET_SECCOMP]
///
/// [PR_GET_SECCOMP]: https://linuxman7.org/linux/man-pages/man2/PR_GET_SECCOMP.2const.html
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum SeccompMode {
    /// The current thread is not in secure computing mode
    None = 0,

    /// The current thread is in strict secure computing mode.
    /// Unused in practice since the syscall to get the value
    /// would kill your process in strict computing mode.
    _Strict = 1,

    /// The current thread is in filter mode
    Filter = 2,
}

/// Enables [seccomp]-based syscall sandboxing in the current thread and all the threads spawned
/// by it, if the current thread does not already have sandboxing enabled.
///
/// Calling the function early in the `main` function effectively enables seccomp for the whole
/// process.
///
/// If seccomp encounters a syscall that is not in the `exception_rules` list it performs the
/// provided [`ViolationAction`]. [`allow_list`] macro can be used to conveniently construct lists
/// of allowed syscalls. In addition, the crate provides [`common_syscall_allow_lists`] that can be
/// merged into the user-provided allow lists.
///
/// To setup sandboxing on a thread has already been sandboxed, use
/// [`enable_syscall_sandboxing_ignore_existing`] on a thread allowing the `prctl` and `seccomp`
/// syscalls.
///
/// [seccomp]: https://man7.org/linux/man-pages/man2/seccomp.2.html
///
/// # Examples
/// Forbid all the syscalls, so this fails:
/// ```should_panic
/// use foundations::security::{enable_syscall_sandboxing, ViolationAction, allow_list};
/// use foundations::security::common_syscall_allow_lists::SERVICE_BASICS;
/// use std::net::TcpListener;
/// use std::panic;
/// use std::thread;
///
/// enable_syscall_sandboxing(ViolationAction::KillProcess, &vec![]).unwrap();
///
/// let _ = TcpListener::bind("127.0.0.1:0");
/// ```
///
/// Allow syscalls required for [`std::net::TcpListener::bind`] to work, so this works:
/// ```
/// use foundations::security::{enable_syscall_sandboxing, ViolationAction, allow_list};
/// use foundations::security::common_syscall_allow_lists::SERVICE_BASICS;
/// use std::net::TcpListener;
///
/// allow_list! {
///    static ALLOW_BIND = [
///        ..SERVICE_BASICS,
///        socket,
///        setsockopt,
///        bind,
///        listen
///    ]
/// }
///
/// enable_syscall_sandboxing(ViolationAction::KillProcess, &ALLOW_BIND).unwrap();
///
/// let _ = TcpListener::bind("127.0.0.1:0");
/// ```
pub fn enable_syscall_sandboxing(
    violation_action: ViolationAction,
    exception_rules: &Vec<Rule>,
) -> Result<(), SandboxingInitializationError> {
    let current_mode = get_current_thread_seccomp_mode()?;
    if current_mode != SeccompMode::None {
        return Err(SandboxingInitializationError::AlreadyInitialized(
            current_mode,
        ));
    }

    enable_syscall_sandboxing_ignore_existing(violation_action, exception_rules)
        .map_err(|e| e.into())
}

/// Attempts to enable [seccomp]-based syscall sandboxing in the current thread and all
/// the threads spawned by it, regardless of whether this thread is already sandboxed.
///
/// If called after sandboxing was previously set up, this will likely violate rules
/// not configured to allow the `prctl` and `seccomp` syscalls.
///
/// See [`enable_syscall_sandboxing`] for more details.
///
/// [seccomp]: https://man7.org/linux/man-pages/man2/seccomp.2.html
pub fn enable_syscall_sandboxing_ignore_existing(
    violation_action: ViolationAction,
    exception_rules: &Vec<Rule>,
) -> BootstrapResult<()> {
    let ctx = unsafe { sys::seccomp_init(violation_action as u32) };

    if ctx.is_null() {
        bail!("failed to initialize seccomp context");
    }

    for rule in exception_rules {
        let RawRule {
            action,
            syscall,
            arg_cmps,
        } = rule.into();

        let init_res = unsafe {
            sys::seccomp_rule_add_exact_array(
                ctx,
                action,
                syscall,
                arg_cmps.len().try_into().unwrap(),
                arg_cmps.as_ptr(),
            )
        };

        if init_res != 0 {
            bail!(
                "failed to add seccomp exception rule {:#?} with error code {}",
                rule,
                init_res
            );
        }
    }

    let load_res = unsafe { sys::seccomp_load(ctx) };

    if load_res != 0 {
        bail!("failed to load seccomp rules with error code {}", load_res);
    }

    Ok(())
}

/// Forbids usage of x86_64 CPU cycle counter for [Spectre] mitigation.
///
/// Any attempts to use [time stamp counter] after this function call would result in process
/// termination.
///
/// Note that this method should be called before [`enable_syscall_sandboxing`] as it can violate
/// syscall sandboxing rules.
///
/// [Spectre]: https://en.wikipedia.org/wiki/Spectre_(security_vulnerability)
/// [time stamp counter]: https://en.wikipedia.org/wiki/Time_Stamp_Counter
///
///  # Examples
///
/// It's possible to obtain CPU cycle count on x86_84 processors, providing a Spectre vulnerability
/// vector:
/// ```
/// assert!(unsafe { std::arch::x86_64::_rdtsc() } > 0);
/// ```
///
/// With forbidden timers the above code will fail to run:
/// ```should_panic
/// foundations::security::forbid_x86_64_cpu_cycle_counter();
///
/// let _ = unsafe { std::arch::x86_64::_rdtsc() } ;
/// ```
#[cfg(target_arch = "x86_64")]
pub fn forbid_x86_64_cpu_cycle_counter() {
    unsafe {
        sys::prctl(
            sys::PR_SET_TSC.try_into().unwrap(),
            sys::PR_TSC_SIGSEGV,
            0,
            0,
            0,
        )
    };
}

/// Gets the secure computing mode of the current thread.
///
/// Uses the [prctl(PR_GET_SECCOMP)] syscall, so calling this without an allow_list such as
/// the following may violate sandboxing rules if called after [`enable_syscall_sandboxing`].
///
/// ```
/// use foundations::security::{ArgCmp, allow_list, common_syscall_allow_lists::RUST_BASICS};
///
/// allow_list! {
///    pub static MY_ALLOW_LIST = [
///        ..RUST_BASICS
///    ]
/// }
/// ```
///
/// [prctl(PR_GET_SECCOMP)]: https://linuxman7.org/linux/man-pages/man2/PR_GET_SECCOMP.2const.html
fn get_current_thread_seccomp_mode() -> BootstrapResult<SeccompMode> {
    let current_seccomp_mode = unsafe { sys::prctl(PR_GET_SECCOMP as i32) };
    match current_seccomp_mode {
        0 => Ok(SeccompMode::None),
        2 => Ok(SeccompMode::Filter),
        _ => bail!("Unable to determine the current seccomp mode. Perhaps the kernel was not configured with CONFIG_SECCOMP?")
    }
}

// NOTE: `#[doc(hidden)]` + `#[doc(inline)]` for `pub use` trick is used to prevent these macros
// to show up in the crate's top level docs.

/// A convenience macro for construction of documented lists with [`Rule::Allow`]s.
///
/// The macro creates a static list of allowed syscalls. In addition to defining the list, the
/// macro also generates a doc comment appendix that lists syscalls enabled by this list (see allow
/// lists in the [`common_syscall_allow_lists`] module for an example of generated docs).
///
/// Existing lists can be merged into the new list, by using `..ANOTHER_LIST` item syntax.
/// A list of [argument comparators] can be added for a syscall by using `<syscall> if [..]` syntax.
///
/// # Examples
///
/// ```
/// use foundations::security::{allow_list, ArgCmp};
/// use foundations::security::common_syscall_allow_lists::RUST_BASICS;
///
/// allow_list! {
///     pub static MY_ALLOW_LIST = [
///         ..RUST_BASICS,
///         connect,
///         mmap,
///         exit if [ ArgCmp::Equal { arg_idx: 0, value: 0u64.into() } ]
///     ]
/// }
/// ```
#[doc(hidden)]
#[macro_export]
macro_rules! __allow_list {
    (
        $(#[$attr:meta])*
        $vis:vis static $SET_NAME:ident = $rules:tt
    ) => {
        $crate::security::allow_list!( @doc
            [],
            $rules,
            {
                $(#[$attr])*
                $vis static $SET_NAME = $rules
            }
         );
    };

    // NOTE: first munch through the list and collect doc comments.
    ( @doc
        [ $($docs:expr)* ],
        [ $(#[$attr:meta])* ..$OTHER_SET:ident $(, $($rest:tt)+ )? ],
        $allow_list_def:tt
    ) => {
        $crate::security::allow_list!( @doc
            [
                $($docs)*
                concat!("* all the syscalls from the [`", stringify!($OTHER_SET), "`] allow list")
            ],
            [ $( $( $rest )+ )? ],
            $allow_list_def
        );
    };

    ( @doc
        [ $($docs:expr)* ],
        [ $(#[$attr:meta])* $syscall:ident if $arg_cmp:tt $(, $($rest:tt)+ )? ],
        $allow_list_def:tt
    ) => {
        $crate::security::allow_list!( @doc
            [
                $($docs)*
                concat!(
                    "* [",
                    stringify!($syscall),
                    "](https://man7.org/linux/man-pages/man2/",
                    stringify!($syscall),
                    ".2.html) with argument conditions (refer to the allow list source code for more information)"
                )
            ],
            [ $( $( $rest )+ )? ],
            $allow_list_def
        );
    };

    ( @doc
        [ $($docs:expr)* ],
        [ $(#[$attr:meta])* $syscall:ident $(, $($rest:tt)+ )? ],
        $allow_list_def:tt
    ) => {
        $crate::security::allow_list!( @doc
            [
                $($docs)*
                concat!(
                    "* [",
                    stringify!($syscall),
                    "](https://man7.org/linux/man-pages/man2/",
                    stringify!($syscall),
                    ".2.html)"
                )
            ],
            [ $( $( $rest )+ )? ],
            $allow_list_def
        );
    };

    // NOTE: now expand the allow list definition
    ( @doc
        [ $($docs:expr)* ],
        [],
        {
            $(#[$attr:meta])*
            $vis:vis static $SET_NAME:ident = $rules:tt
        }
    ) => {
        $(#[$attr])*
        ///
        /// Syscalls in this allow list:
        ///
        $( #[doc = $docs] )*
        $vis static $SET_NAME:
            $crate::reexports_for_macros::once_cell::sync::Lazy<Vec<$crate::security::Rule>> =
            $crate::reexports_for_macros::once_cell::sync::Lazy::new(|| {
                let mut list = vec![];

                #[allow(clippy::vec_init_then_push)]
                {
                    $crate::security::allow_list!( @rule list, $rules );
                }

                list
            });
    };

    // NOTE: for rules we need to go through munching again. We could have done it in doc
    // collection step, but for allow list concatenation we need the list vector and macro
    // hygiene would not allow us to use the vector before its definition.
    ( @rule
        $list:ident,
        [
            $(#[$attr:meta])*
            ..$OTHER_SET:ident
            $(, $($rest:tt)+ )?
        ]
    ) => {
        $(#[$attr])*
        $list.extend_from_slice(&$OTHER_SET);

        $crate::security::allow_list!( @rule $list, [ $( $( $rest )+ )? ] );
    };

    ( @rule
        $list:ident,
        [
            $(#[$attr:meta])*
            $syscall:ident if [ $( $arg_cmp:expr ),+ ]
            $(, $($rest:tt)+ )?
        ]
    ) => {
        $(#[$attr])*
        $list.push($crate::security::Rule::Allow(
            $crate::security::Syscall::$syscall,
            vec![ $( $arg_cmp ),+ ]
        ));

        $crate::security::allow_list!( @rule $list, [ $( $( $rest )+ )? ] );
    };

    ( @rule
        $list:ident,
        [
            $(#[$attr:meta])*
            $syscall:ident
            $(, $($rest:tt)+ )?
        ]
    ) => {
        $(#[$attr])*
        $list.push($crate::security::Rule::Allow(
            $crate::security::Syscall::$syscall,
            vec![]
        ));

        $crate::security::allow_list!( @rule $list, [ $( $( $rest )+ )? ] );
    };

    ( @rule $list:ident, [] ) => {}
}

#[doc(inline)]
pub use __allow_list as allow_list;
