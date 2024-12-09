use slab::Slab;
use std::sync::{Arc, Mutex, Weak};

/// Stores a set of reference-counted objects and allows iteration over them
/// when requested. When the objects are dropped, they will be removed from
/// the set.
///
/// The objects are wrapped by the `LiveReferenceSet` through its `track`
/// method.
pub(crate) struct LiveReferenceSet<T>(Arc<LiveReferenceSetInner<T>>);

/// No default bound on T is required
impl<T> Default for LiveReferenceSet<T> {
    fn default() -> Self {
        LiveReferenceSet(Arc::new(LiveReferenceSetInner {
            active_set: Default::default(),
        }))
    }
}

struct LiveReferenceSetInner<T> {
    active_set: Mutex<Slab<Weak<LiveReferenceHandle<T>>>>,
}

impl<T> LiveReferenceSet<T> {
    /// Wrap `value` in an `Arc` and track the lifetime of the object.
    ///
    /// While the object has strong references, it is possible to obtain a
    /// reference to it through the `get_live_references` method.
    pub(crate) fn track(&self, value: T) -> Arc<LiveReferenceHandle<T>> {
        let set_ref = Arc::clone(&self.0);
        Arc::new_cyclic(|weak| {
            let slot = self.0.active_set.lock().unwrap().insert(Weak::clone(weak));
            LiveReferenceHandle {
                set_ref,
                slot,
                value,
            }
        })
    }

    /// Get references to all live objects tracked by the `LiveReferenceSet`.
    ///
    /// Because this object is internally locked, the references are cloned and
    /// collected into a `Vec`. The assumption is that any operation using
    /// the references is expensive enough that it should happen outside the critical
    /// section.
    pub(crate) fn get_live_references(&self) -> Vec<Arc<LiveReferenceHandle<T>>> {
        self.0
            .active_set
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(_, span)| span.upgrade())
            .collect()
    }
}

/// Wrapper around an object whose lifetime is tracked by `LiveReferenceSet`.
/// Access to the object is possible via the `Deref` implementation.
pub(crate) struct LiveReferenceHandle<T> {
    value: T,
    set_ref: Arc<LiveReferenceSetInner<T>>,
    slot: usize,
}

impl<T> Drop for LiveReferenceHandle<T> {
    fn drop(&mut self) {
        self.set_ref.active_set.lock().unwrap().remove(self.slot);
    }
}

impl<T> std::ops::Deref for LiveReferenceHandle<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for LiveReferenceHandle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NotifyOnDrop {
        target: Arc<Mutex<Vec<usize>>>,
        inner_val: usize,
    }

    impl Drop for NotifyOnDrop {
        fn drop(&mut self) {
            self.target.lock().unwrap().push(self.inner_val);
        }
    }

    #[test]
    fn test_live_references() {
        let notify_vec = Arc::new(Mutex::new(vec![]));
        let ref_set = Arc::new(LiveReferenceSet::default());

        // Dropping a returned reference should immediately drop the inner object
        drop(ref_set.track(NotifyOnDrop {
            target: Arc::clone(&notify_vec),
            inner_val: 1,
        }));

        assert_eq!(&*notify_vec.lock().unwrap(), &[1]);
        assert_eq!(ref_set.get_live_references().len(), 0);

        // Holding a reference should allow us to get it through the reference set
        let r1 = ref_set.track(NotifyOnDrop {
            target: Arc::clone(&notify_vec),
            inner_val: 2,
        });

        assert_eq!(&*notify_vec.lock().unwrap(), &[1]);
        assert_eq!(ref_set.get_live_references()[0].inner_val, 2);

        // Holding a second reference...
        let r2 = ref_set.track(NotifyOnDrop {
            target: Arc::clone(&notify_vec),
            inner_val: 3,
        });

        assert_eq!(&*notify_vec.lock().unwrap(), &[1]);
        assert_eq!(ref_set.get_live_references()[0].inner_val, 2);
        assert_eq!(ref_set.get_live_references()[1].inner_val, 3);

        // then dropping the first
        drop(r1);
        assert_eq!(&*notify_vec.lock().unwrap(), &[1, 2]);
        assert_eq!(ref_set.get_live_references()[0].inner_val, 3);

        drop(r2);
        assert_eq!(&*notify_vec.lock().unwrap(), &[1, 2, 3]);
        assert_eq!(ref_set.get_live_references().len(), 0);
    }
}
