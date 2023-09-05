use super::Settings;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt::Debug;

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
