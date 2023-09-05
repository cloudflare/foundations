//! Collections that implement [`Settings`] trait.
//!
//! [`Settings`]: super::Settings

use super::Settings;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;
use std::ops::{Deref, DerefMut};

/// An ordered hash map of items with [`std::string::String`] keys.
///
/// This is essentially a thin wrapper around [`indexmap::IndexMap`] that implements [`Settings`]
/// trait.
///
/// For settings it's preferable for hash map items to be ordered, otherwise generation of
/// default configuration will add a lot of ordering-related diffs between the runs which is
/// undesirable.
///
/// This is the reason why [`Settings`] trait is not implemented for [`std::collections::HashMap`].
/// In contrast, [`std::collections::BTreeMap`] provides lexicographic ordering of the keys, but
/// its usage is still discoraged as the ordering is implicit and can differ from the intended by
/// an implementor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Map<V>(#[serde(bound = "")] IndexMap<String, V>)
where
    V: Settings + Send + Sync + Clone + Serialize + DeserializeOwned + Debug + 'static;

impl<V> Default for Map<V>
where
    V: Settings + Send + Sync + Clone + Serialize + DeserializeOwned + Debug + 'static,
{
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<V> Deref for Map<V>
where
    V: Settings + Send + Sync + Clone + Serialize + DeserializeOwned + Debug + 'static,
{
    type Target = IndexMap<String, V>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<V> DerefMut for Map<V>
where
    V: Settings + Send + Sync + Clone + Serialize + DeserializeOwned + Debug + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<V> From<Map<V>> for IndexMap<String, V>
where
    V: Settings + Send + Sync + Clone + Serialize + DeserializeOwned + Debug + 'static,
{
    fn from(value: Map<V>) -> Self {
        value.0
    }
}

impl<V> From<IndexMap<String, V>> for Map<V>
where
    V: Settings + Send + Sync + Clone + Serialize + DeserializeOwned + Debug + 'static,
{
    fn from(value: IndexMap<String, V>) -> Self {
        Self(value)
    }
}

impl<V> FromIterator<(String, V)> for Map<V>
where
    V: Settings + Send + Sync + Clone + Serialize + DeserializeOwned + Debug + 'static,
{
    fn from_iter<I: IntoIterator<Item = (String, V)>>(iterable: I) -> Self {
        Self(IndexMap::from_iter(iterable))
    }
}

impl<V> Settings for Map<V>
where
    V: Settings + Send + Sync + Clone + Serialize + DeserializeOwned + Debug + 'static,
{
    fn add_docs(
        &self,
        parent_key: &[String],
        docs: &mut HashMap<Vec<String>, &'static [&'static str]>,
    ) {
        for (k, v) in self.0.iter() {
            let mut key = parent_key.to_vec();

            key.push(k.to_string());

            v.add_docs(&key, docs);
        }
    }
}
