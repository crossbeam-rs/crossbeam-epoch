use std::cmp;
use std::cell::{Cell, UnsafeCell};
use std::sync::atomic::Ordering::{Relaxed, SeqCst};

use atomic::Ptr;
use registry::Registry;
use epoch::Epoch;
use garbage::{Garbage, Bag};

use sync::list::{List, Node};
use sync::ms_queue::MsQueue;


/// Number of pinnings after which a thread will collect some global garbage.
const PINS_BETWEEN_COLLECT: usize = 128;

/// Number of bags to destroy.
const COLLECT_STEPS: usize = 8;


pub trait Namespace: Copy {
    fn registries(&self) -> &List<Registry>;
    fn garbages(&self) -> &MsQueue<Self, (usize, Bag)>;
    fn epoch(&self) -> &Epoch;

    #[inline]
    fn push_bag<'scope>(self, bag: &mut Bag, scope: &'scope Scope<Self>) {
        let epoch = self.epoch().load(Relaxed);
        let bag = ::std::mem::replace(bag, Bag::new());
        ::std::sync::atomic::fence(SeqCst);
        self.garbages().push((epoch, bag), scope);
    }

    /// Collect several bags from the global old garbage queue and destroys their objects.
    /// Note: This may itself produce garbage and in turn allocate new bags.
    #[inline]
    fn collect(self, scope: &Scope<Self>) {
        let epoch = self.epoch().try_advance(self.registries(), scope);
        let garbages = self.garbages();
        let condition = |bag: &(usize, Bag)| {
            // A pinned thread can witness at most one epoch advancement. Therefore, any bag that is
            // within one epoch of the current one cannot be destroyed yet.
            let diff = epoch.wrapping_sub(bag.0);
            cmp::min(diff, 0usize.wrapping_sub(diff)) > 2
        };

        for _ in 0..COLLECT_STEPS {
            match garbages.try_pop_if(&condition, scope) {
                None => break,
                Some(bag) => drop(bag)
            }
        }
    }

    #[inline]
    unsafe fn unprotected_with_bag<F, R>(self, bag: &mut Bag, f: F) -> R where
        F: FnOnce(&Scope<Self>) -> R,
    {
        let scope = &Scope { namespace: self, bag: bag };
        f(scope)
    }

    #[inline]
    unsafe fn unprotected<F, R>(self, f: F) -> R where
        F: FnOnce(&Scope<Self>) -> R,
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
    registry: &'scope Node<Registry>,
    /// The local garbage objects that will be later freed.
    bag: UnsafeCell<Bag>,
    /// Whether the thread is currently pinned.
    is_pinned: Cell<bool>,
    /// Total number of pinnings performed.
    pin_count: Cell<usize>,
}

impl<'scope, N> Agent<'scope, N> where
    N: Namespace + 'scope,
{
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
    pub fn pin<F, R>(&self, f: F) -> R where
        F: FnOnce(&Scope<N>) -> R,
    {
        let registry = self.registry.get();
        let scope = &Scope { namespace: self.namespace, bag: self.bag.get() };

        let was_pinned = self.is_pinned.get();
        if !was_pinned {
            // Increment the pin counter.
            let count = self.pin_count.get();
            self.pin_count.set(count.wrapping_add(1));

            // Pin the thread.
            self.is_pinned.set(true);
            registry.set_pinned(self.namespace);

            // If the counter progressed enough, try advancing the epoch and collecting garbage.
            if count % PINS_BETWEEN_COLLECT == 0 {
                self.namespace.collect(scope);
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

    pub fn is_pinned(&'scope self) -> bool {
        self.is_pinned.get()
    }
}

impl<'scope, N: Namespace> Drop for Agent<'scope, N> {
    fn drop(&mut self) {
        unsafe {
            let bag = &mut *self.bag.get();

            self.pin(|scope| {
                self.namespace.collect(scope);

                // Unregister the thread by marking this entry as deleted.
                self.registry.delete(scope);

                // Push the local bag into the global garbage queue.
                self.namespace.push_bag(bag, scope);
            });
        }
    }
}


#[derive(Debug)]
pub struct Scope<N: Namespace> {
    namespace: N,
    bag: *mut Bag, // !Send + !Sync
}

impl<N> Scope<N> where
    N: Namespace,
{
    unsafe fn get_bag(&self) -> &mut Bag {
        &mut *self.bag
    }

    unsafe fn defer_garbage(&self, mut garbage: Garbage) {
        let bag = self.get_bag();

        while let Err(g) = bag.try_push(garbage) {
            self.namespace.push_bag(bag, self);
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
            if !bag.is_empty() {
                self.namespace.push_bag(bag, self);
            }
        }

        self.namespace.collect(self);
    }
}
