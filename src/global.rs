//! The global garbage collection realm
//!
//! This is the default realm for garbage collection.  For each thread, a handle is lazily
//! initialized on its first use, thus registering the current thread.  If initialized, the
//! thread's handle will get destructed on thread exit, which in turn unregisters the thread.
//!
//! `registries` is the list is the registered handles, and `epoch` is the global epoch.

use std::cmp;
use std::sync::atomic::Ordering::{Relaxed, SeqCst};
use handle::{Handle, LocalEpoch, Scope};
use garbage::Bag;
use epoch::Epoch;
use sync::list::List;
use sync::queue::Queue;


/// The global data for epoch-based memory reclamation.
#[derive(Debug)]
pub struct Global {
    /// The head pointer of the list of handle registries.
    pub registries: List<LocalEpoch>,
    /// A reference to the global queue of garbages.
    pub garbages: Queue<(usize, Bag)>,
    /// A reference to the global epoch.
    pub epoch: Epoch,
}

impl Global {
    /// Number of bags to destroy.
    const COLLECT_STEPS: usize = 8;

    pub fn new() -> Self {
        Global {
            registries: List::new(),
            garbages: Queue::new(),
            epoch: Epoch::new(),
        }
    }

    /// Pushes the bag onto the global queue and replaces the bag with a new empty bag.
    #[inline]
    pub fn push_bag<'scope>(&self, bag: &mut Bag, scope: &'scope Scope) {
        let epoch = self.epoch.load(Relaxed);
        let bag = ::std::mem::replace(bag, Bag::new());
        ::std::sync::atomic::fence(SeqCst);
        self.garbages.push((epoch, bag), scope);
    }

    /// Collect several bags from the global old garbage queue and destroys their objects.
    ///
    /// Note: This may itself produce garbage and in turn allocate new bags.
    ///
    /// `pin()` rarely calls `collect()`, so we want the compiler to place that call on a cold
    /// path. In other words, we want the compiler to optimize branching for the case when
    /// `collect()` is not called.
    #[cold]
    pub fn collect(&self, scope: &Scope) {
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
}


lazy_static! {
    /// The default global data.
    // FIXME(jeehoonkang): accessing globals in `lazy_static!` is blocking.
    pub static ref GLOBAL: Global = Global::new();
}

thread_local! {
    /// The thread-local handle for the default global data.
    static HANDLE: Handle<'static> = Handle::new(&GLOBAL);
}

/// Pin the current thread.
pub fn pin<F, R>(f: F) -> R
where
    F: FnOnce(&Scope) -> R,
{
    HANDLE.with(|handle| handle.pin(f))
}

/// Check if the current thread is pinned.
pub fn is_pinned() -> bool {
    HANDLE.with(|handle| handle.is_pinned())
}


#[cfg(test)]
mod tests {
    use std::thread;
    use std::sync::atomic::Ordering::Relaxed;

    use super::*;

    #[test]
    fn pin_reentrant() {
        assert!(!is_pinned());
        pin(|_| {
            pin(|_| {
                assert!(is_pinned());
            });
            assert!(is_pinned());
        });
        assert!(!is_pinned());
    }

    #[test]
    fn pin_holds_advance() {
        let threads = (0..8)
            .map(|_| {
                thread::spawn(|| for _ in 0..500_000 {
                    pin(|scope| {
                        let before = GLOBAL.epoch.load(Relaxed);
                        GLOBAL.epoch.try_advance(&GLOBAL.registries, scope);
                        let after = GLOBAL.epoch.load(Relaxed);

                        assert!(after.wrapping_sub(before) <= 2);
                    });
                })
            })
            .collect::<Vec<_>>();

        for t in threads {
            t.join().unwrap();
        }
    }
}
