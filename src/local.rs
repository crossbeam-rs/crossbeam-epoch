use registry::Registry;
use epoch::Epoch;
use garbage::Bag;
use scope::Namespace;
use sync::list::List;
use sync::queue::Queue;


pub struct LocalNamespace {
    epoch: Epoch,
    garbages: Queue<(usize, Bag)>,
    registries: List<Registry>,
}

impl LocalNamespace {
    pub fn new() -> Self {
        LocalNamespace { epoch: Epoch::new(), garbages: Queue::new(), registries: List::new() }
    }
}

impl Default for LocalNamespace {
    fn default() -> Self {
        Self::new()
    }
}

impl<'scope> Namespace for &'scope LocalNamespace {
    fn epoch(&self) -> &Epoch {
        &self.epoch
    }

    fn garbages(&self) -> &Queue<(usize, Bag)> {
        &self.garbages
    }

    fn registries(&self) -> &List<Registry> {
        &self.registries
    }
}
