//! Handle: reference to a garbage collector
//!
//! # Pinning
//!
//! Every handle contains an integer that tells whether the handle is pinned and if so, what was the
//! global epoch at the time it was pinned. Handles also hold a pin counter that aids in periodic
//! global epoch advancement.
//!
//! When a handle is pinned, a `Scope` is returned as a witness that the handle is pinned.  Scopes
//! are necessary for performing atomic operations, and for freeing/dropping locations.

use std::cell::{Cell, UnsafeCell};
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::{Relaxed, Release, SeqCst};

use sync::list::Node;
use garbage::{Garbage, Bag};
use collector::Global;


// FIXME(stjepang): Registries are stored in a linked list because linked lists are fairly easy to
// implement in a lock-free manner. However, traversal is rather slow due to cache misses and data
// dependencies. We should experiment with other data structures as well.
/// Reference to a garbage collector
#[derive(Debug)]
pub struct Handle {
    /// A reference to the global data.
    global: Arc<Global>,
    /// The local garbage objects that will be later freed.
    bag: UnsafeCell<Bag>,
    /// This handle's entry in the local epoch list.
    ///
    /// It points to a node in `global`, so it is live as far as the handle has a counted reference
    /// to `global`.
    local_epoch: *const Node<LocalEpoch>,
    /// Whether the handle is currently pinned.
    is_pinned: Cell<bool>,
    /// Total number of pinnings performed.
    pin_count: Cell<usize>,
}

/// An entry in the linked list of the registered handles.
#[derive(Default, Debug)]
pub struct LocalEpoch {
    /// The least significant bit is set if the handle is currently pinned. The rest of the bits
    /// encode the current epoch.
    state: AtomicUsize,
}

/// A witness that the current handle is pinned.
///
/// A `Scope` is a witness that the current handle is pinned. Lots of methods that interact with
/// [`Atomic`]s can safely be called only while the handle is pinned so they often require a
/// `Scope`.
///
/// This data type is inherently bound to the thread that created it, therefore it does not
/// implement `Send` nor `Sync`.
///
/// [`Atomic`]: struct.Atomic.html
#[derive(Clone, Copy, Debug)]
pub struct Scope<'scope> {
    /// A reference to the handle.
    handle: Option<&'scope Handle>,
    /// !Send + !Sync
    _marker: PhantomData<*mut u8>,
}


impl Handle {
    /// Number of pinnings after which a handle will collect some global garbage.
    const PINS_BETWEEN_COLLECT: usize = 128;

    pub(crate) fn new(global: &Arc<Global>) -> Self {
        let global = global.clone();
        let local_epoch = global.register();

        Handle {
            global,
            bag: UnsafeCell::new(Bag::new()),
            local_epoch,
            is_pinned: Cell::new(false),
            pin_count: Cell::new(0),
        }
    }

    /// Pins the current handle, executes a function, and unpins the handle.
    ///
    /// The provided function takes a `Scope`, which can be used to interact with [`Atomic`]s. The
    /// scope serves as a proof that whatever data you load from an [`Atomic`] will not be
    /// concurrently deleted by another handle while the scope is alive.
    ///
    /// Note that keeping a handle pinned for a long time prevents memory reclamation of any newly
    /// deleted objects protected by [`Atomic`]s. The provided function should be very quick -
    /// generally speaking, it shouldn't take more than 100 ms.
    ///
    /// Pinning is reentrant. There is no harm in pinning a handle while it's already pinned
    /// (repinning is essentially a noop).
    ///
    /// Pinning itself comes with a price: it begins with a `SeqCst` fence and performs a few other
    /// atomic operations. However, this mechanism is designed to be as performant as possible, so
    /// it can be used pretty liberally. On a modern machine pinning takes 10 to 15 nanoseconds.
    ///
    /// [`Atomic`]: struct.Atomic.html
    pub fn pin<F, R>(&self, f: F) -> R
    where
        F: for<'scope> FnOnce(Scope<'scope>) -> R,
    {
        let local_epoch = unsafe { (*self.local_epoch).get() };
        let scope = Scope {
            handle: Some(self),
            _marker: PhantomData,
        };

        let was_pinned = self.is_pinned.get();
        if !was_pinned {
            // Increment the pin counter.
            let count = self.pin_count.get();
            self.pin_count.set(count.wrapping_add(1));

            // Pin the handle.
            self.is_pinned.set(true);
            let epoch = self.global.get_epoch();
            local_epoch.set_pinned(epoch);

            // If the counter progressed enough, try advancing the epoch and collecting garbage.
            if count % Self::PINS_BETWEEN_COLLECT == 0 {
                self.global.collect(scope);
            }
        }

        // This will unpin the handle even if `f` panics.
        defer! {
            if !was_pinned {
                // Unpin the handle.
                local_epoch.set_unpinned();
                self.is_pinned.set(false);
            }
        }

        f(scope)
    }

    /// Returns `true` if the current handle is pinned.
    pub fn is_pinned(&self) -> bool {
        self.is_pinned.get()
    }
}

