//! Mutator: an entity that changes shared locations
//!
//! # Registration
//!
//! In order to track all mutators in one place, we need some form of mutator registration. When a
//! mutator is created, it is registered to a global lock-free singly-linked list of registries; and
//! when a mutator is dropped, it is unregistered from the list.
//!
//! # Pinning
//!
//! Every registry contains an integer that tells whether the mutator is pinned and if so, what was
//! the global epoch at the time it was pinned. Mutators also hold a pin counter that aids in
//! periodic global epoch advancement.
//!
//! When a mutator is pinned, a `Scope` is returned as a witness that the mutator is pinned.  Scopes
//! are necessary for performing atomic operations, and for freeing/dropping locations.

use std::cmp;
use std::cell::{Cell, UnsafeCell};
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::{Relaxed, Release, SeqCst};

use atomic::Ptr;
use garbage::{Garbage, Bag};
use epoch::Epoch;
use global;
use sync::list::{List, Node};
use sync::ms_queue::MsQueue;


/// Number of pinnings after which a mutator will collect some global garbage.
const PINS_BETWEEN_COLLECT: usize = 128;

/// Number of bags to destroy.
const COLLECT_STEPS: usize = 8;


/// Garbage collection realm
pub trait Realm: Copy {
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
                Some(bag) => drop(bag),
            }
        }
    }

    /// Returns a [`Scope`] without pinning any mutator.
    ///
    /// Sometimes, we'd like to have longer-lived scopes in which we know our thread is the only one
    /// accessing atomics. This is true e.g. when destructing a big data structure, or when
    /// constructing it from a long iterator. In such cases we don't need to be overprotective
    /// because there is no fear of other threads concurrently destroying objects.
    ///
    /// Function `unprotected` is *unsafe* because we must promise that no other thread is accessing
    /// the Atomics and objects at the same time. The function is safe to use only if (1) the
    /// locations that we access should not be deallocated by concurrent mutators, and (2) the
    /// locations that we deallocate should not be accessed by concurrent mutators.
    ///
    /// Just like with the safe epoch::pin function, unprotected use of atomics is enclosed within a
    /// scope so that pointers created within it don't leak out or get mixed with pointers from
    /// other scopes.
    #[inline]
    unsafe fn unprotected<F, R>(self, f: F) -> R
    where
        F: FnOnce(&Scope<Self>) -> R,
    {
        let mut bag = Bag::new();
        let result = self.unprotected_with_bag(&mut bag, f);
        drop(bag);
        result
    }

    /// Returns a [`Scope`] without pinning any mutator, with arbitrary bag.
    #[inline]
    unsafe fn unprotected_with_bag<F, R>(self, bag: &mut Bag, f: F) -> R
    where
        F: FnOnce(&Scope<Self>) -> R,
    {
        let scope = &Scope {
            realm: self,
            bag: bag,
        };
        f(scope)
    }
}


/// Entity that changes shared locations.
pub struct Mutator<'scope, N: Realm + 'scope> {
    /// This mutator's realm
    realm: N,
    /// This mutator's entry in the registry list.
    registry: &'scope Node<Registry>,
    /// The local garbage objects that will be later freed.
    bag: UnsafeCell<Bag>,
    /// Whether the thread is currently pinned.
    is_pinned: Cell<bool>,
    /// Total number of pinnings performed.
    pin_count: Cell<usize>,
}

/// An entry in the linked list of the registered mutators.
#[derive(Default, Debug)]
pub struct Registry {
    /// The least significant bit is set if the mutator is currently pinned. The rest of the bits
    /// encode the current epoch.
    state: AtomicUsize,
}

/// A witness that the current mutator is pinned.
///
/// A reference to `Scope` is a witness that the current mutator is pinned. Lots of methods that
/// interact with [`Atomic`]s can safely be called only while the mutator is pinned so they often
/// require a reference to `Scope`.
///
/// This data type is inherently bound to the thread that created it, therefore it does not
/// implement `Send` nor `Sync`.
///
/// [`Atomic`]: struct.Atomic.html
#[derive(Debug)]
pub struct Scope<N: Realm> {
    realm: N,
    bag: *mut Bag, // !Send + !Sync
}


