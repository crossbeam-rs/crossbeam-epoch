use std::sync::atomic::{AtomicUsize, ATOMIC_USIZE_INIT};

/// Maximum number of objects a bag can contain.
#[cfg(not(feature = "strict_gc"))]
const MAX_OBJECTS: usize = 64;
#[cfg(feature = "strict_gc")]
const MAX_OBJECTS: usize = 4;

/// The global epoch.
pub static EPOCH: AtomicUsize = ATOMIC_USIZE_INIT;

pub struct Bag {
    /// Number of objects in the bag.
    len: AtomicUsize,
    /// Removed objects.
    objects: [UnsafeCell<(unsafe fn(*mut u8, usize), *mut u8, usize)>; MAX_OBJECTS],
    /// The global epoch at the moment when this bag got pushed into the queue.
    epoch: usize,
}

impl Bag {
    /// Returns a new, empty bag.
    pub fn new() -> Self {
        Bag {
            len: AtomicUsize::new(0),
            objects: unsafe { mem::zeroed() },
            epoch: unsafe { mem::uninitialized() },
        }
    }

    /// Returns `true` if the bag is empty.
    pub fn is_empty(&self) -> bool {
        self.len.load(Relaxed) == 0
    }

    /// Attempts to insert a garbage object into the bag and returns `true` if succeeded.
    pub fn try_insert<T>(&self, destroy: unsafe fn(*mut T, usize), object: *mut T, count: usize)
                         -> bool {
        // Erase type `*mut T` and use `*mut u8` instead.
        let destroy: unsafe fn(*mut u8, usize) = unsafe { mem::transmute(destroy) };
        let object = object as *mut u8;

        let mut len = self.len.load(Acquire);
        loop {
            // Is the bag full?
            if len == self.objects.len() {
                return false;
            }

            // Try incrementing `len`.
            match self.len.compare_exchange(len, len + 1, AcqRel, Acquire) {
                Ok(_) => {
                    // Success! Now store the garbage object into the array. The current thread
                    // will synchronize with the thread that destroys it through epoch advancement.
                    unsafe { *self.objects[len].get() = (destroy, object, count) }
                    return true;
                }
                Err(l) => len = l,
            }
        }
    }    
}
