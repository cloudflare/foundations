//! Collections that implement [`Settings`] trait.
//!
//! [`Settings`]: super::Settings

use super::Settings;
use indexmap::IndexMap;
use indexmap::map::{IntoIter, Iter, IterMut};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::ops::{Deref, DerefMut};

/// An ordered hash map of items.
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
pub struct Map<K, V>(#[serde(bound = "")] IndexMap<K, V>)
where
    K: Eq + Hash + Clone + Serialize + DeserializeOwned + Debug + 'static,
    V: Settings;

impl<K, V> Default for Map<K, V>
where
    K: Eq + Hash + Clone + Serialize + DeserializeOwned + Debug + 'static,
    V: Settings,
{
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<K, V> Deref for Map<K, V>
where
    K: Eq + Hash + Clone + Serialize + DeserializeOwned + Debug + 'static,
    V: Settings,
{
    type Target = IndexMap<K, V>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<K, V> DerefMut for Map<K, V>
where
    K: Eq + Hash + Clone + Serialize + DeserializeOwned + Debug + 'static,
    V: Settings,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<K, V> From<Map<K, V>> for IndexMap<K, V>
where
    K: Eq + Hash + Clone + Serialize + DeserializeOwned + Debug + 'static,
    V: Settings,
{
    fn from(value: Map<K, V>) -> Self {
        value.0
    }
}

impl<K, V> From<IndexMap<K, V>> for Map<K, V>
where
    K: Eq + Hash + Clone + Serialize + DeserializeOwned + Debug + 'static,
    V: Settings,
{
    fn from(value: IndexMap<K, V>) -> Self {
        Self(value)
    }
}

impl<K, V> FromIterator<(K, V)> for Map<K, V>
where
    K: Eq + Hash + Clone + Serialize + DeserializeOwned + Debug + 'static,
    V: Settings,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iterable: I) -> Self {
        Self(IndexMap::from_iter(iterable))
    }
}

impl<'a, K, V> IntoIterator for &'a Map<K, V>
where
    K: Eq + Hash + Clone + Serialize + DeserializeOwned + Debug + 'static,
    V: Settings,
{
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a, K, V> IntoIterator for &'a mut Map<K, V>
where
    K: Eq + Hash + Clone + Serialize + DeserializeOwned + Debug + 'static,
    V: Settings,
{
    type Item = (&'a K, &'a mut V);
    type IntoIter = IterMut<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

impl<K, V> IntoIterator for Map<K, V>
where
    K: Eq + Hash + Clone + Serialize + DeserializeOwned + Debug + 'static,
    V: Settings,
{
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<K, V> Settings for Map<K, V>
where
    K: Display + Eq + Hash + Clone + Serialize + DeserializeOwned + Debug + 'static,
    V: Settings,
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
