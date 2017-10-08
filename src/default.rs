//! The default garbage collector.
//!
//! For each thread, a participant is lazily initialized on its first use, when the current thread
//! is registered in the default collector.  If initialized, the thread's participant will get
//! destructed on thread exit, which in turn unregisters the thread.

use std::ops::Deref;

use internal::{Global, Local};
use scope::Scope;

lazy_static! {
    /// The global data for the default garbage collector.
    static ref GLOBAL: Global = Global::new();
}

thread_local! {
    /// The per-thread participant for the default garbage collector.
    static HANDLE: Handle = Handle::new();
}

struct Handle(Local);

impl Handle {
    fn new() -> Self {
        Self { 0: Local::new(&GLOBAL) }
    }
}

impl Deref for Handle {
    type Target = Local;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { self.0.finalize(&GLOBAL) }
    }
}

/// Pin the current thread.
pub fn pin<F, R>(f: F) -> R
where
    F: for<'scope> FnOnce(Scope<'scope>) -> R,
{
    // FIXME(jeehoonkang): thread-local storage may be destructed at the time `pin()` is called. For
    // that case, we should use `HANDLE.try_with()` instead.
    HANDLE.with(|handle| handle.pin(&GLOBAL, f))
}

/// Check if the current thread is pinned.
pub fn is_pinned() -> bool {
    // FIXME(jeehoonkang): thread-local storage may be destructed at the time `pin()` is called. For
    // that case, we should use `HANDLE.try_with()` instead.
    HANDLE.with(|handle| handle.is_pinned())
}