impl<'scope, N> Mutator<'scope, N>
where
    N: Realm + 'scope,
{
    pub fn new(n: N) -> Self {
        Mutator {
            realm: n,
            registry: unsafe {
                // Since we dereference no pointers in this block and create no garbages, it is safe
                // to use `unprotected_with_bag` with an invalid bag.
                let mut bag = ::std::mem::zeroed::<Bag>();
                n.unprotected_with_bag(&mut bag, |scope| {
                    &*n.registries().insert_head(Registry::new(), scope).as_raw()
                })
            },
            bag: UnsafeCell::new(Bag::new()),
            is_pinned: Cell::new(false),
            pin_count: Cell::new(0),
        }
    }

    /// Pins the current mutator, executes a function, and unpins the mutator.
    ///
    /// The provided function takes a reference to a `Scope`, which can be used to interact with
    /// [`Atomic`]s. The scope serves as a proof that whatever data you load from an [`Atomic`] will
    /// not be concurrently deleted by another mutator while the scope is alive.
    ///
    /// Note that keeping a mutator pinned for a long time prevents memory reclamation of any newly
    /// deleted objects protected by [`Atomic`]s. The provided function should be very quick -
    /// generally speaking, it shouldn't take more than 100 ms.
    ///
    /// Pinning is reentrant. There is no harm in pinning a mutator while it's already pinned
    /// (repinning is essentially a noop).
    ///
    /// Pinning itself comes with a price: it begins with a `SeqCst` fence and performs a few other
    /// atomic operations. However, this mechanism is designed to be as performant as possible, so
    /// it can be used pretty liberally. On a modern machine pinning takes 10 to 15 nanoseconds.
    ///
    /// [`Atomic`]: struct.Atomic.html
    pub fn pin<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Scope<N>) -> R,
    {
        let registry = self.registry.get();
        let scope = &Scope {
            realm: self.realm,
            bag: self.bag.get(),
        };

        let was_pinned = self.is_pinned.get();
        if !was_pinned {
            // Increment the pin counter.
            let count = self.pin_count.get();
            self.pin_count.set(count.wrapping_add(1));

            // Pin the mutator.
            self.is_pinned.set(true);
            registry.set_pinned();

            // If the counter progressed enough, try advancing the epoch and collecting garbage.
            if count % PINS_BETWEEN_COLLECT == 0 {
                self.realm.collect(scope);
            }
        }

        // This will unpin the mutator even if `f` panics.
        defer! {
            if !was_pinned {
                // Unpin the mutator.
                registry.set_unpinned();
                self.is_pinned.set(false);
            }
        }

        f(scope)
    }

    /// Returns `true` if the current mutator is pinned.
    pub fn is_pinned(&'scope self) -> bool {
        self.is_pinned.get()
    }
}

impl<'scope, N: Realm> Drop for Mutator<'scope, N> {
    fn drop(&mut self) {
        // Now that the mutator is exiting, we must move the local bag into the global garbage
        // queue. Also, let's try advancing the epoch and help free some garbage.

        unsafe {
            let bag = &mut *self.bag.get();

            self.pin(|scope| {
                // Spare some cycles on garbage collection.
                self.realm.collect(scope);

                // Unregister the mutator by marking this entry as deleted.
                self.registry.delete(scope);

                // Push the local bag into the global garbage queue.
                self.realm.push_bag(bag, scope);
            });
        }
    }
}

impl Registry {
    // FIXME(stjepang): Registries are stored in a linked list because linked lists are fairly easy
    // to implement in a lock-free manner. However, traversal is rather slow due to cache misses and
    // data dependencies. We should experiment with other data structures as well.

    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns if the mutator is pinned, and if so, the epoch at which it is pinned.
    #[inline]
    pub fn get_state(&self) -> (bool, usize) {
        let state = self.state.load(Relaxed);
        ((state & 1) == 1, state & !1)
    }

