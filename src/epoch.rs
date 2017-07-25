use std::ops::Deref;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::{Relaxed, Acquire, Release, SeqCst};

use registry::Registry;
use scope::{Namespace, Scope};
use sync::list::{List, IterResult};


#[derive(Default, Debug)]
pub struct Epoch {
    epoch: AtomicUsize,
}

impl Epoch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attempts to advance the global epoch.
    ///
    /// The global epoch can advance only if all currently pinned threads have been pinned in the
    /// current epoch.
    ///
    /// Returns the current global epoch.
    #[cold]
    pub fn try_advance<'scope, N>(&self, registries: &List<Registry>, scope: &Scope<N>) -> usize where
        N: Namespace + 'scope,
    {
        let epoch = self.epoch.load(Relaxed);
        ::std::sync::atomic::fence(SeqCst);

        // Traverse the linked list of thread registries.
        let mut registries = registries.iter(scope);
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
        self.epoch.store(epoch_new, Release);
        epoch_new
    }
}

impl Deref for Epoch {
    type Target = AtomicUsize;

    fn deref(&self) -> &Self::Target {
        &self.epoch
    }
}
