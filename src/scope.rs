use std::cell::{Cell, UnsafeCell};

use atomic::Ptr;
use registry::Registry;
use global;
use garbage::Bag;

use sync::list::Node;


/// Number of pinnings after which a thread will collect some global garbage.
const PINS_BETWEEN_COLLECT: usize = 128;


pub struct Mutator<'scope> {
    /// This mutator's entry in the registry list.
    registry: &'scope Node<Registry>,
    /// The local garbage objects that will be later freed.
    bag: UnsafeCell<Bag>,
    /// Whether the thread is currently pinned.
    is_pinned: Cell<bool>,
    /// Total number of pinnings performed.
    pin_count: Cell<usize>,
}

impl<'scope> Mutator<'scope> {
    pub fn new() -> Self {
        Mutator {
            registry: unsafe {
                // Since we don't dereference any pointers in this block, it's okay to use
                // `unprotected`.  Also, we use an invalid bag since no garbages are created in list
                // insertion.
                let mut bag = ::std::mem::zeroed::<Bag>();
                unprotected_with_bag(&mut bag, |scope| {
                    &*global::registries().insert_head(Registry::new(), scope).as_raw()
                })
            },
            bag: UnsafeCell::new(Bag::new()),
            is_pinned: Cell::new(false),
            pin_count: Cell::new(0),
        }
    }

    /// Pins the current thread.
    ///
    /// The provided function takes a reference to a `Scope`, which can be used to interact with
    /// [`Atomic`]s. The scope serves as a proof that whatever data you load from an [`Atomic`] will
    /// not be concurrently deleted by another thread while the scope is alive.
    ///
    /// Note that keeping a thread pinned for a long time prevents memory reclamation of any newly
    /// deleted objects protected by [`Atomic`]s. The provided function should be very quick -
    /// generally speaking, it shouldn't take more than 100 ms.
    ///
    /// Pinning is reentrant. There is no harm in pinning a thread while it's already pinned
    /// (repinning is essentially a noop).
    ///
    /// Pinning itself comes with a price: it begins with a `SeqCst` fence and performs a few other
    /// atomic operations. However, this mechanism is designed to be as performant as possible, so
    /// it can be used pretty liberally. On a modern machine pinning takes 10 to 15 nanoseconds.
    ///
    /// [`Atomic`]: struct.Atomic.html
    pub fn pin<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Scope) -> R,
    {
        let registry = self.registry.get();
        let scope = &Scope {
            bag: self.bag.get(),
        };

        let was_pinned = self.is_pinned.get();
        if !was_pinned {
            // Increment the pin counter.
            let count = self.pin_count.get();
            self.pin_count.set(count.wrapping_add(1));

            // Pin the thread.
            self.is_pinned.set(true);
            registry.set_pinned();

            // If the counter progressed enough, try advancing the epoch and collecting garbage.
            if count % PINS_BETWEEN_COLLECT == 0 {
                global::collect(scope);
            }
        }

        // This will unpin the thread even if `f` panics.
        defer! {
            if !was_pinned {
                // Unpin the thread.
                registry.set_unpinned();
                self.is_pinned.set(false);
            }
        }

        f(scope)
    }

    pub fn is_pinned(&'scope self) -> bool {
        self.is_pinned.get()
    }
}

impl<'scope> Drop for Mutator<'scope> {
    fn drop(&mut self) {
        unsafe {
            let bag = &mut *self.bag.get();

            self.pin(|scope| {
                global::collect(scope);

                // Unregister the thread by marking this entry as deleted.
                self.registry.delete(scope);

                // Push the local bag into the global garbage queue.
                global::push_bag(bag, scope);
            });
        }
    }
}


#[derive(Debug)]
pub struct Scope {
    bag: *mut Bag, // !Send + !Sync
}

impl Scope
{
    // Deferred deallocation of heap-allocated object `ptr`.
    pub unsafe fn defer_free<T>(&self, ptr: Ptr<T>) {
        unimplemented!()
    }

    // Deferred destruction and deallocation of heap-allocated object `ptr`.
    pub unsafe fn defer_drop<T: Send + 'static>(&self, ptr: Ptr<T>) {
        unimplemented!()
    }

    // Deferred execution of arbitrary function `f`.
    pub unsafe fn defer<F: FnOnce() + Send + 'static>(&self, f: F) {
        unimplemented!()
    }

    pub fn flush(&self) {
        unimplemented!()
    }
}


pub unsafe fn unprotected_with_bag<F, R>(bag: &mut Bag, f: F) -> R
    where
    F: FnOnce(&Scope) -> R,
{
    let scope = &Scope {
        bag: bag,
    };
    f(scope)
}

pub unsafe fn unprotected<F, R>(f: F) -> R
where
    F: FnOnce(&Scope) -> R,
{
    let mut bag = Bag::new();
    let result = unprotected_with_bag(&mut bag, f);
    drop(bag);
    result
}
