use std::sync::atomic::{AtomicUsize, ATOMIC_USIZE_INIT};
use std::sync::atomic::Ordering::{Relaxed, Release, SeqCst};

use garbage::Bag;
use scope::{self, Scope};
use sync::list::{List, ListEntry};

pub struct Registry {
    /// The least significant bit is set if the thread is currently pinned. The rest of the bits
    /// encode the current epoch.
    state: AtomicUsize,
}

impl Registry {
    // FIXME(stjepang): Registrys are stored in a linked list because linked lists are fairly easy
    // to implement in a lock-free manner. However, traversal is rather slow due to cache misses and
    // data dependencies. We should experiment with other data structures as well.

    #[inline]
    pub fn new() -> Self {
        Registry { state: ATOMIC_USIZE_INIT }
    }

    #[inline]
    pub fn get_state(&self) -> (bool, usize) {
        let state = self.state.load(Relaxed);
        ((state & 1) == 1, state & !1)
    }

    /// Marks the thread as pinned.
    ///
    /// Must not be called if the thread is already pinned!
    #[inline]
    pub fn set_pinned(&self, epoch: usize, _scope: &Scope) {
        let state = epoch | 1;

        // Now we must store `state` into `self.state`. It's important that any succeeding loads
        // don't get reordered with this store. In order words, this thread's epoch must be fully
        // announced to other threads. Only then it becomes safe to load from the shared memory.
        if cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
            // On x86 architectures we have a choice:
            // 1. `atomic::fence(SeqCst)`, which compiles to a `mfence` instruction.
            // 2. `compare_and_swap(_, _, SeqCst)`, which compiles to a `lock cmpxchg` instruction.
            //
            // Both instructions have the effect of a full barrier, but the second one seems to be
            // faster in this particular case.
            let previous = self.state.load(Relaxed);
            self.state.compare_and_swap(previous, state, SeqCst);
        } else {
            self.state.store(state, Relaxed);
            ::std::sync::atomic::fence(SeqCst);
        }
    }

    /// Marks the thread as unpinned.
    #[inline]
    pub fn set_unpinned(&self) {
        // Clear the last bit.
        // We don't need to preserve the epoch, so just store the number zero.
        self.state.store(0, Release);
    }
}

impl List<Registry> {
    #[inline]
    pub fn register<'scope>(&self) -> &'scope ListEntry<Registry> {
        // Since we don't dereference any pointers in this block, it's okay to use `unprotected`.
        // Also, we use an invalid bag since no garbages are created in list insertion.
        unsafe {
            let mut bag = ::std::mem::zeroed::<Bag>();
            scope::unprotected_with_bag(&mut bag, |scope| {
                &*self.insert_head(Registry::new(), scope).as_raw()
            })
        }
    }
}
