//! The global garbage collection realm
//!
//! This is the default realm for garbage collection.  For each thread, a mutator is lazily
//! initialized on its first use, thus registering the current thread.  If initialized, the
//! thread's mutator will get destructed on thread exit, which in turn unregisters the thread.
//!
//! `registries` is the list is the registered mutators, and `epoch` is the global epoch.

use epoch::Epoch;
use garbage::Bag;
use mutator::{self, Realm, Registry};

use sync::list::List;
use sync::ms_queue::MsQueue;


type Mutator = mutator::Mutator<'static, GlobalRealm>;
type Scope = mutator::Scope<GlobalRealm>;


/// registries() returns a reference to the head pointer of the list of mutator registries.
lazy_static_null!(pub, registries, List<Registry>);

/// garbages() returns a reference to the global garbage queue, which is lazily initialized.
lazy_static!(pub, garbages,
             MsQueue<GlobalRealm, (usize, Bag)>,
             MsQueue::new(GlobalRealm::new()));

/// epoch() returns a reference to the global epoch.
lazy_static_null!(pub, epoch, Epoch);


#[derive(Clone, Copy, Default, Debug)]
pub struct GlobalRealm {}

impl GlobalRealm {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Realm for GlobalRealm {
    fn registries(&self) -> &List<Registry> {
        registries()
    }

    fn garbages(&self) -> &MsQueue<Self, (usize, Bag)> {
        unsafe { garbages::get_unsafe() }
    }

    fn epoch(&self) -> &Epoch {
        epoch()
    }
}

pub unsafe fn unprotected<F, R>(f: F) -> R
where
    F: FnOnce(&Scope) -> R,
{
    GlobalRealm::new().unprotected(f)
}


thread_local! {
    /// The per-thread mutator.
    static MUTATOR: Mutator = {
        // Ensure that the registries and the epoch are properly initialized.
        registries();
        garbages::get();
        epoch();

        Mutator::new(GlobalRealm::new())
    }
}

/// Pin the current thread.
pub fn pin<F, R>(f: F) -> R
where
    F: FnOnce(&Scope) -> R,
{
    MUTATOR.with(|mutator| mutator.pin(f))
}

/// Check if the current thread is pinned.
pub fn is_pinned() -> bool {
    MUTATOR.with(|mutator| mutator.is_pinned())
}


#[cfg(test)]
mod tests {
    use std::thread;
    use std::sync::atomic::Ordering::Relaxed;

    use super::*;

    #[test]
    fn pin_reentrant() {
        assert!(!is_pinned());
        pin(|_| {
            pin(|_| {
                assert!(is_pinned());
            });
            assert!(is_pinned());
        });
        assert!(!is_pinned());
    }

    #[test]
    fn pin_holds_advance() {
        let threads = (0..8)
            .map(|_| {
                thread::spawn(|| for _ in 0..500_000 {
                    pin(|scope| {
                        let before = epoch().load(Relaxed);
                        epoch().try_advance(registries(), scope);
                        let after = epoch().load(Relaxed);

                        assert!(after.wrapping_sub(before) <= 2);
                    });
                })
            })
            .collect::<Vec<_>>();

        for t in threads {
            t.join().unwrap();
        }
    }
}
