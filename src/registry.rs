use std::sync::atomic::{AtomicUsize, ATOMIC_USIZE_INIT};
use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};

use {Atomic, Owned, Ptr, Scope, unprotected};
use participant::Participant;

// FIXME(stjepang): Participants are stored in a linked list because linked lists are fairly easy
// to implement in a lock-free manner. However, traversal is rather slow due to cache misses and
// data dependencies. We should experiment with other data structures as well.

/// An entry in the linked list of participating threads.
struct Entry {
    /// State of the participating thread.
    participant: Participant,

    /// The next entry in the linked list.
    /// If the tag is 1, this entry is marked as deleted.
    next: Atomic<Entry>,
}

/// Registers a thread by inserting a new entry into the list of participating threads.
///
/// Returns a pointer to the newly allocated and inserted entry.
fn register() -> *const Entry {
    let list = participants();

    let mut new = Owned::new(Entry {
        participant: Participant::new(),
        next: Atomic::null(),
    });

    unsafe {
        // Since we don't dereference any pointers in this block, it's okay to use `unprotected`.
        unprotected(|scope| {
            let mut head = list.load(Acquire, scope);

            loop {
                new.next.store(head, Relaxed);

                // Try installing the new entry as the new head.
                match list.compare_and_set_weak_owned(head, new, AcqRel, scope) {
                    Ok(n) => return n.as_raw(),
                    Err((h, n)) => {
                        head = h;
                        new = n;
                    }
                }
            }
        })
    }
}

thread_local! {
    /// The thread registration harness.
    ///
    /// The harness is lazily initialized on its first use, thus registrating the current thread.
    /// If initialized, the harness will get destructed on thread exit, which in turn unregisters
    /// the thread.
    static HARNESS: Harness = Harness {
        entry: register(),
    };
}

/// Holds a registered entry and unregisters it when dropped.
struct Harness {
    entry: *const Entry,
}

impl Drop for Harness {
    fn drop(&mut self) {
        unsafe {
            let entry = &*self.entry;

            // Unregister the thread by marking this entry as deleted.
            unprotected(|scope| { entry.next.fetch_or(1, Release, scope); });
        }
    }
}

/// Returns a reference to the head pointer of the list of participating threads.
fn participants() -> &'static Atomic<Entry> {
    static PARTICIPANTS: AtomicUsize = ATOMIC_USIZE_INIT;
    unsafe { &*(&PARTICIPANTS as *const AtomicUsize as *const Atomic<Entry>) }
}

/// Acquires a reference to the current participant.
///
/// Participants are lazily initialized on the first use.
///
/// # Panics
///
/// If this function is called while the thread is exiting, it might panic because it accesses
/// thread-local data.
pub fn with_current<F, R>(f: F) -> R
where
    F: FnOnce(&Participant) -> R,
{
    HARNESS.with(|harness| {
        let entry = unsafe { &*harness.entry };
        f(&entry.participant)
    })
}

/// Returns an iterator over all participating threads.
///
/// Note that the iterator might return the same participant multiple times.
pub fn iter(scope: &Scope) -> Iter {
    let pred = participants();
    let curr = pred.load(Acquire, scope);
    Iter { scope, pred, curr }
}

/// An iterator over all participating threads.
///
/// Note that the iterator might return the same participant multiple times.
pub struct Iter<'scope> {
    /// The scope in which the iterator is operating.
    scope: &'scope Scope,

    /// Pointer from the predecessor to the current entry.
    pred: &'scope Atomic<Entry>,

    /// The current entry.
    curr: Ptr<'scope, Entry>,
}

impl<'scope> Iterator for Iter<'scope> {
    type Item = &'scope Participant;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(c) = unsafe { self.curr.as_ref() } {
            let succ = c.next.load(Acquire, self.scope);

            if succ.tag() == 1 {
                // This thread has exited. Try unlinking it from the list.
                let succ = succ.with_tag(0);

                if self.pred
                    .compare_and_set(self.curr, succ, AcqRel, self.scope)
                    .is_err()
                {
                    // We lost the race to unlink this entry. There's no option other than to
                    // restart traversal from the beginning.
                    *self = iter(self.scope);
                    continue;
                }

                // FIXME(stjepang): Garbage-collect the unlinked entry `curr`.

                // Move forward, but don't change the predecessor.
                self.curr = succ;
            } else {
                // Move one step forward.
                self.pred = &c.next;
                self.curr = succ;

                return Some(&c.participant);
            }
        }

        // We reached the end of the list.
        None
    }
}
