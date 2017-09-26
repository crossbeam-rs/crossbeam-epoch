//! The garbage collector.
//!
//! # Registration
//!
//! In order to track all mutators in one place, we need some form of mutator registration. When a
//! mutator is created, it is registered to a global lock-free singly-linked list of registries; and
//! when a mutator is dropped, it is unregistered from the list.

use std::cell::{Cell, UnsafeCell};
use std::cmp;
use std::sync::atomic::Ordering::{Relaxed, SeqCst};
use mutator::{Mutator, LocalEpoch, Scope, unprotected};
use garbage::Bag;
use epoch::Epoch;
use sync::list::List;
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
/// let mutator = collector.add_mutator();
/// mutator.pin(|scope| {
///     scope.flush();
/// });
/// ```
#[derive(Debug)]
pub struct Collector {
    /// The head pointer of the list of mutator registries.
    registries: List<LocalEpoch>,
    /// A reference to the global queue of garbages.
    garbages: Queue<(usize, Bag)>,
    /// A reference to the global epoch.
    epoch: Epoch,
}

impl Collector {
    /// Number of bags to destroy.
    const COLLECT_STEPS: usize = 8;

    pub fn new() -> Self {
        Collector {
            registries: List::new(),
            garbages: Queue::new(),
            epoch: Epoch::new(),
        }
    }

    /// Get the global epoch.
    #[inline]
    pub fn get_epoch(&self) -> usize {
        self.epoch.load(Relaxed)
    }

    /// Pushes the bag onto the global queue and replaces the bag with a new empty bag.
    #[inline]
    pub fn push_bag<'scope>(&self, bag: &mut Bag, scope: &'scope Scope) {
        let epoch = self.epoch.load(Relaxed);
        let bag = ::std::mem::replace(bag, Bag::new());
        ::std::sync::atomic::fence(SeqCst);
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

    /// Add a mutator.
    pub fn add_mutator<'scope>(&'scope self) -> Mutator<'scope> {
        let local_epoch = unsafe {
            // Since we dereference no pointers in this block, it is safe to use `unprotected`.
            unprotected(|scope| {
                &*self.registries
                    .insert_head(LocalEpoch::new(), scope)
                    .as_raw()
            })
        };

        Mutator {
            collector: self,
            bag: UnsafeCell::new(Bag::new()),
            local_epoch: local_epoch,
            is_pinned: Cell::new(false),
            pin_count: Cell::new(0),
        }
    }
}
