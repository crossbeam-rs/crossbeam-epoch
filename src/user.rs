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

pub fn with_namespace<F, R>(f: F) -> R where
    for<'scope> F: FnOnce(&'scope UserNamespace<'scope>) -> R,
{
    unsafe {
        let mut namespace = UserNamespace {
            registries: List::new(),
            garbages: ::std::mem::zeroed(),
            epoch: Epoch::new(),
        };
        let garbages = ::std::mem::transmute(MsQueue::<_, (usize, Bag)>::new(&namespace));
        let _ = ::std::mem::replace(&mut namespace.garbages, garbages);
        f(&namespace)
    }
}
