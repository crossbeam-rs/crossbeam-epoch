//! Garbage collection.
//!
//! # Garbages
//!
//! Objects that get unlinked from concurrent data structures must be stashed away until the global
//! epoch sufficiently advances so that they become safe for destruction.  We call these objects
//! garbages.  When the global epoch advances sufficiently, `Destroy` garbages are dropped (i.e. the
//! destructors are called), and `Free` garbages are freed.  In addition, you can register arbitrary
//! function to be called later using the `Fn` garbages.
//!
//! # Bags
//!
//! Pointers to such garbages are pushed into thread-local bags, and when it becomes full, the bag
//! is marked with the current global epoch and pushed into a global queue of garbage bags.  We
//! store garbages in thread-local storages for amortizing the synchronization cost of pushing the
//! garbages to a global queue.
//!
//! # Garbage queues
//!
//! Whenever a bag is pushed into a queue, some garbage in the queue is collected and destroyed
//! along the way.  Garbage collection can also be manually triggered by calling `collect()`.  This
//! design reduces contention on data structures.  Ideally each instance of concurrent data
//! structure may have its own queue that gets fully destroyed as soon as the data structure gets
//! dropped.

use std::mem;
use arrayvec::ArrayVec;
use deferred::Deferred;

/// Maximum number of objects a bag can contain.
#[cfg(not(feature = "strict_gc"))]
const MAX_OBJECTS: usize = 64;
#[cfg(feature = "strict_gc")]
const MAX_OBJECTS: usize = 4;


pub struct Garbage {
    func: Deferred,
}

unsafe impl Sync for Garbage {}
unsafe impl Send for Garbage {}

impl Garbage {
    /// Make a garbage object that will later be destroyed using `destroy`.
    ///
    /// The specified object is an array allocated at address `object` and consists of `size`
    /// elements of type `T`.
    ///
    /// Note: The object must be `Send + 'static`.
    pub fn new_destroy<T>(object: *mut T, size: usize, destroy: unsafe fn(*mut T, usize)) -> Self {
        let object = object as usize;
        let destroy: unsafe fn(*mut u8, usize) = unsafe { mem::transmute(destroy) };
        Garbage {
            func: Deferred::new(move || unsafe {
                destroy(object as *mut u8, size);
            }),
        }
    }

    /// Make a garbage object that will later be freed.
    ///
    /// The specified object is an array allocated at address `object` and consists of `size`
    /// elements of type `T`.
    pub fn new_free<T>(object: *mut T, size: usize) -> Self {
        let object = object as usize;
        Garbage {
            func: Deferred::new(move || unsafe {
                drop(Vec::from_raw_parts(object as *mut T, 0, size));
            }),
        }
    }

    /// Make a garbage object that will later be dropped and freed.
    ///
    /// The specified object is an array allocated at address `object` and consists of `size`
    /// elements of type `T`.
    ///
    /// Note: The object must be `Send + 'static`.
    pub fn new_drop<T>(object: *mut T, size: usize) -> Self {
        unsafe fn destruct<T>(object: *mut T, size: usize) {
            // Run the destructors and free the memory.
            drop(Vec::from_raw_parts(object, size, size));
        }
        Self::new_destroy(object, size, destruct)
    }

    /// Make a closure that will later be called.
    pub fn new<F: FnOnce() + Send>(f: F) -> Self {
        Garbage {
            func: Deferred::new(move || f()),
        }
    }
}

impl Drop for Garbage {
    fn drop(&mut self) {
        self.func.call();
    }
}


/// Bag of garbages.
#[derive(Default)]
pub struct Bag {
    /// Removed objects.
    objects: ArrayVec<[Garbage; MAX_OBJECTS]>,
}

impl Bag {
    /// Returns a new, empty bag.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if the bag is empty.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Attempts to insert a garbage object into the bag and returns `true` if succeeded.
    pub fn try_push(&mut self, garbage: Garbage) -> Result<(), Garbage> {
        self.objects.try_push(garbage).map_err(|e| e.element())
    }
}
