// NOTE: don't complain about unused macro for feature combinations that don't use it.
#[allow(unused_macros)]
macro_rules! feature_use {
    ( $cfg:meta, { $( $tokens:tt )+ } ) => {
        // NOTE: a trick to apply attribute to all the tokens in a group without
        // introducing a new block or scope: we apply the attribute
        // to a macro call that expand to those tokens instead.
        #[$cfg]
        $crate::utils::feature_use!(@tokens $($tokens)*);
    };

    ( @tokens $( $tokens:tt )* ) => { $( $tokens )* };
}

// NOTE: don't complain about unused macro for feature combinations that don't use it.
#[allow(unused_imports)]
pub(crate) use feature_use;
