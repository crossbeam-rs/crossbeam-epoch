//! Garbage collection.
//!
//! # Garbages
//!
//! FIXME(jeehoonkang): fill it
//!
//! # Bags
//!
//! Objects that get unlinked from concurrent data structures must be stashed away until the global
//! epoch sufficiently advances so that they become safe for destruction.  For that purposes, each
//! thread has a thread-local bag that is populated with pointers to such garbage objects, and when
//! it becomes full, the bag is marked with the current global epoch and pushed into a global queue
//! of garbage bags.
//!
//! # Garbage queues
//!
//! Whenever a bag is pushed into a queue, some garbage in the queue is collected and destroyed
//! along the way.  Garbage collection can also be manually triggered by calling `collect()`.  This
//! design reduces contention on data structures.  Ideally each instance of concurrent data
//! structure may have it's own queue that gets fully destroyed as soon as the data structure gets
//! dropped.
//!
//! # The global garbage bag queue
//!
//! However, some data structures don't own objects but merely transfer them between threads,
//! e.g. queues.  As such, queues don't execute destructors - they only allocate and free some
//! memory. it would be costly for each queue to handle it's own queue, so there is a special global
//! queue all data structures can share.

use std::cmp;
use std::mem;
use boxfnonce::SendBoxFnOnce;
use std::sync::atomic::Ordering::SeqCst;

use scope::{Namespace, Scope};
use sync::queue::Queue;


/// Maximum number of objects a bag can contain.
#[cfg(not(feature = "strict_gc"))]
const MAX_OBJECTS: usize = 64;
#[cfg(feature = "strict_gc")]
const MAX_OBJECTS: usize = 4;

/// Number of bags to destroy.
const COLLECT_STEPS: usize = 8;


pub enum Garbage {
    Destroy { object: *mut u8, size: usize, destroy: unsafe fn(*mut u8, usize), },
    Fn { f: Option<SendBoxFnOnce<(), ()>>, },
}

impl Garbage {
    /// Make a garbage object that will later be destroyed using `destroy`.
    ///
    /// The specified object is an array allocated at address `object` and consists of `size`
    /// elements of type `T`.
    ///
    /// Note: The object must be `Send + 'self`.
    pub fn new_destroy<T>(object: *mut T, size: usize, destroy: unsafe fn(*mut T, usize)) -> Self {
        Garbage::Destroy {
            object: object as *mut u8,
            size: size,
            destroy: unsafe { mem::transmute(destroy) },
        }
    }

    /// Make a garbage object that will later be freed.
    ///
    /// The specified object is an array allocated at address `object` and consists of `size`
    /// elements of type `T`.
    pub fn new_free<T>(object: *mut T, size: usize) -> Self {
        unsafe fn free<T>(object: *mut T, size: usize) {
            // Free the memory, but don't run the destructors.
            drop(Vec::from_raw_parts(object, 0, size));
        }
        Self::new_destroy(object, size, free)
    }

    /// Make a garbage object that will later be dropped and freed.
    ///
    /// The specified object is an array allocated at address `object` and consists of `count`
    /// elements of type `T`.
    ///
    /// Note: The object must be `Send + 'self`.
    pub fn new_drop<T>(object: *mut T, size: usize) -> Self {
        unsafe fn destruct<T>(object: *mut T, size: usize) {
            // Run the destructors and free the memory.
            drop(Vec::from_raw_parts(object, size, size));
        }
        Self::new_destroy(object, size, destruct)
    }

    /// Make a closure that will later be called.
    pub fn new<F: FnOnce() + Send + 'static>(f: F) -> Self {
        Garbage::Fn { f: Some(SendBoxFnOnce::from(f)) }
    }
}

impl Drop for Garbage {
    fn drop(&mut self) {
        match self {
            &mut Garbage::Destroy { destroy, object, size } => {
                unsafe { (destroy)(object, size); }
            },
            &mut Garbage::Fn { ref mut f } => {
                let f = f.take().unwrap();
                f.call();
            },
        }
    }
}


/// Bag of garbages.
pub struct Bag {
    /// Number of objects in the bag.
    len: usize,
    /// Removed objects.
    objects: [Garbage; MAX_OBJECTS],
}

impl Bag {
    /// Returns a new, empty bag.
    pub fn new() -> Self {
        Bag {
            len: 0,
            objects: unsafe { mem::zeroed() },
        }
    }

    /// Returns `true` if the bag is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Attempts to insert a garbage object into the bag and returns `true` if succeeded.
    pub fn try_insert(&mut self, garbage: Garbage) -> Result<(), Garbage> {
        if self.len == self.objects.len() {
            return Err(garbage);
        }

        self.objects[self.len] = garbage;
        self.len += 1;
        Ok(())
    }
}

impl Drop for Bag {
    fn drop(&mut self) {
        for garbage in self.objects.into_iter().take(self.len) {
            drop(garbage)
        }
    }
}

impl Queue<(usize, Bag)> {
    /// Collects several bags from the global old garbage queue and destroys their objects.
    pub fn collect<N: Namespace>(&self, epoch: usize, scope: &Scope<N>) {
        let condition = |bag: &(usize, Bag)| {
            // A pinned thread can witness at most one epoch advancement. Therefore, any bag that is
            // within one epoch of the current one cannot be destroyed yet.
            let diff = epoch.wrapping_sub(bag.0);
            cmp::min(diff, 0usize.wrapping_sub(diff)) > 2
        };

        for _ in 0..COLLECT_STEPS {
            match self.try_pop_if(&condition, scope) {
                None => break,
                Some(bag) => drop(bag)
            }
        }
    }

    /// Migrates garbages to the global queues.
    pub fn migrate_bag(&self, epoch: usize, bag: &mut Bag) {
        let bag = ::std::mem::replace(bag, Bag::new());
        ::std::sync::atomic::fence(SeqCst);
        self.push((epoch, bag));
    }
}