    /// Marks the mutator as pinned.
    ///
    /// Must not be called if the mutator is already pinned!
    #[inline]
    pub fn set_pinned(&self) {
        let epoch = global::epoch().load(Relaxed);
        let state = epoch | 1;

        // Now we must store `state` into `self.state`. It's important that any succeeding loads
        // don't get reordered with this store. In order words, this mutator's epoch must be fully
        // announced to other mutators. Only then it becomes safe to load from the shared memory.
        if cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
            // On x86 architectures we have a choice:
            // 1. `atomic::fence(SeqCst)`, which compiles to a `mfence` instruction.
            // 2. `compare_and_swap(_, _, SeqCst)`, which compiles to a `lock cmpxchg` instruction.
            //
            // Both instructions have the effect of a full barrier, but the second one seems to be
            // faster in this particular case.
            let result = self.state.compare_and_swap(0, state, SeqCst);
            debug_assert_eq!(0, result, "Registry::set_pinned()'s CAS should succeed.");
        } else {
            self.state.store(state, Relaxed);
            ::std::sync::atomic::fence(SeqCst);
        }
    }

    /// Marks the mutator as unpinned.
    #[inline]
    pub fn set_unpinned(&self) {
        // Clear the last bit.
        // We don't need to preserve the epoch, so just store the number zero.
        self.state.store(0, Release);
    }
}

impl<N> Scope<N>
where
    N: Realm,
{
    unsafe fn get_bag(&self) -> &mut Bag {
        &mut *self.bag
    }

    unsafe fn defer_garbage(&self, mut garbage: Garbage) {
        let bag = self.get_bag();

        while let Err(g) = bag.try_push(garbage) {
            self.realm.push_bag(bag, self);
            garbage = g;
        }
    }

    /// Deferred deallocation of heap-allocated object `ptr`.
    ///
    /// The specified object is an array allocated at address `object` and consists of `count`
    /// elements of type `T`.
    ///
    /// This function inserts the object into a mutator-local [`Bag`]. When the bag becomes full,
    /// the bag is flushed into the globally shared queue of bags.
    ///
    /// If the object is unusually large, it is wise to follow up with a call to [`flush`] so that
    /// it doesn't get stuck waiting in the local bag for a long time.
    ///
    /// [`Bag`]: struct.Bag.html
    /// [`flush`]: fn.flush.html
    pub unsafe fn defer_free<T>(&self, ptr: Ptr<T>) {
        self.defer_garbage(Garbage::new_free(ptr.as_raw() as *mut T, 1))
    }

    /// Deferred destruction and deallocation of heap-allocated object `ptr`.
    ///
    /// The specified object is an array allocated at address `object` and consists of `count`
    /// elements of type `T`.
    pub unsafe fn defer_drop<T: Send + 'static>(&self, ptr: Ptr<T>) {
        self.defer_garbage(Garbage::new_drop(ptr.as_raw() as *mut T, 1))
    }

    /// Deferred execution of an arbitrary function `f`.
    pub unsafe fn defer<F: FnOnce() + Send + 'static>(&self, f: F) {
        self.defer_garbage(Garbage::new(f))
    }

    /// Flushes all garbage in the thread-local storage into the global garbage queue, attempts to
    /// advance the epoch, and collects some garbage.
    ///
    /// Even though flushing can be explicitly called, it is also automatically triggered when the
    /// thread-local storage fills up or when we pin the current thread a specific number of times.
    ///
    /// It is wise to flush the bag just after passing a very large object to [`defer_free`] or
    /// [`defer_drop`], so that it isn't sitting in the local bag for a long time.
    ///
    /// [`defer_free`]: fn.defer_free.html
    /// [`defer_drop`]: fn.defer_drop.html
    pub fn flush(&self) {
        unsafe {
            let bag = self.get_bag();
            if !bag.is_empty() {
                self.realm.push_bag(bag, self);
            }
        }

        self.realm.collect(self);
    }
}


#[cfg(test)]
mod tests {}
