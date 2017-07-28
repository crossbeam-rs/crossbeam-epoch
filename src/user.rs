use mutator::{Realm, Registry};
use epoch::Epoch;
use garbage::Bag;
use sync::list::List;
use sync::ms_queue::MsQueue;


pub struct UserRealm<'scope> {
    registries: List<Registry>,
    garbages: MsQueue<&'scope UserRealm<'scope>, (usize, Bag)>,
    epoch: Epoch,
}

impl<'scope> Realm for &'scope UserRealm<'scope> {
    fn registries(&self) -> &List<Registry> {
        &self.registries
    }

    fn garbages(&self) -> &MsQueue<&'scope UserRealm<'scope>, (usize, Bag)> {
        &self.garbages
    }

    fn epoch(&self) -> &Epoch {
        &self.epoch
    }
}

pub fn with_realm<F, R>(f: F) -> R
where
    for<'scope> F: FnOnce(&'scope UserRealm<'scope>) -> R,
{
    unsafe {
        let mut realm = UserRealm {
            registries: List::new(),
            garbages: ::std::mem::zeroed(),
            epoch: Epoch::new(),
        };
        let garbages = ::std::mem::transmute(MsQueue::<_, (usize, Bag)>::new(&realm));
        let _ = ::std::mem::replace(&mut realm.garbages, garbages);
        f(&realm)
    }
}
