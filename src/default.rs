//! The default collector for garbage collection.
//!
//! For each thread, a handle is lazily initialized on its first use, when the current thread is
//! registered in the default collector.  If initialized, the thread's handle will get destructed
//! on thread exit, which in turn unregisters the thread.

use collector::Collector;
use handle::{Handle, Scope};

lazy_static! {
    /// The default global data.
    pub static ref COLLECTOR: Collector = Collector::new();
}

thread_local! {
    /// The thread-local handle for the default global data.
    static HANDLE: Handle = COLLECTOR.add_handle();
}

/// Pin the current thread.
pub fn pin<F, R>(f: F) -> R
where
    F: for<'scope> FnOnce(Scope<'scope>) -> R,
{
    // FIXME(jeehoonkang): thread-local storage may be destructed at the time `pin()` is called. For
    // that case, we should use `HANDLE.try_with()` instead.
    HANDLE.with(|handle| handle.pin(f))
}

/// Check if the current thread is pinned.
pub fn is_pinned() -> bool {
    // FIXME(jeehoonkang): thread-local storage may be destructed at the time `pin()` is called. For
    // that case, we should use `HANDLE.try_with()` instead.
    HANDLE.with(|handle| handle.is_pinned())
}
