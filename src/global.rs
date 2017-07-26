use registry::Registry;
use epoch::Epoch;
use garbage::Bag;
use scope::{self, Namespace};
use sync::list::List;
use sync::ms_queue::MsQueue;


type Agent = scope::Agent<'static, GlobalNamespace>;
type Scope = scope::Scope<GlobalNamespace>;


/// registries() returns a reference to the head pointer of the list of thread registries.
lazy_static_null!(pub, registries, List<Registry>);

/// garbages() returns a reference to the global garbage queue, which is lazily initialized.
lazy_static!(pub, garbages,
             MsQueue<GlobalNamespace, (usize, Bag)>,
             MsQueue::new(GlobalNamespace::new()));

/// epoch() returns a reference to the global epoch.
lazy_static_null!(pub, epoch, Epoch);


#[derive(Clone, Copy, Default, Debug)]
pub struct GlobalNamespace {}

impl GlobalNamespace {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Namespace for GlobalNamespace {
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
    GlobalNamespace::new().unprotected(f)
}


thread_local! {
    /// The thread registration agent.
    ///
    /// The agent is lazily initialized on its first use, thus registrating the current thread.
    /// If initialized, the agent will get destructed on thread exit, which in turn unregisters
    /// the thread.
    static AGENT: Agent = {
        registries();
        garbages::get();
        epoch();
        Agent::new(GlobalNamespace::new())
    }
}

pub fn pin<F, R>(f: F) -> R
where
    F: FnOnce(&Scope) -> R,
{
    AGENT.with(|agent| agent.pin(f))
}

pub fn is_pinned() -> bool {
    AGENT.with(|agent| agent.is_pinned())
}
