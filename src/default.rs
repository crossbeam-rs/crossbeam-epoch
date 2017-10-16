//! The default garbage collector.
//!
//! For each thread, a participant is lazily initialized on its first use, when the current thread
//! is registered in the default collector.  If initialized, the thread's participant will get
//! destructed on thread exit, which in turn unregisters the thread.

use collector::{Collector, Handle};
use scope::Scope;

lazy_static! {
    /// The global data for the default garbage collector.
    static ref COLLECTOR: Collector = Collector::new();
}

thread_local! {
    /// The per-thread participant for the default garbage collector.
    static HANDLE: Handle = COLLECTOR.handle();
}

/// Returns the default handle.
///
/// # Safety
///
/// It should not be called in another TLS storage's `drop()`, because `HANDLE` may already be
/// droped.
pub unsafe fn default_handle() -> &'static Handle {
    &*HANDLE.with(|handle| handle as *const _)
}

/// Pin the current thread.
pub fn pin<F, R>(f: F) -> R
where
    F: FnOnce(&Scope) -> R,
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


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_handle_flush() {
        unsafe { default_handle().pin(|scope| { scope.flush(); }) }
    }
}
