use once_cell::sync::OnceCell;
use tokio::runtime::{EnterGuard, Runtime};

static RUNTIME: OnceCell<Runtime> = OnceCell::new();

pub(crate) fn get_runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        // `current_thread`, not `multi_thread`. With a current-thread
        // runtime, `Runtime::block_on` drives the future on the
        // calling thread itself — which is the thread on which the
        // Kotlin/JNI bridge called `init`. That thread is the one
        // whose `CONTEXT.current.handle` we populate via the
        // leaked `EnterGuard` below.
        //
        // We tried `new_multi_thread().worker_threads(1)` first and
        // observed that sqlx 0.9's `pool::inner::acquire` →
        // `rt::timeout` calls `Handle::try_current()` from the
        // tokio *worker* thread, which has no `CONTEXT` set, so the
        // check returns `Err` and `missing_rt` panics. With
        // current-thread the future is polled on the FFI thread,
        // where the leaked `EnterGuard` is in scope, and
        // `Handle::try_current()` succeeds.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");
        // SAFETY: `EnterGuard` stores only two raw pointers (the
        // runtime + the previous CONTEXT slot) and a `usize` depth;
        // it does not actually borrow `runtime` in the lifetime
        // sense. The `PhantomData<&'a Handle>` in its definition is
        // purely for variance, so the `'a → 'static` transmutation
        // is sound. `Box::leak` then gives the guard `'static`
        // storage and skips its `Drop` (the `CONTEXT` thread-local
        // is torn down on thread/process exit anyway).
        let guard: EnterGuard<'static> =
            unsafe { std::mem::transmute::<EnterGuard<'_>, EnterGuard<'static>>(runtime.enter()) };
        let _ = Box::leak(Box::new(guard));
        runtime
    })
}

pub fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    get_runtime().block_on(fut)
}
