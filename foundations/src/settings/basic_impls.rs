use super::Settings;
use indexmap::IndexSet;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, LinkedList, VecDeque};
use std::ffi::{CString, OsString};
use std::fmt::Debug;
use std::hash::Hash;
use std::marker::PhantomData;
use std::num::{NonZero, Saturating, Wrapping};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

macro_rules! impl_noop {
    ( $( $impl_desc:tt )* ) => {
        impl $( $impl_desc )* {
            #[inline]
            fn add_docs(
                &self,
                _parent_key: &[String],
                _docs: &mut std::collections::HashMap<Vec<String>, &'static [&'static str]>,
            ) {
            }
        }
    };
}

impl_noop!(<T> Settings for PhantomData<T> where T: 'static);
impl_noop!(<Idx> Settings for Range<Idx> where Idx: Debug + Serialize + DeserializeOwned + Clone + Default + 'static);
impl_noop!(<T> Settings for Reverse<T> where T: Settings);
impl_noop!(<T> Settings for Wrapping<T> where T: Settings);

// serde does not have generic impls for Saturating<T> and NonZero<T>
macro_rules! impl_for_num {
    ( $( $Ty:ty )* ) => { $(
        impl_noop!(Settings for $Ty);
        impl_noop!(Settings for Saturating<$Ty>);
        impl_noop!(Settings for Option<NonZero<$Ty>>);
    )* };
}

impl_for_num! {
    i8 i16 i32 i64 i128 isize
    u8 u16 u32 u64 u128 usize
}

macro_rules! impl_for_non_generic {
    ( $( $Ty:ty ),* ) => {
        $( impl_noop!(Settings for $Ty); )*
    };
}

impl_for_non_generic! {
    bool,
    char,
    f32,
    f64,
    String,
    (),
    CString,
    OsString,
    Duration,
    PathBuf
}

macro_rules! impl_for_ref {
    ( $( $impl_desc:tt )* ) => {
        impl $( $impl_desc )* {
            #[inline]
            fn add_docs(
                &self,
                parent_key: &[String],
                docs: &mut std::collections::HashMap<Vec<String>, &'static [&'static str]>,
            ) {
                (**self).add_docs(parent_key, docs);
            }
        }
    };
}

impl_for_ref!(<T> Settings for Box<T> where T: Settings);
impl_for_ref!(<T> Settings for Rc<T> where T: Settings, Rc<T>: Serialize + DeserializeOwned);
impl_for_ref!(<T> Settings for Arc<T> where T: Settings, Arc<T>: Serialize + DeserializeOwned);

macro_rules! impl_for_seq {
    ( $( $impl_desc:tt )* ) => {
        impl $( $impl_desc )* {
            fn add_docs(
                &self,
                parent_key: &[String],
                docs: &mut std::collections::HashMap<Vec<String>, &'static [&'static str]>,
            ) {
                let mut key = parent_key.to_vec();

                for (k, v) in self.iter().enumerate() {

                    key.push(k.to_string());
                    v.add_docs(&key, docs);
                    key.pop();
                }
            }
        }
    };
}

impl_for_seq!(<T> Settings for Vec<T> where T: Settings);
impl_for_seq!(<T> Settings for VecDeque<T> where T: Settings);
impl_for_seq!(<T> Settings for BinaryHeap<T> where T: Settings + Ord);
impl_for_seq!(<T> Settings for IndexSet<T> where T: Settings + Eq + Hash);
impl_for_seq!(<T> Settings for LinkedList<T> where T: Settings);

macro_rules! impl_for_array {
    ( $( $len:tt )* ) => {
        $( impl_for_seq!(<T> Settings for [T; $len] where T: Settings); )*
    };
}

impl_for_array! {
     0  1  2  3  4  5  6  7  8  9
    10 11 12 13 14 15 16 17 18 19
    20 21 22 23 24 25 26 27 28 29
    30 31 32
}

impl<T: Settings> Settings for Option<T> {
    fn add_docs(
        &self,
        parent_key: &[String],
        docs: &mut std::collections::HashMap<Vec<String>, &'static [&'static str]>,
    ) {
        if let Some(v) = self {
            v.add_docs(parent_key, docs);
        }
    }
}
