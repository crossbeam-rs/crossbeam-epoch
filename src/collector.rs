/// Epoch-based garbage collector.
///
/// # Examples
///
/// ```
/// use crossbeam_epoch::Collector;
///
/// let collector = Collector::new();
///
/// let handle = collector.handle();
/// drop(collector); // `handle` still works after dropping `collector`
///
/// handle.pin(|scope| {
///     scope.flush();
/// });
/// ```

use std::sync::Arc;
use internal::{Global, Local};
use scope::{Scope, unprotected};

/// An epoch-based garbage collector.
pub struct Collector(Arc<Global>);

/// A handle to a garbage collector.
pub struct Handle {
    global: Arc<Global>,
    local: Local,
}

impl Collector {
    /// Creates a new collector.
    pub fn new() -> Self {
        Self { 0: Arc::new(Global::new()) }
    }

    /// Collect several bags from the global garbage queue and destroy their objects.
    ///
    /// # Safety
    ///
    /// It is assumed that no handles are concurrently accessing objects in the global garbage
    /// queue. Otherwise, the behavior is undefined. For example, if there are concurrent calls to
    /// `Collector::collect()`, then a thread may try to read from a node in the queue after it is
    /// deallocated by a concurrent thread.
    #[inline]
    pub unsafe fn collect(&self) {
        unprotected(|scope| self.0.collect(scope))
    }

    /// Creates a new handle for the collector.
    #[inline]
    pub fn handle(&self) -> Handle {
        Handle::new(self.0.clone())
    }
}

impl Handle {
    fn new(global: Arc<Global>) -> Self {
        let local = Local::new(&global);
        Self { global, local }
    }

    /// Pin the current handle.
    #[inline]
    pub fn pin<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Scope) -> R,
    {
        unsafe { self.local.pin(&self.global, f) }
    }

    /// Check if the current handle is pinned.
    #[inline]
    pub fn is_pinned(&self) -> bool {
        self.local.is_pinned()
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { self.local.unregister(&self.global) }
    }
}
