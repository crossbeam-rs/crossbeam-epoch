use registry::Registry;
use epoch::Epoch;
use garbage::Bag;
use scope::{Mutator, Scope};
use sync::list::List;


/// registries() returns a reference to the head pointer of the list of thread registries.
lazy_static_null!(pub, registries, List<Registry>);

/// epoch() returns a reference to the global epoch.
lazy_static_null!(pub, epoch, Epoch);


pub fn push_bag<'scope>(bag: &mut Bag, scope: &'scope Scope) {
    unimplemented!()
}

/// Collect several bags from the global old garbage queue and destroys their objects.
/// Note: This may itself produce garbage and in turn allocate new bags.
pub fn collect(scope: &Scope) {
    unimplemented!()
}


thread_local! {
    /// The thread registration mutator.
    ///
    /// The mutator is lazily initialized on its first use, thus registrating the current thread.
    /// If initialized, the mutator will get destructed on thread exit, which in turn unregisters
    /// the thread.
    static MUTATOR: Mutator<'static> = {
        registries();
        epoch();
        Mutator::new()
    }
}

pub fn pin<F, R>(f: F) -> R
where
    F: FnOnce(&Scope) -> R,
{
    MUTATOR.with(|mutator| mutator.pin(f))
}

pub fn is_pinned() -> bool {
    MUTATOR.with(|mutator| mutator.is_pinned())
}
