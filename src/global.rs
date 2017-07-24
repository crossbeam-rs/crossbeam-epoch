use registry::Registry;
use epoch::Epoch;
use garbage::Bag;
use scope::{self, Namespace};
use sync::list::List;
use sync::queue::Queue;


type Agent = scope::Agent<'static, GlobalNamespace>;
type Scope = scope::Scope<GlobalNamespace>;


/// epoch() returns a reference to the global epoch.
lazy_static_null!(pub, epoch, Epoch);

/// garbages() returns a reference to the global garbage queue, which is lazily initialized.
lazy_static!(pub, garbages, Queue<(usize, Bag)>);

/// registries() returns a reference to the head pointer of the list of thread registries.
lazy_static_null!(pub, registries, List<Registry>);

#[derive(Clone, Copy, Debug)]
pub struct GlobalNamespace {
}

impl GlobalNamespace {
    pub fn new() -> Self {
        GlobalNamespace { }
    }
}

impl Namespace for GlobalNamespace {
    fn epoch(&self) -> &Epoch {
        epoch()
    }

    fn garbages(&self) -> &Queue<(usize, Bag)> {
        unsafe { garbages::get_unsafe() }
    }

    fn registries(&self) -> &List<Registry> {
        registries()
    }
}

thread_local! {
    /// The thread registration agent.
    ///
    /// The agent is lazily initialized on its first use, thus registrating the current thread.
    /// If initialized, the agent will get destructed on thread exit, which in turn unregisters
    /// the thread.
    static AGENT: Agent = {
        epoch();
        garbages::get();
        registries();
        Agent::new(GlobalNamespace::new())
    }
}

pub fn pin<F, R>(f: F) -> R
    where F: FnOnce(&Scope) -> R,
{
    AGENT.with(|agent| { agent.pin(f) })
}

pub fn is_pinned() -> bool {
    AGENT.with(|agent| { agent.is_pinned() })
}


pub unsafe fn unprotected_with_bag<F, R>(bag: &mut Bag, f: F) -> R
    where F: FnOnce(&Scope) -> R,
{
    AGENT.with(|agent| { agent.unprotected_with_bag(bag, f) })
}

pub unsafe fn unprotected<F, R>(f: F) -> R
    where F: FnOnce(&Scope) -> R,
{
    AGENT.with(|agent| { agent.unprotected(f) })
}
