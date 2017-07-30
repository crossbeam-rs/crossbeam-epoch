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

use std::cell::Cell;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::{Relaxed, Release, SeqCst};

use atomic::Ptr;
use sync::list::Node;
use global;


/// Number of pinnings after which a mutator will collect some global garbage.
const PINS_BETWEEN_COLLECT: usize = 128;


/// Entity that changes shared locations.
pub struct Mutator<'scope> {
    /// This mutator's entry in the registry list.
    registry: &'scope Node<Registry>,
    /// Whether the mutator is currently pinned.
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
pub struct Scope {
    _private: *mut (), // !Send + !Sync

    // FIXME(jeehoonkang): it should have a garbage bag.
}


impl<'scope> Mutator<'scope> {
    pub fn new() -> Self {
        Mutator {
            registry: unsafe {
                // Since we dereference no pointers in this block, it is safe to use `unprotected`.
                //
                // FIXME(jeehoonkang): in fact, since we create no garbages, it is safe to use
                // `unprotected_with_bag` with an invalid bag.
                unprotected(|scope| {
                    &*global::registries()
                        .insert_head(Registry::new(), scope)
                        .as_raw()
                })
            },
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
        F: FnOnce(&Scope) -> R,
    {
        let registry = self.registry.get();
        let scope = &Scope { _private: ::std::ptr::null_mut() };

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
                global::collect(scope);
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

impl<'scope> Drop for Mutator<'scope> {
    fn drop(&mut self) {
        // Now that the mutator is exiting, we must move the local bag into the global garbage
        // queue. Also, let's try advancing the epoch and help free some garbage.

        self.pin(|scope| {
            // Spare some cycles on garbage collection.
            global::collect(scope);

            // Unregister the mutator by marking this entry as deleted.
            self.registry.delete(scope);

            // Push the local bag into the global garbage queue.
            unimplemented!();
        });
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

impl Scope {
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
        unimplemented!()
    }

    /// Deferred destruction and deallocation of heap-allocated object `ptr`.
    ///
    /// The specified object is an array allocated at address `object` and consists of `count`
    /// elements of type `T`.
    pub unsafe fn defer_drop<T: Send + 'static>(&self, ptr: Ptr<T>) {
        unimplemented!()
    }

    /// Deferred execution of an arbitrary function `f`.
    pub unsafe fn defer<F: FnOnce() + Send + 'static>(&self, f: F) {
        unimplemented!()
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
        unimplemented!()
    }
}

/// Returns a [`Scope`] without pinning any mutator.
///
/// Sometimes, we'd like to have longer-lived scopes in which we know our thread is the only one
/// accessing atomics. This is true e.g. when destructing a big data structure, or when constructing
/// it from a long iterator. In such cases we don't need to be overprotective because there is no
/// fear of other threads concurrently destroying objects.
///
/// Function `unprotected` is *unsafe* because we must promise that no other thread is accessing the
/// Atomics and objects at the same time. The function is safe to use only if (1) the locations that
/// we access should not be deallocated by concurrent mutators, and (2) the locations that we
/// deallocate should not be accessed by concurrent mutators.
///
/// Just like with the safe epoch::pin function, unprotected use of atomics is enclosed within a
/// scope so that pointers created within it don't leak out or get mixed with pointers from other
/// scopes.
pub unsafe fn unprotected<F, R>(f: F) -> R
where
    F: FnOnce(&Scope) -> R,
{
    let scope = &Scope { _private: ::std::ptr::null_mut() };
    f(scope)
}


#[cfg(test)]
mod tests {}
