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


/// Garbage.
#[derive(Default)]
pub struct Garbage {
    _private: (),
}


/// Bag of garbages.
#[derive(Default)]
pub struct Bag {
    _private: (),
}

impl Bag {
    /// Returns a new, empty bag.
    pub fn new() -> Self {
        Self::default()
    }
}
