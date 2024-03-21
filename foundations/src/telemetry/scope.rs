use std::cell::RefCell;
use std::marker::PhantomData;
use thread_local::ThreadLocal;

// NOTE: this prevents scope handles to be held between `await` points by making scope
// handles `!Send`. Negative trait bounds are not available in Rust yet, so instead we
// use a raw pointer here which is `!Send`. We introduce a zero-sized structure behind
// the pointer to give some context to the occuring error.
struct DontHoldScopeHandlesAcrossAwaitPoints;
type AwaitPointGuard = PhantomData<*mut DontHoldScopeHandlesAcrossAwaitPoints>;

pub(crate) struct ScopeStack<T>(ThreadLocal<RefCell<Vec<T>>>)
where
    T: Send + Clone + 'static;

impl<T> Default for ScopeStack<T>
where
    T: Send + Clone + 'static,
{
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<T> ScopeStack<T>
where
    T: Send + Clone + 'static,
{
    pub(crate) fn current(&self) -> Option<T> {
        self.0.get_or_default().borrow().last().cloned()
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
        scope_stack.0.get_or_default().borrow_mut().push(item);

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
        self.scope_stack.0.get_or_default().borrow_mut().pop();
    }
}
