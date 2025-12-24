use super::TelemetryScope;
use crate::utils::feature_use;
use pin_project_lite::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

feature_use!(cfg(feature = "logging"), {
    use super::log::internal::{LogScope, SharedLog, current_log, fork_log};
    use std::sync::Arc;
});

feature_use!(cfg(feature = "tracing"), {
    use super::tracing::SpanScope;
    use super::tracing::internal::{SharedSpan, current_span, fork_trace};
    use std::borrow::Cow;

    feature_use!(cfg(feature = "testing"), {
        use super::tracing::internal::Tracer;
        use super::tracing::testing::{TestTracerScope, current_test_tracer};
    });
});

#[cfg(feature = "testing")]
use super::testing::TestTelemetryContext;

/// Wrapper for a future that provides it with [`TelemetryContext`].
pub struct WithTelemetryContext<'f, T> {
    // NOTE: we intentionally erase type here as we can get close to the type
    // length limit, adding telemetry wrappers on top causes compiler to fail in some
    // cases.
    inner: Pin<Box<dyn Future<Output = T> + Send + 'f>>,
    ctx: TelemetryContext,
}

impl<T> Future for WithTelemetryContext<'_, T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let _telemetry_scope = self.ctx.scope();

        self.inner.as_mut().poll(cx)
    }
}

/// The same as [`WithTelemetryContext`], but for futures that are `!Send`.
pub struct WithTelemetryContextLocal<'f, T> {
    // NOTE: we intentionally erase type here as we can get close to the type
    // length limit, adding telemetry wrappers on top causes compiler to fail in some
    // cases.
    inner: Pin<Box<dyn Future<Output = T> + 'f>>,
    ctx: TelemetryContext,
}

impl<T> Future for WithTelemetryContextLocal<'_, T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let _telemetry_scope = self.ctx.scope();

        self.inner.as_mut().poll(cx)
    }
}

pin_project! {
    /// The same as [`WithTelemetryContext`], but for futures that are not boxed
    pub struct WithTelemetryContextGeneric<T> {
        #[pin]
        inner: T,
        ctx: TelemetryContext,
    }
}

impl<T: Future> Future for WithTelemetryContextGeneric<T> {
    type Output = T::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _telemetry_scope = this.ctx.scope();

        this.inner.poll(cx)
    }
}

/// Implicit context for logging and tracing.
///
/// Current context can be obtained with the [`TelemetryContext::current`] method.
///
/// The context can be forked with [`TelemetryContext::with_forked_log`] and
/// [`TelemetryContext::with_forked_trace`] methods. And propagated in different scopes with
/// [`TelemetryContext::scope`] and [`TelemetryContext::apply`] methods.
#[derive(Debug, Clone)]
pub struct TelemetryContext {
    #[cfg(feature = "logging")]
    pub(super) log: SharedLog,

    // NOTE: we might not have tracing root at this point
    #[cfg(feature = "tracing")]
    pub(super) span: Option<SharedSpan>,

    #[cfg(all(feature = "tracing", feature = "testing"))]
    pub(super) test_tracer: Option<Tracer>,
}

impl TelemetryContext {
    /// Returns the telemetry context that is active in the current scope.
    pub fn current() -> Self {
        Self {
            #[cfg(feature = "logging")]
            log: current_log(),

            #[cfg(feature = "tracing")]
            span: current_span(),

            #[cfg(all(feature = "tracing", feature = "testing"))]
            test_tracer: current_test_tracer(),
        }
    }

    /// Creates a scope handle for the telemetry context.
    ///
    /// The telemetry context is active in the scope unless the handle is dropped.
    ///
    /// Note that the scope can only be used in the sync contexts. To propagate telemetry context
    /// to async contexts, [`TelemetryContext::apply`] should be used instead.
    ///
    /// The scope handle is useful to propagate the telemetry context to callbacks provided
    /// by third party libraries or other threads.
    ///
    /// The other use case is situations where code control flow doesn't align with telemetry flow.
    /// E.g. there is a centralized dispatcher that drives certain tasks and tasks are registered
    /// with the dispatcher via callbacks. It's desirable for each task to have its own telemetry
    /// context, so scope can be used to propagate task context to the dispatcher's callbacks.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::TelemetryContext;
    /// use foundations::telemetry::tracing::{self, test_trace};
    ///
    /// // Test context is used for demonstration purposes to show the resulting traces.
    /// let ctx = TelemetryContext::test();
    ///
    /// {
    ///     let _scope = ctx.scope();
    ///     let _root = tracing::span("root");
    ///     let telemetry_ctx = TelemetryContext::current();
    ///
    ///     let handle = std::thread::spawn(move || {
    ///         let _scope = telemetry_ctx.scope();
    ///         let _child = tracing::span("child");
    ///     });
    ///
    ///     handle.join();
    /// }
    ///
    /// assert_eq!(
    ///     ctx.traces(Default::default()),
    ///     vec![
    ///         test_trace! {
    ///             "root" => {
    ///                 "child"
    ///             }
    ///         },
    ///     ]
    /// );
    /// ```
    pub fn scope(&self) -> TelemetryScope {
        TelemetryScope {
            #[cfg(feature = "logging")]
            _log_scope: LogScope::new(Arc::clone(&self.log)),

            #[cfg(feature = "tracing")]
            _span_scope: self.span.as_ref().cloned().map(SpanScope::new),

            #[cfg(all(feature = "tracing", feature = "testing"))]
            _test_tracer_scope: self.test_tracer.as_ref().cloned().map(TestTracerScope::new),
        }
    }

