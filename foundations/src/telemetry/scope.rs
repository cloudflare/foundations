use std::cell::RefCell;
use std::marker::PhantomData;
use std::thread::LocalKey;

// NOTE: this prevents scope handles to be held between `await` points by making scope
// handles `!Send`. Negative trait bounds are not available in Rust yet, so instead we
// use a raw pointer here which is `!Send`. We introduce a zero-sized structure behind
// the pointer to give some context to the occuring error.
struct DontHoldScopeHandlesAcrossAwaitPoints;
type AwaitPointGuard = PhantomData<*mut DontHoldScopeHandlesAcrossAwaitPoints>;

// Keep scope stacks in the standard library's thread-local storage, avoid
// `thread_local::ThreadLocal` due to false sharing possibility.
pub(crate) struct ScopeStack<T>(&'static LocalKey<RefCell<Vec<T>>>)
where
    T: Send + Clone + 'static;

impl<T> ScopeStack<T>
where
    T: Send + Clone + 'static,
{
    pub(crate) const fn new(stack: &'static LocalKey<RefCell<Vec<T>>>) -> Self {
        Self(stack)
    }

    pub(crate) fn current(&self) -> Option<T> {
        self.0.with_borrow(|stack| stack.last().cloned())
    }
}

pub(crate) struct Scope<T>
where
    T: Send + Clone + 'static,
{
    scope_stack: &'static ScopeStack<T>,
    await_point_guard: AwaitPointGuard,
}

impl<T> Scope<T>
where
    T: Send + Clone + 'static,
{
    pub(crate) fn new(scope_stack: &'static ScopeStack<T>, item: T) -> Self {
        scope_stack.0.with_borrow_mut(|stack| stack.push(item));

        Self {
            scope_stack,
            await_point_guard: PhantomData,
        }
    }
}

impl<T> Drop for Scope<T>
where
    T: Send + Clone + 'static,
{
    fn drop(&mut self) {
        self.scope_stack.0.with_borrow_mut(|stack| {
            stack.pop();
        });
    }
}
