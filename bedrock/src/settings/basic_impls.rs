use super::Settings;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;

macro_rules! impl_for_basic_types {
    ($($ty:ty),*) => {
        $(impl Settings for $ty {})*
    };
}

impl_for_basic_types![
    bool,
    isize,
    i8,
    i16,
    i32,
    i64,
    usize,
    u8,
    u16,
    u32,
    u64,
    f32,
    f64,
    char,
    String,
    std::path::PathBuf,
    ()
];

impl<T: Send + Sync + Clone + Serialize + DeserializeOwned + Debug + 'static> Settings for Vec<T> {}

impl<T: Send + Sync + Clone + Serialize + DeserializeOwned + Debug + 'static> Settings
    for Option<T>
{
}

impl<K, V> Settings for HashMap<K, V>
where
    K: Send + Sync + Clone + Serialize + DeserializeOwned + Debug + Hash + Eq + 'static,
    V: Send + Sync + Clone + Serialize + DeserializeOwned + Debug + 'static,
{
}