    /// Creates a test telemetry context.
    ///
    /// Returned context has the same API as standard context, but also exposes API to obtain the
    /// telemetry collected in it.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::TelemetryContext;
    /// use foundations::telemetry::tracing::{self, test_trace};
    /// use foundations::telemetry::log::{self, TestLogRecord};
    /// use foundations::telemetry::settings::Level;
    ///
    /// #[tracing::span_fn("sync_fn")]
    /// fn some_sync_production_fn_that_we_test() {
    ///     log::warn!("Sync hello!");
    /// }
    ///
    /// #[tracing::span_fn("async_fn")]
    /// async fn some_async_production_fn_that_we_test() {
    ///     log::warn!("Async hello!");
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let ctx = TelemetryContext::test();
    ///
    ///     {
    ///         let _scope = ctx.scope();
    ///         let _root = tracing::span("root");
    ///
    ///         let handle = tokio::spawn(TelemetryContext::current().apply(async {
    ///             some_async_production_fn_that_we_test().await;
    ///         }));
    ///
    ///         handle.await;
    ///
    ///         some_sync_production_fn_that_we_test();
    ///     }
    ///
    ///     assert_eq!(*ctx.log_records(), &[
    ///         TestLogRecord {
    ///             level: Level::Warning,
    ///             message: "Async hello!".into(),
    ///             fields: vec![]
    ///         },
    ///         TestLogRecord {
    ///             level: Level::Warning,
    ///             message: "Sync hello!".into(),
    ///             fields: vec![]
    ///         }
    ///     ]);
    ///
    ///     assert_eq!(
    ///         ctx.traces(Default::default()),
    ///         vec![
    ///             test_trace! {
    ///                 "root" => {
    ///                     "async_fn",
    ///                     "sync_fn"
    ///                 }
    ///             }
    ///         ]
    ///     );
    /// }
    /// ```
    #[cfg(feature = "testing")]
    pub fn test() -> TestTelemetryContext {
        TestTelemetryContext::new()
    }

    /// Wraps a future with the telemetry context.
    ///
    /// [`TelemetryScope`] can't be used across `await` points to propagate the telemetry context,
    /// so to use telemetry context in async blocks, futures should be wrapped using this
    /// method instead.
    ///
    /// Note that you don't need to use this method to wrap async function's bodies,
    /// as [`tracing::span_fn`] macro takes care of that.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::TelemetryContext;
    /// use foundations::telemetry::tracing::{self, test_trace};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // Test context is used for demonstration purposes to show the resulting traces.
    ///     let ctx = TelemetryContext::test();
    ///
    ///     {
    ///         let _scope = ctx.scope();
    ///
    ///         let handle = tokio::spawn(
    ///             tracing::span("root").into_context().apply(async {
    ///                 let _child = tracing::span("child");
    ///             })
    ///         );
    ///
    ///         handle.await;
    ///     }
    ///
    ///     assert_eq!(
    ///         ctx.traces(Default::default()),
    ///         vec![
    ///             test_trace! {
    ///                 "root" => {
    ///                     "child"
    ///                 }
    ///             }
    ///         ]
    ///     );
    /// }
    /// ```
    ///
    /// [`tracing::span_fn`]: crate::telemetry::tracing::span_fn
    pub fn apply<'f, F>(&self, fut: F) -> WithTelemetryContext<'f, F::Output>
    where
        F: Future + Send + 'f,
    {
        WithTelemetryContext {
            inner: Box::pin(fut),
            ctx: self.clone(),
        }
    }

    /// The same as [`TelemetryContext::apply`], but for futures that are `!Send`.
    pub fn apply_local<'f, F>(&self, fut: F) -> WithTelemetryContextLocal<'f, F::Output>
    where
        F: Future + 'f,
    {
        WithTelemetryContextLocal {
            inner: Box::pin(fut),
            ctx: self.clone(),
        }
    }

