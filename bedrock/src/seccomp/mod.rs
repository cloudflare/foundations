//! [TODO]

pub mod common_allow_lists;
mod syscalls;

pub use self::syscalls::Syscall;

/// A raw OS error code to be returned by [`Rule::ReturnError`].
pub type RawOsErrorNum = u16;

/// Syscall argument comparators to be used in [`Rule`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ArgCmp {
    /// Checks that argument is not equal to the provided value.
    NotEqual {
        /// The index of the argument.
        arg_idx: usize,

        /// Value to compare the argument with (can be a raw pointer).
        value: u64,
    },

    /// Checks that argument is less than the provided value.
    LessThan {
        /// The index of the argument.
        arg_idx: usize,

        /// Value to compare the argument with (can be a raw pointer).
        value: u64,
    },

    /// Checks that argument is less than or equal to the provided value.
    LessThanOrEqual {
        /// The index of the argument.
        arg_idx: usize,

        /// Value to compare the argument with (can be a raw pointer).
        value: u64,
    },

    /// Checks that argument is equal to the provided value.
    Equal {
        /// The index of the argument.
        arg_idx: usize,

        /// Value to compare the argument with (can be a raw pointer).
        value: u64,
    },

    /// Checks that argument is greater than or equal to the provided value.
    GreaterThanOrEqual {
        /// The index of the argument.
        arg_idx: usize,

        /// Value to compare the argument with (can be a raw pointer).
        value: u64,
    },

    /// Checks that argument is greater than the provided value.
    GreaterThan {
        /// The index of the argument.
        arg_idx: usize,

        /// Value to compare the argument with (can be a raw pointer).
        value: u64,
    },

    /// Checks that argument is equal to the provided value after application of the provided
    /// bitmask.
    EqualMasked {
        /// The index of the argument.
        arg_idx: usize,

        /// The bitmask to be applied to the argument before comparison.
        mask: u64,

        /// Value to compare the masked argument with.
        value: u64,
    },
}

/// [TODO]
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    /// [TODO]
    Allow(Syscall, Vec<ArgCmp>),
    /// [TODO]
    ReturnError(Syscall, Vec<ArgCmp>, RawOsErrorNum),
}

// NOTE: `#[doc(hidden)]` + `#[doc(inline)]` for `pub use` trick is used to prevent these macros
// to show up in the crate's top level docs.

/// [TODO]
#[doc(hidden)]
#[macro_export]
macro_rules! __allow_list {
    (
        $(#[$attr:meta])*
        $vis:vis static $SET_NAME:ident = $rules:tt
    ) => {
        $crate::seccomp::allow_list!( @doc
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
        $crate::seccomp::allow_list!( @doc
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
        $crate::seccomp::allow_list!( @doc
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
        $crate::seccomp::allow_list!( @doc
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
        #[doc = ""] // NOTE: blank line
        #[doc = "Syscalls in this allow list:"]
        #[doc = ""] // NOTE: blank line
        $( #[doc = $docs] )*
        $vis static $SET_NAME:
            $crate::reexports_for_macros::once_cell::sync::Lazy<Vec<$crate::seccomp::Rule>> =
            $crate::reexports_for_macros::once_cell::sync::Lazy::new(|| {
                let mut list = vec![];

                #[allow(clippy::vec_init_then_push)]
                {
                    $crate::seccomp::allow_list!( @rule list, $rules );
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

        $crate::seccomp::allow_list!( @rule $list, [ $( $( $rest )+ )? ] );
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
        $list.push($crate::seccomp::Rule::Allow(
            $crate::seccomp::Syscall::$syscall,
            vec![ $( $arg_cmp ),+ ]
        ));

        $crate::seccomp::allow_list!( @rule $list, [ $( $( $rest )+ )? ] );
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
        $list.push($crate::seccomp::Rule::Allow(
            $crate::seccomp::Syscall::$syscall,
            vec![]
        ));

        $crate::seccomp::allow_list!( @rule $list, [ $( $( $rest )+ )? ] );
    };

    ( @rule $list:ident, [] ) => {}
}

#[doc(inline)]
pub use __allow_list as allow_list;