impl Clone for Handle {
    fn clone(&self) -> Self {
        Self::new(&self.global)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // Now that the handle is exiting, we must move the local bag into the global garbage
        // queue. Also, let's try advancing the epoch and help free some garbage.

        self.pin(|scope| {
            // Spare some cycles on garbage collection.
            self.global.collect(scope);

            // Unregister the handle by marking this entry as deleted.
            unsafe {
                (*self.local_epoch).delete(scope);
            }

            // Push the local bag into the global garbage queue.
            unsafe {
                self.global.push_bag(&mut *self.bag.get(), scope);
            }
        });
    }
}

/// Returns a [`Scope`] without pinning any handle.
///
/// Sometimes, we'd like to have longer-lived scopes in which we know our thread can access atomics
/// without protection. This is true e.g. when deallocating a big data structure, or when
/// constructing it from a long iterator. In such cases we don't need to be overprotective because
/// there is no fear of other threads concurrently destroying objects.
///
/// Function `unprotected` is *unsafe* because we must promise that (1) the locations that we access
/// should not be deallocated by concurrent handles, and (2) the locations that we deallocate
/// should not be accessed by concurrent handles.
///
/// Just like with the safe epoch::pin function, unprotected use of atomics is enclosed within a
/// scope so that pointers created within it don't leak out or get mixed with pointers from other
/// scopes.
#[inline]
pub unsafe fn unprotected<F, R>(f: F) -> R
where
    F: for<'scope> FnOnce(Scope<'scope>) -> R,
{
    let scope = Scope {
        handle: None,
        _marker: PhantomData,
    };
    f(scope)
}

impl LocalEpoch {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns if the handle is pinned, and if so, the epoch at which it is pinned.
    #[inline]
    pub fn get_state(&self) -> (bool, usize) {
        let state = self.state.load(Relaxed);
        ((state & 1) == 1, state & !1)
    }

    /// Marks the handle as pinned.
    ///
    /// Must not be called if the handle is already pinned!
    #[inline]
    pub fn set_pinned(&self, epoch: usize) {
        let state = epoch | 1;

        // Now we must store `state` into `self.state`. It's important that any succeeding loads
        // don't get reordered with this store. In order words, this handle's epoch must be fully
        // announced to other handles. Only then it becomes safe to load from the shared memory.
        if cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
            // On x86 architectures we have a choice:
            // 1. `atomic::fence(SeqCst)`, which compiles to a `mfence` instruction.
            // 2. `compare_and_swap(_, _, SeqCst)`, which compiles to a `lock cmpxchg` instruction.
            //
            // Both instructions have the effect of a full barrier, but the second one seems to be
            // faster in this particular case.
            let result = self.state.compare_and_swap(0, state, SeqCst);
            debug_assert_eq!(0, result, "LocalEpoch::set_pinned()'s CAS should succeed.");
        } else {
            self.state.store(state, Relaxed);
            ::std::sync::atomic::fence(SeqCst);
        }
    }

    /// Marks the handle as unpinned.
    #[inline]
    pub fn set_unpinned(&self) {
        // Clear the last bit.
        // We don't need to preserve the epoch, so just store the number zero.
        self.state.store(0, Release);
    }
}

impl<'scope> Scope<'scope> {
    unsafe fn defer_garbage(self, mut garbage: Garbage) {
        self.handle.map(|handle| {
            let bag = &mut *handle.bag.get();
            while let Err(g) = bag.try_push(garbage) {
                handle.global.push_bag(bag, self);
                garbage = g;
            }
        });
    }

    /// Deferred execution of an arbitrary function `f`.
    ///
    /// This function inserts the function into a mutator-local [`Bag`]. When the bag becomes full,
    /// the bag is flushed into the globally shared queue of bags.
    ///
    /// If this function is destroying a particularly large object, it is wise to follow up with a
    /// call to [`flush`] so that it doesn't get stuck waiting in the local bag for a long time.
    ///
    /// [`Bag`]: struct.Bag.html
    /// [`flush`]: fn.flush.html
    pub unsafe fn defer<R, F: FnOnce() -> R + Send>(self, f: F) {
        self.defer_garbage(Garbage::new(|| drop(f())))
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
    /// [`defer_free`]: fn.defer_free.html [`defer_drop`]: fn.defer_drop.html
    pub fn flush(self) {
        unsafe {
            self.handle.map(|handle| {
                let bag = &mut *handle.bag.get();

                if !bag.is_empty() {
                    handle.global.push_bag(bag, self);
                }

                handle.global.collect(self);
            });
        }
    }
}


#[cfg(test)]
mod tests {
    use Collector;

    #[test]
    fn pin_reentrant() {
        let collector = Collector::new();
        let handle = collector.handle();
        drop(collector);

        assert!(!handle.is_pinned());
        handle.pin(|_| {
            handle.pin(|_| {
                assert!(handle.is_pinned());
            });
            assert!(handle.is_pinned());
        });
        assert!(!handle.is_pinned());
    }
}