    /// The same as [`TelemetryContext::apply`], but for futures that are not boxed.
    pub fn apply_generic<F>(&self, fut: F) -> WithTelemetryContextGeneric<F>
    where
        F: Future,
    {
        WithTelemetryContextGeneric {
            inner: fut,
            ctx: self.clone(),
        }
    }
}

#[cfg(feature = "tracing")]
impl TelemetryContext {
    /// Creates a new telemetry context, that includes a forked trace, creating a
    /// linked child trace.
    ///
    /// If the current trace is sampled, the new child trace also will be sampled.
    /// If the current trace isn't sampled, no new child trace is created.
    ///
    /// This method is useful to avoid a single trace from ballooning in size
    /// while still keeping navigability from the source trace to the child
    /// traces and vice-versa.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::TelemetryContext;
    /// use foundations::telemetry::tracing::{self, test_trace};
    ///
    /// // Test context is used for demonstration purposes to show the resulting traces.
    /// let ctx = TelemetryContext::test();
    ///
    /// {
    ///     let _scope = ctx.scope();
    ///     let _root = tracing::span("root");
    ///
    ///     {
    ///         let _span1 = tracing::span("span1");
    ///     }
    ///
    ///     let _scope = TelemetryContext::current()
    ///         .with_forked_trace("new fork")
    ///         .scope();
    ///
    ///     let _span2 = tracing::span("span2");
    /// }
    ///
    /// assert_eq!(
    ///     ctx.traces(Default::default()),
    ///     vec![
    ///         test_trace! {
    ///             "root" => {
    ///                 "span1",
    ///                 "[new fork ref]"
    ///             }
    ///         },
    ///         test_trace! {
    ///             "new fork" => {
    ///                 "span2"
    ///             }
    ///         }
    ///     ]
    /// );
    pub fn with_forked_trace(&self, fork_name: impl Into<Cow<'static, str>>) -> Self {
        Self {
            #[cfg(feature = "logging")]
            log: Arc::clone(&self.log),

            span: Some(fork_trace(fork_name)),

            #[cfg(feature = "testing")]
            test_tracer: self.test_tracer.clone(),
        }
    }
}

#[cfg(feature = "logging")]
impl TelemetryContext {
    /// Creates a telemetry context with log that is detached from the current context's log, but
    /// inherits its log fields.
    ///
    /// For example, can be used in server software to produce separate logs for HTTP requests, each
    /// of which has log fields added during the HTTP connection establishment.
    ///
    /// # Examples
    /// ```
    /// use foundations::telemetry::TelemetryContext;
    /// use foundations::telemetry::log::{self, TestLogRecord};
    /// use foundations::telemetry::settings::Level;
    ///
    /// // Test context is used for demonstration purposes to show the resulting log records.
    /// let ctx = TelemetryContext::test();
    /// let _scope = ctx.scope();
    ///
    /// log::add_fields!("conn_field" => 42);
    ///
    /// {
    ///     let _scope = TelemetryContext::current().with_forked_log().scope();
    ///
    ///     log::add_fields!("req1_field" => "foo");
    ///     log::warn!("Hello from request 1");
    /// }
    ///
    /// {
    ///     let _scope = TelemetryContext::current().with_forked_log().scope();
    ///
    ///     log::add_fields!("req2_field" => "bar");
    ///     log::warn!("Hello from request 2");
    /// }
    ///
    /// log::warn!("Hello from connection");
    ///
    /// assert_eq!(*ctx.log_records(), &[
    ///     TestLogRecord {
    ///         level: Level::Warning,
    ///         message: "Hello from request 1".into(),
    ///         fields: vec![
    ///             ("req1_field".into(), "foo".into()),
    ///             ("conn_field".into(), "42".into()),
    ///         ]
    ///     },
    ///     TestLogRecord {
    ///         level: Level::Warning,
    ///         message: "Hello from request 2".into(),
    ///         fields: vec![
    ///             ("req2_field".into(), "bar".into()),
    ///             ("conn_field".into(), "42".into()),
    ///         ]
    ///     },
    ///     TestLogRecord {
    ///         level: Level::Warning,
    ///         message: "Hello from connection".into(),
    ///         fields: vec![
    ///             ("conn_field".into(), "42".into()),
    ///         ]
    ///     }
    /// ]);
    /// ```
    pub fn with_forked_log(&self) -> Self {
        Self {
            log: fork_log(),

            #[cfg(feature = "tracing")]
            span: self.span.clone(),

            #[cfg(all(feature = "tracing", feature = "testing"))]
            test_tracer: self.test_tracer.clone(),
        }
    }
}
