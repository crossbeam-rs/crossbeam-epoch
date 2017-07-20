use std::mem;
use boxfnonce::SendBoxFnOnce;


/// Maximum number of objects a bag can contain.
#[cfg(not(feature = "strict_gc"))]
const MAX_OBJECTS: usize = 64;
#[cfg(feature = "strict_gc")]
const MAX_OBJECTS: usize = 4;


pub enum Garbage {
    Destroy { object: *mut u8, size: usize, destroy: unsafe fn(*mut u8, usize), },
    Fn { f: Option<SendBoxFnOnce<(), ()>>, },
}

impl Garbage {
    /// Make a garbage object that will later be destroyed using `destroy`.
    ///
    /// The specified object is an array allocated at address `object` and consists of `size`
    /// elements of type `T`.
    ///
    /// Note: The object must be `Send + 'self`.
    pub fn new_destroy<T>(object: *mut T, size: usize, destroy: unsafe fn(*mut T, usize)) -> Self {
        Garbage::Destroy {
            object: object as *mut u8,
            size: size,
            destroy: unsafe { mem::transmute(destroy) },
        }
    }

    /// Make a garbage object that will later be freed.
    ///
    /// The specified object is an array allocated at address `object` and consists of `size`
    /// elements of type `T`.
    pub fn new_free<T>(object: *mut T, size: usize) -> Self {
        unsafe fn free<T>(object: *mut T, size: usize) {
            // Free the memory, but don't run the destructors.
            drop(Vec::from_raw_parts(object, 0, size));
        }
        Self::new_destroy(object, size, free)
    }

    /// Make a garbage object that will later be dropped and freed.
    ///
    /// The specified object is an array allocated at address `object` and consists of `count`
    /// elements of type `T`.
    ///
    /// Note: The object must be `Send + 'self`.
    pub fn new_drop<T>(object: *mut T, size: usize) -> Self {
        unsafe fn destruct<T>(ptr: *mut T, size: usize) {
            // Run the destructors and free the memory.
            drop(Vec::from_raw_parts(ptr, size, size));
        }
        Self::new_destroy(object, size, destruct)
    }

    /// Make a closure that will later be called.
    pub fn new<F: FnOnce() + Send + 'static>(f: F) -> Self {
        Garbage::Fn { f: Some(SendBoxFnOnce::from(f)) }
    }
}

impl Drop for Garbage {
    fn drop(&mut self) {
        match self {
            &mut Garbage::Destroy { destroy, object, size } => {
                unsafe { (destroy)(object, size); }
            },
            &mut Garbage::Fn { ref mut f } => {
                let f = f.take().unwrap();
                f.call();
            },
        }
    }
}


/// Bag of garbages.
pub struct Bag {
    /// Number of objects in the bag.
    len: usize,
    /// Removed objects.
    objects: [Garbage; MAX_OBJECTS],
}

impl Bag {
    /// Returns a new, empty bag.
    pub fn new() -> Self {
        Bag {
            len: 0,
            objects: unsafe { mem::zeroed() },
        }
    }

    /// Returns `true` if the bag is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Attempts to insert a garbage object into the bag and returns `true` if succeeded.
    pub fn try_insert(&mut self, garbage: Garbage) -> Result<(), Garbage> {
        if self.len == self.objects.len() {
            return Err(garbage);
        }

        self.objects[self.len] = garbage;
        self.len += 1;
        return Ok(());
    }
}

impl Drop for Bag {
    fn drop(&mut self) {
        for garbage in self.objects.into_iter().take(self.len) {
            drop(garbage)
        }
    }
}
