use alloc::vec::Vec;
use core::marker::PhantomData;
use core::mem;
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
use core::ptr;

use {AtomicTmpl, OwnedTmpl, SharedTmpl, Storage};

/// TODO
pub type AtomicArray<T> = AtomicTmpl<Array<T>, ArrayBox<T>>;

/// TODO
pub type OwnedArray<T> = OwnedTmpl<Array<T>, ArrayBox<T>>;

/// TODO
pub type SharedArray<'g, T> = SharedTmpl<'g, Array<T>, ArrayBox<T>>;

/// TODO
#[derive(Debug)]
pub struct Array<T> {
    anchor: ManuallyDrop<T>,
}

impl<T> Array<T> {
    /// TODO
    pub fn size(&self) -> usize {
        let usize_align = core::mem::align_of::<usize>();
        let usize_size = core::mem::size_of::<usize>();
        let t_align = core::mem::align_of::<T>();
        let t_size = core::mem::size_of::<T>();

        unsafe {
            if t_align <= usize_align {
                let ptr_num = (&self.anchor as *const _ as *const usize).sub(1);
                ptr::read(ptr_num)
            } else {
                let usize_elts = div_ceil(usize_size, t_size);
                let ptr_num =
                    (&self.anchor as *const ManuallyDrop<T>).sub(usize_elts) as *const usize;
                ptr::read(ptr_num)
            }
        }
    }

    /// TODO: index should be < self.size()
    pub unsafe fn at(&self, index: usize) -> *const ManuallyDrop<T> {
        debug_assert!(
            index < self.size(),
            "Array::at(): index {} should be < size {}",
            index,
            self.size()
        );

        let anchor = &self.anchor as *const ManuallyDrop<T>;
        anchor.add(index)
    }
}

/// TODO
///
/// # Examples
///
/// ```
/// use crossbeam_epoch::{self as epoch, OwnedArray, Array, ArrayBox};
///
/// let a = ArrayBox::<i32>::new(10);
/// let o = OwnedArray::from(a);
/// ```
#[derive(Debug)]
pub struct ArrayBox<T> {
    ptr: *mut Array<T>,
    _marker: PhantomData<T>,
}

fn div_ceil(a: usize, b: usize) -> usize {
    (a + b - 1) / b
}

impl<T> ArrayBox<T> {
    /// TODO
    pub fn new(num: usize) -> Self {
        let usize_align = core::mem::align_of::<usize>();
        let usize_size = core::mem::size_of::<usize>();
        let t_align = core::mem::align_of::<T>();
        let t_size = core::mem::size_of::<T>();

        if t_align <= usize_align {
            let t_bytes = num * t_size;
            let t_words = div_ceil(t_bytes, usize_size);

            let mut vec = Vec::<usize>::with_capacity(1 + t_words);
            let ptr = vec.as_mut_ptr();
            mem::forget(vec);

            unsafe {
                ptr::write(ptr, num);
            }

            Self {
                ptr: unsafe { (ptr.add(1)) } as *mut Array<T>,
                _marker: PhantomData,
            }
        } else {
            let usize_elts = div_ceil(usize_size, t_size);

            let mut vec = Vec::<T>::with_capacity(usize_elts + num);
            let ptr = vec.as_mut_ptr();
            mem::forget(vec);

            unsafe {
                ptr::write(ptr as *mut usize, num);
            }

            Self {
                ptr: unsafe { (ptr.add(usize_elts)) } as *mut Array<T>,
                _marker: PhantomData,
            }
        }
    }
}

impl<T> Drop for ArrayBox<T> {
    fn drop(&mut self) {
        let usize_align = core::mem::align_of::<usize>();
        let usize_size = core::mem::size_of::<usize>();
        let t_align = core::mem::align_of::<T>();
        let t_size = core::mem::size_of::<T>();

        unsafe {
            if t_align <= usize_align {
                let ptr_num = (self.ptr as *mut usize).sub(1);
                let num = ptr::read(ptr_num);

                let t_bytes = num * t_size;
                let t_words = div_ceil(t_bytes, usize_size);

                drop(Vec::from_raw_parts(ptr_num, 0, 1 + t_words));
            } else {
                let usize_elts = div_ceil(usize_size, t_size);
                let ptr_num = self.ptr.sub(usize_elts) as *mut usize;
                let num = ptr::read(ptr_num);

                drop(Vec::from_raw_parts(ptr_num as *mut T, 0, usize_elts + num));
            }
        }
    }
}

unsafe impl<T> Storage<Array<T>> for ArrayBox<T> {
    fn into_raw(self) -> *mut Array<T> {
        let ptr = self.ptr;
        mem::forget(self);
        ptr
    }

    unsafe fn from_raw(ptr: *mut Array<T>) -> Self {
        Self {
            ptr,
            _marker: PhantomData,
        }
    }
}

impl<T> Deref for ArrayBox<T> {
    type Target = Array<T>;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.ptr }
    }
}

impl<T> DerefMut for ArrayBox<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.ptr }
    }
}
