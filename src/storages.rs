use alloc::boxed::Box;

use atomic::decompose_data;

use {Atomic, Owned, Pointer, Storage};

unsafe impl<T> Storage<T> for Box<T> {
    fn into_raw(self) -> *mut T {
        Self::into_raw(self)
    }

    unsafe fn from_raw(data: *mut T) -> Self {
        Self::from_raw(data)
    }
}

impl<T> Atomic<T, Box<T>> {
    /// Allocates `value` on the heap and returns a new atomic pointer pointing to it.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::Atomic;
    ///
    /// let a = Atomic::new(1234);
    /// ```
    pub fn new(value: T) -> Self {
        Self::from(Owned::new(value))
    }
}

impl<T> From<T> for Atomic<T, Box<T>> {
    fn from(t: T) -> Self {
        Self::new(t)
    }
}

impl<T> Owned<T, Box<T>> {
    /// Allocates `value` on the heap and returns a new owned pointer pointing to it.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::Owned;
    ///
    /// let o = Owned::new(1234);
    /// ```
    pub fn new(value: T) -> Owned<T, Box<T>> {
        Self::from(Box::new(value))
    }

    /// Converts the owned pointer into a `Box`.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Owned};
    ///
    /// let o = Owned::new(1234);
    /// let b: Box<i32> = o.into_box();
    /// assert_eq!(*b, 1234);
    /// ```
    pub fn into_box(self) -> Box<T> {
        let (raw, _) = decompose_data::<T>(self.into_usize());
        unsafe { Box::from_raw(raw) }
    }
}

impl<T: Clone> Clone for Owned<T, Box<T>> {
    fn clone(&self) -> Self {
        Owned::new((**self).clone()).with_tag(self.tag())
    }
}

impl<T> From<T> for Owned<T, Box<T>> {
    fn from(t: T) -> Self {
        Owned::new(t)
    }
}
