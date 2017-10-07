//! The default garbage collector.
//!
//! For each thread, a participant is lazily initialized on its first use, when the current thread
//! is registered in the default collector.  If initialized, the thread's participant will get
//! destructed on thread exit, which in turn unregisters the thread.

use internal::{Global, Participant};
use scope::Scope;

lazy_static! {
    /// The global data for the default garbage collector.
    static ref COLLECTOR: Global = Global::new();
}

thread_local! {
    /// The per-thread participant for the default garbage collector.
    static PARTICIPANT: Participant = Participant::new(&COLLECTOR);
}

/// Pin the current thread.
pub fn pin<F, R>(f: F) -> R
where
    F: for<'scope> FnOnce(Scope<'scope>) -> R,
{
    // FIXME(jeehoonkang): thread-local storage may be destructed at the time `pin()` is called. For
    // that case, we should use `PARTICIPANT.try_with()` instead.
    PARTICIPANT.with(|participant| participant.pin(&COLLECTOR, f))
}

/// Check if the current thread is pinned.
pub fn is_pinned() -> bool {
    // FIXME(jeehoonkang): thread-local storage may be destructed at the time `pin()` is called. For
    // that case, we should use `PARTICIPANT.try_with()` instead.
    PARTICIPANT.with(|participant| participant.is_pinned())
}
