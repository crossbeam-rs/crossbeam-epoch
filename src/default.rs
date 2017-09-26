//! The default collector for garbage collection.
//!
//! For each thread, a mutator is lazily initialized on its first use, when the current thread is
//! registered in the default collector.  If initialized, the thread's mutator will get destructed on
//! thread exit, which in turn unregisters the thread.

use collector::Collector;
use mutator::{Mutator, Scope};

lazy_static! {
    /// The default global data.
    // FIXME(jeehoonkang): accessing globals in `lazy_static!` is blocking.
    pub static ref COLLECTOR: Collector = Collector::new();
}

thread_local! {
    /// The thread-local mutator for the default global data.
    static MUTATOR: Mutator<'static> = COLLECTOR.add_mutator();
}

/// Pin the current thread.
pub fn pin<F, R>(f: F) -> R
where
    F: FnOnce(&Scope) -> R,
{
    // FIXME(jeehoonkang): thread-local storage may be destructed at the time `pin()` is called. For
    // that case, we should use `MUTATOR.try_with()` instead.
    MUTATOR.with(|mutator| mutator.pin(f))
}

/// Check if the current thread is pinned.
pub fn is_pinned() -> bool {
    // FIXME(jeehoonkang): thread-local storage may be destructed at the time `pin()` is called. For
    // that case, we should use `MUTATOR.try_with()` instead.
    MUTATOR.with(|mutator| mutator.is_pinned())
}
