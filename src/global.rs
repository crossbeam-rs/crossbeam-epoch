use std::cmp;
use std::sync::atomic::{AtomicUsize, ATOMIC_USIZE_INIT};
use std::sync::atomic::Ordering::{Relaxed, Acquire, Release, SeqCst};

use garbage::Bag;
use registry::Registry;
use scope::Scope;
use sync::list::IterResult;
use sync::queue::Queue;


/// Number of bags to destroy.
const COLLECT_STEPS: usize = 8;

/// The global epoch.
pub static EPOCH: AtomicUsize = ATOMIC_USIZE_INIT;


/// Attempts to advance the global epoch.
///
/// The global epoch can advance only if all currently pinned threads have been pinned in the
/// current epoch.
///
/// Returns the current global epoch.
#[cold]
pub fn try_advance(scope: &Scope) -> usize {
    let epoch = EPOCH.load(Relaxed);
    ::std::sync::atomic::fence(SeqCst);

    // Traverse the linked list of thread registries.
    let mut registries = Registry::list().iter(scope);
    loop {
        match registries.next() {
            IterResult::Abort => {
                // We leave the job to the thread that also tries to advance to epoch and continues
                // to iterate the registries.
                return epoch;
            },
            IterResult::None => break,
            IterResult::Some(registry) => {
                let (thread_is_pinned, thread_epoch) = registry.get_state();

                // If the thread was pinned in a different epoch, we cannot advance the global epoch
                // just yet.
                if thread_is_pinned && thread_epoch != epoch {
                    return epoch;
                }
            }
        }
    }
    ::std::sync::atomic::fence(Acquire);

    // All pinned threads were pinned in the current global epoch.
    // Try advancing the epoch. We increment by 2 and simply wrap around on overflow.
    let epoch_new = epoch.wrapping_add(2);
    EPOCH.store(epoch_new, Release);
    epoch_new
}

/// Collects several bags from the global old garbage queue and destroys their objects.
pub fn collect(epoch: usize, scope: &Scope) {
    let queue = garbages();

    let condition = |bag: &(usize, Bag)| {
        // A pinned thread can witness at most one epoch advancement. Therefore, any bag that is
        // within one epoch of the current one cannot be destroyed yet.
        let diff = epoch.wrapping_sub(bag.0);
        cmp::min(diff, 0usize.wrapping_sub(diff)) > 2
    };

    for _ in 0..COLLECT_STEPS {
        match queue.try_pop_if(&condition, scope) {
            None => break,
            Some(bag) => drop(bag)
        }
    }
}

/// Migrates garbages to the global queues.
pub fn migrate_bag(bag: &mut Bag) {
    let bag = ::std::mem::replace(bag, Bag::new());
    let epoch = EPOCH.load(Relaxed);
    ::std::sync::atomic::fence(SeqCst);
    garbages().push((epoch, bag));
}

/// Returns a reference to the global garbage queue, which is lazily initialized.
fn garbages() -> &'static Queue<(usize, Bag)> {
    static GLOBAL: AtomicUsize = ATOMIC_USIZE_INIT;

    let current = GLOBAL.load(Acquire);

    let garbage = if current == 0 {
        // Initialize the singleton.
        let raw = Box::into_raw(Box::new(Queue::<(usize, Bag)>::new()));
        let new = raw as usize;
        let previous = GLOBAL.compare_and_swap(0, new, Release);

        if previous == 0 {
            // Ok, we initialized it.
            new
        } else {
            // Another thread has already initialized it.
            unsafe { drop(Box::from_raw(raw)); }
            previous
        }
    } else {
        current
    };

    unsafe { &*(garbage as *const Queue<(usize, Bag)>) }
}
