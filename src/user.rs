use registry::Registry;
use epoch::Epoch;
use garbage::Bag;
use scope::Namespace;
use sync::list::List;
use sync::ms_queue::MsQueue;


pub struct UserNamespace<'scope> {
    registries: List<Registry>,
    garbages: MsQueue<&'scope UserNamespace<'scope>, (usize, Bag)>,
    epoch: Epoch,
}

impl<'scope> UserNamespace<'scope> {
    pub fn new() -> Self {
        unimplemented!()
        // UserNamespace {
        //     epoch: Epoch::new(),
        //     garbages: MsQueue::new(&self),
        //     registries: List::new()
        // }
    }
}

impl<'scope> Default for UserNamespace<'scope> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'scope> Namespace for &'scope UserNamespace<'scope> {
    fn registries(&self) -> &List<Registry> {
        &self.registries
    }

    fn garbages(&self) -> &MsQueue<&'scope UserNamespace<'scope>, (usize, Bag)> {
        &self.garbages
    }

    fn epoch(&self) -> &Epoch {
        &self.epoch
    }
}
