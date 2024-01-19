use super::Settings;

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

impl<T: Settings> Settings for Vec<T> {
    fn add_docs(
        &self,
        parent_key: &[String],
        docs: &mut std::collections::HashMap<Vec<String>, &'static [&'static str]>,
    ) {
        for (k, v) in self.iter().enumerate() {
            let mut key = parent_key.to_vec();

            key.push(k.to_string());

            v.add_docs(&key, docs);
        }
    }
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
