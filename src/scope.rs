use std::cell::{Cell, UnsafeCell};

use atomic::Ptr;
use garbage::{Garbage, Bag};
use global;
use registry::Registry;
use sync::list::ListEntry;


/// Number of pinnings after which a thread will collect some global garbage.
const PINS_BETWEEN_COLLECT: usize = 128;

thread_local! {
    /// The thread registration harness.
    ///
    /// The harness is lazily initialized on its first use, thus registrating the current thread.
    /// If initialized, the harness will get destructed on thread exit, which in turn unregisters
    /// the thread.
    static HARNESS: Harness = Harness {
        registry: Registry::register(),
        bag: UnsafeCell::new(Bag::new()),
        is_pinned: Cell::new(false),
        pin_count: Cell::new(0),
    }
}

struct Harness {
    /// This thread's entry in the registry list.
    registry: *const ListEntry<Registry>,
    /// The local garbage objects that will be later freed.
    bag: UnsafeCell<Bag>,
    /// Whether the thread is currently pinned.
    is_pinned: Cell<bool>,
    /// Total number of pinnings performed.
    pin_count: Cell<usize>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        unsafe {
            let registry = &*self.registry;
            let bag = &mut *self.bag.get();

            unprotected_with_bag(bag, |scope| {
                // Spare some cycles on garbage collection.
                // Note: This may itself produce garbage and in turn allocate new bags.
                global::try_advance(scope);
                global::collect(scope);

                // Unregister the thread by marking this entry as deleted.
                registry.delete(scope);
            });

            // Push the local bag into the global garbage queue.
            global::migrate_bag(bag);
        }
    }
}


#[derive(Debug)]
pub struct Scope {
    bag: *mut Bag, // !Send + !Sync
}

impl Scope {
    unsafe fn get_bag(&self) -> &mut Bag {
        &mut *self.bag
    }

    unsafe fn defer_garbage(&self, mut garbage: Garbage) {
        let bag = self.get_bag();

        while let Err(g) = bag.try_insert(garbage) {
            global::migrate_bag(bag);
            garbage = g;
        }
    }

    // Deferred deallocation of heap-allocated object `ptr`.
    pub unsafe fn defer_free<T>(&self, ptr: Ptr<T>) {
        self.defer_garbage(Garbage::new_free(ptr.as_raw() as *mut T, 1))
    }

    // Deferred destruction and deallocation of heap-allocated object `ptr`.
    pub unsafe fn defer_drop<T: Send + 'static>(&self, ptr: Ptr<T>) {
        self.defer_garbage(Garbage::new_drop(ptr.as_raw() as *mut T, 1))
    }

    // Deferred execution of arbitrary function `f`.
    pub unsafe fn defer<F: FnOnce() + Send + 'static>(&self, f: F) {
        self.defer_garbage(Garbage::new(f))
    }

    pub fn flush(&self) {
        unsafe {
            let bag = self.get_bag();
            if bag.is_empty() { return; }
            global::migrate_bag(bag);
        }

        // Spare some cycles on garbage collection.
        // Note: This may itself produce garbage and allocate new bags.
        global::try_advance(self);
        global::collect(self);
    }
}

/// Pins the current thread.
///
/// The provided function takes a reference to a `Scope`, which can be used to interact with
/// [`Atomic`]s. The scope serves as a proof that whatever data you load from an [`Atomic`] will
/// not be concurrently deleted by another thread while the scope is alive.
///
/// Note that keeping a thread pinned for a long time prevents memory reclamation of any newly
/// deleted objects protected by [`Atomic`]s. The provided function should be very quick -
/// generally speaking, it shouldn't take more than 100 ms.
///
/// Pinning is reentrant. There is no harm in pinning a thread while it's already pinned (repinning
/// is essentially a noop).
///
/// Pinning itself comes with a price: it begins with a `SeqCst` fence and performs a few other
/// atomic operations. However, this mechanism is designed to be as performant as possible, so it
/// can be used pretty liberally. On a modern machine pinning takes 10 to 15 nanoseconds.
///
/// [`Atomic`]: struct.Atomic.html
pub fn pin<F, R>(f: F) -> R
    where F: FnOnce(&Scope) -> R,
{
    HARNESS.with(|harness| {
        let registry = unsafe { (&*harness.registry).get() };
        let scope = &Scope { bag: harness.bag.get() };

        let was_pinned = harness.is_pinned.get();
        if !was_pinned {
            // Pin the thread.
            harness.is_pinned.set(true);
            registry.set_pinned(scope);

            // Increment the pin counter.
            let count = harness.pin_count.get();
            harness.pin_count.set(count.wrapping_add(1));

            // If the counter progressed enough, try advancing the epoch and collecting garbage.
            if count % PINS_BETWEEN_COLLECT == 0 {
                global::try_advance(scope);
                global::collect(scope);
            }
        }

        // This will unpin the thread even if `f` panics.
        defer! {
            if !was_pinned {
                // Unpin the thread.
                registry.set_unpinned();
                harness.is_pinned.set(false);
            }
        }

        f(scope)
    })
}

pub fn is_pinned() -> bool {
    HARNESS.with(|harness| {
        harness.is_pinned.get()
    })
}

pub unsafe fn unprotected_with_bag<F, R>(bag: &mut Bag, f: F) -> R
    where F: FnOnce(&Scope) -> R,
{
    let scope = &Scope { bag };
    f(scope)
}


pub unsafe fn unprotected<F, R>(f: F) -> R
    where F: FnOnce(&Scope) -> R,
{
    let mut bag = Bag::new();
    let result = unprotected_with_bag(&mut bag, f);
    global::migrate_bag(&mut bag);
    result
}
