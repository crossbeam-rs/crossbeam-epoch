use std::cell::{Cell, UnsafeCell};
use std::sync::atomic::Ordering::Relaxed;

use atomic::Ptr;
use registry::Registry;
use epoch::Epoch;
use garbage::{Garbage, Bag};
use global;

use sync::list::{List, ListEntry};
use sync::queue::Queue;


/// Number of pinnings after which a thread will collect some global garbage.
const PINS_BETWEEN_COLLECT: usize = 128;


pub trait Namespace: Copy {
    fn epoch(&self) -> &Epoch;
    fn garbages(&self) -> &Queue<(usize, Bag)>;
    fn registries(&self) -> &List<Registry>;

    unsafe fn unprotected_with_bag<F, R>(self, bag: &mut Bag, f: F) -> R
        where F: FnOnce(&Scope<Self>) -> R,
    {
        let scope = &Scope { namespace: self, bag: bag };
        f(scope)
    }

    unsafe fn unprotected<F, R>(self, f: F) -> R
        where F: FnOnce(&Scope<Self>) -> R,
    {
        let mut bag = Bag::new();
        let result = self.unprotected_with_bag(&mut bag, f);
        drop(bag); // FIXME(jeehoonkang)
        result
    }
}


pub struct Agent<'scope, N: Namespace + 'scope> {
    /// This agent's namespace
    namespace: N,
    /// This agent's entry in the registry list.
    registry: &'scope ListEntry<Registry>,
    /// The local garbage objects that will be later freed.
    bag: UnsafeCell<Bag>,
    /// Whether the thread is currently pinned.
    is_pinned: Cell<bool>,
    /// Total number of pinnings performed.
    pin_count: Cell<usize>,
}

impl<'scope, N: Namespace> Drop for Agent<'scope, N> {
    fn drop(&mut self) {
        unsafe {
            let bag = &mut *self.bag.get();

            global::unprotected_with_bag(bag, |scope| {
                // Spare some cycles on garbage collection.
                // Note: This may itself produce garbage and in turn allocate new bags.
                let epoch = self.namespace.epoch().try_advance(self.namespace.registries(), scope);
                self.namespace.garbages().collect(epoch, scope);

                // Unregister the thread by marking this entry as deleted.
                self.registry.delete(scope);
            });

            // Push the local bag into the global garbage queue.
            let epoch = self.namespace.epoch().load(Relaxed);
            self.namespace.garbages().migrate_bag(epoch, bag);
        }
    }
}

impl<'scope, N: Namespace> Agent<'scope, N> {
    pub fn new(n: N) -> Self {
        Agent {
            namespace: n,
            registry: n.registries().register(),
            bag: UnsafeCell::new(Bag::new()),
            is_pinned: Cell::new(false),
            pin_count: Cell::new(0),
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
    pub fn pin<F, R>(&self, f: F) -> R
        where F: FnOnce(&Scope<N>) -> R,
    {
        let registry = self.registry.get();
        let scope = &Scope { namespace: self.namespace, bag: self.bag.get() };

        let was_pinned = self.is_pinned.get();
        if !was_pinned {
            // Pin the thread.
            self.is_pinned.set(true);
            let epoch = self.namespace.epoch().load(Relaxed);
            registry.set_pinned(epoch);

            // Increment the pin counter.
            let count = self.pin_count.get();
            self.pin_count.set(count.wrapping_add(1));

            // If the counter progressed enough, try advancing the epoch and collecting garbage.
            if count % PINS_BETWEEN_COLLECT == 0 {
                let epoch = self.namespace.epoch().try_advance(self.namespace.registries(), scope);
                self.namespace.garbages().collect(epoch, scope);
            }
        }

        // This will unpin the thread even if `f` panics.
        defer! {
            if !was_pinned {
                // Unpin the thread.
                registry.set_unpinned();
                self.is_pinned.set(false);
            }
        }

        f(scope)
    }

    pub fn is_pinned(&self) -> bool {
        self.is_pinned.get()
    }
}


#[derive(Debug)]
pub struct Scope<N: Namespace> {
    namespace: N,
    bag: *mut Bag, // !Send + !Sync
}

impl<N: Namespace> Scope<N> {
    unsafe fn get_bag(&self) -> &mut Bag {
        &mut *self.bag
    }

    unsafe fn defer_garbage(&self, mut garbage: Garbage) {
        let bag = self.get_bag();

        while let Err(g) = bag.try_insert(garbage) {
            let epoch = self.namespace.epoch().load(Relaxed);
            self.namespace.garbages().migrate_bag(epoch, bag);
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
            let epoch = self.namespace.epoch().load(Relaxed);
            self.namespace.garbages().migrate_bag(epoch, bag);
        }

        // Spare some cycles on garbage collection.
        // Note: This may itself produce garbage and allocate new bags.
        let epoch = self.namespace.epoch().try_advance(self.namespace.registries(), self);
        self.namespace.garbages().collect(epoch, self);
    }
}
