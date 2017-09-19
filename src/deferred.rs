use std::mem;
use std::ptr;

/// Provides methods to dispatch a call to a `FnOnce()` from a trait object.
pub trait Callback {
    /// Calls the function from a trait object on the stack.
    ///
    /// This will copy `self`, call the function, and finally drop the copy.
    /// This method may be called only once, and `self` must not be dropped after that (tip: pass
    /// it to `std::mem::forget`).
    unsafe fn copy_and_call(&self);

    /// Calls the function from a trait object on the heap.
    fn call_box(self: Box<Self>);
}

impl<F: FnOnce()> Callback for F {
    #[inline]
    unsafe fn copy_and_call(&self) {
        let f: Self = ptr::read(self);
        f();
    }

    #[inline]
    fn call_box(self: Box<Self>) {
        let f: Self = *self;
        f();
    }
}

/// The representation of a trait object like `&SomeTrait`.
///
/// This struct has the same layout as types like `&SomeTrait` and `Box<AnotherTrait>`.
///
/// It is actually already provided as `std::raw::TraitObject` gated under the nightly `raw`
/// feature. But we don't use nightly Rust, so the struct was simply copied over into Crossbeam.
///
/// If the layout of this struct changes in the future, Crossbeam will break, but that is a fairly
/// unlikely scenario.
// FIXME(stjepang): When feature `raw` gets stabilized, use `std::raw::TraitObject` instead.
#[repr(C)]
#[derive(Copy, Clone)]
struct TraitObject {
    data: *mut (),
    vtable: *mut (),
}

/// Some space to keep a `FnOnce()` object on the stack.
type Data = [usize; 3];

/// A `FnOnce()` that is stored inline if small, or otherwise boxed on the heap.
///
/// This is a handy way of keeping an unsized `FnOnce()` within a sized structure.
pub struct Deferred {
    vtable: *mut (),
    data: Data,
}

impl Deferred {
    /// Constructs a new `Deferred` from a `FnOnce()`.
    pub fn new<F: FnOnce()>(f: F) -> Self {
        let size = mem::size_of::<F>();
        let align = mem::align_of::<F>();

        unsafe {
            if size <= mem::size_of::<Data>() && align <= mem::align_of::<Data>() {
                let vtable = {
                    let callback: &Callback = &f;
                    let obj: TraitObject = mem::transmute(callback);
                    obj.vtable
                };

                let mut data = Data::default();
                ptr::write(&mut data as *mut Data as *mut F, f);

                Deferred { vtable, data }
            } else {
                let mut data = Data::default();
                let b: Box<Callback> = Box::new(f);
                ptr::write(&mut data as *mut Data as *mut Box<Callback>, b);

                Deferred {
                    vtable: 1 as *mut (),
                    data,
                }
            }
        }
    }

    /// Calls the function or panics if it was already called.
    #[inline]
    pub fn call(&mut self) {
        let vtable = mem::replace(&mut self.vtable, ptr::null_mut());
        assert!(!vtable.is_null(), "cannot call `FnOnce` more than once");

        unsafe {
            if vtable as usize != 0x1 {
                let data = &mut self.data as *mut _ as *mut ();
                let obj = TraitObject { data, vtable };
                let callback: &Callback = mem::transmute(obj);
                callback.copy_and_call();
            } else {
                let b: Box<Callback> = ptr::read(&self.data as *const Data as *const Box<Callback>);
                b.call_box();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Deferred;

    #[test]
    fn smoke_on_stack() {
        let a = [0usize; 1];
        let mut d = Deferred::new(move || drop(a));
        d.call();
    }

    #[test]
    fn smoke_on_heap() {
        let a = [0usize; 10];
        let mut d = Deferred::new(move || drop(a));
        d.call();
    }

    #[test]
    #[should_panic(expected = "cannot call `FnOnce` more than once")]
    fn twice_on_stack() {
        let a = [0usize; 1];
        let mut d = Deferred::new(move || drop(a));
        d.call();
        d.call();
    }

    #[test]
    #[should_panic(expected = "cannot call `FnOnce` more than once")]
    fn twice_on_heap() {
        let a = [0usize; 10];
        let mut d = Deferred::new(move || drop(a));
        d.call();
        d.call();
    }

    #[test]
    fn string() {
        let a = "hello".to_string();
        let mut d = Deferred::new(move || assert_eq!(a, "hello"));
        d.call();
    }

    #[test]
    fn boxed_slice_i32() {
        let a: Box<[i32]> = vec![2, 3, 5, 7].into_boxed_slice();
        let mut d = Deferred::new(move || assert_eq!(*a, [2, 3, 5, 7]));
        d.call();
    }

    #[test]
    fn long_slice_usize() {
        let a: [usize; 5] = [2, 3, 5, 7, 11];
        let mut d = Deferred::new(move || assert_eq!(a, [2, 3, 5, 7, 11]));
        d.call();
    }
}
