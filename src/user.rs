use registry::Registry;
use epoch::Epoch;
use garbage::Bag;
use scope::Namespace;
use sync::list::List;
use sync::ms_queue::MsQueue;


pub struct UserNamespace<'scope> {
    epoch: Epoch,
    garbages: MsQueue<&'scope UserNamespace<'scope>, (usize, Bag)>,
    registries: List<Registry>,
}

impl<'scope> UserNamespace<'scope> {
    pub fn new() -> Self {
        unimplemented!()
        // UserNamespace { epoch: Epoch::new(), garbages: MsQueue::new(&self), registries: List::new() }
    }
}

impl<'scope> Default for UserNamespace<'scope> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'scope> Namespace for &'scope UserNamespace<'scope> {
    fn epoch(&self) -> &Epoch {
        &self.epoch
    }

    fn garbages(&self) -> &MsQueue<&'scope UserNamespace<'scope>, (usize, Bag)> {
        &self.garbages
    }

    fn registries(&self) -> &List<Registry> {
        &self.registries
    }
}

impl<'scope> Drop for UserNamespace<'scope> {
    fn drop(&mut self) {
        drop(&mut self.registries);
        drop(&mut self.garbages);
        drop(&mut self.epoch);
    }
}
