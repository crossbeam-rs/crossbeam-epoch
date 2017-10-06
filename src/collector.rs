//! The garbage collector.
//!
//! # Registration
//!
//! In order to track all handles in one place, we need some form of handle registration. When a
//! handle is created, it is registered to a global lock-free singly-linked list of registries; and
//! when a handle is dropped, it is unregistered from the list.

use std::cmp;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use handle::{Handle, LocalEpoch, Scope, unprotected};
use garbage::Bag;
use epoch::Epoch;
use sync::list::{List, Node};
use sync::queue::Queue;


/// A garbage collector.
///
/// # Examples
///
/// ```
/// use crossbeam_epoch as epoch;
///
/// let collector = epoch::Collector::new();
///
/// let handle = collector.handle();
/// handle.pin(|scope| {
///     scope.flush();
/// });
/// ```
#[derive(Debug)]
pub struct Collector(Arc<Global>);

/// The global data for a garbage collector.
#[derive(Debug)]
pub struct Global {
    /// The head pointer of the list of handle registries.
    registries: List<LocalEpoch>,
    /// A reference to the global queue of garbages.
    garbages: Queue<(usize, Bag)>,
    /// A reference to the global epoch.
    epoch: Epoch,
}

impl Collector {
    /// Create a collector.
    pub fn new() -> Self {
        Self { 0: Arc::new(Global::new()) }
    }

    /// Create a new handle for the collector.
    pub fn handle(&self) -> Handle {
        Handle::new(&self.0)
    }

    /// Collect several bags from the global garbage queue and destroy their objects.
    ///
    /// # Safety
    ///
    /// It is assumed that no handles are concurrently accessing objects in the global garbage
    /// queue. Otherwise, the behavior is undefined.
    #[inline]
    pub unsafe fn collect<'scope>(&'scope self) {
        unprotected(|scope| self.0.collect(scope))
    }
}

impl Global {
    /// Number of bags to destroy.
    const COLLECT_STEPS: usize = 8;

    fn new() -> Self {
        Self {
            registries: List::new(),
            garbages: Queue::new(),
            epoch: Epoch::new(),
        }
    }

    #[inline]
    pub fn get_epoch(&self) -> usize {
        self.epoch.load(Ordering::Relaxed)
    }

    /// Pushes the bag onto the global queue and replaces the bag with a new empty bag.
    #[inline]
    pub fn push_bag<'scope>(&self, bag: &mut Bag, scope: Scope<'scope>) {
        let epoch = self.epoch.load(Ordering::Relaxed);
        let bag = ::std::mem::replace(bag, Bag::new());
        ::std::sync::atomic::fence(Ordering::SeqCst);
        self.garbages.push((epoch, bag), scope);
    }

    /// Collect several bags from the global garbage queue and destroy their objects.
    ///
    /// Note: This may itself produce garbage and in turn allocate new bags.
    ///
    /// `pin()` rarely calls `collect()`, so we want the compiler to place that call on a cold
    /// path. In other words, we want the compiler to optimize branching for the case when
    /// `collect()` is not called.
    #[cold]
    pub fn collect<'scope>(&'scope self, scope: Scope<'scope>) {
        let epoch = self.epoch.try_advance(&self.registries, scope);

        let condition = |bag: &(usize, Bag)| {
            // A pinned thread can witness at most one epoch advancement. Therefore, any bag that is
            // within one epoch of the current one cannot be destroyed yet.
            let diff = epoch.wrapping_sub(bag.0);
            cmp::min(diff, 0usize.wrapping_sub(diff)) > 2
        };

        for _ in 0..Self::COLLECT_STEPS {
            match self.garbages.try_pop_if(&condition, scope) {
                None => break,
                Some(bag) => drop(bag),
            }
        }
    }

    /// Register a handle.
    pub fn register(&self) -> *const Node<LocalEpoch> {
        unsafe {
            // Since we dereference no pointers in this block, it is safe to use `unprotected`.
            unprotected(|scope| {
                self.registries.insert(LocalEpoch::new(), scope).as_raw()
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use crossbeam_utils::scoped;

    use super::*;

    const NUM_THREADS: usize = 8;

    #[test]
    fn pin_holds_advance() {
        let collector = Collector::new();

        let threads = (0..NUM_THREADS)
            .map(|_| {
                scoped::scope(|scope| {
                    scope.spawn(|| for _ in 0..100_000 {
                        let handle = collector.handle();
                        handle.pin(|_| {
                            let before = collector.0.get_epoch();
                            unsafe {
                                collector.collect();
                            }
                            let after = collector.0.get_epoch();

                            assert!(after.wrapping_sub(before) <= 2);
                        });
                    })
                })
            })
            .collect::<Vec<_>>();
        drop(collector);

        for t in threads {
            t.join();
        }
    }
}
