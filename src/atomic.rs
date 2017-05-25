use std::marker::PhantomData;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use Scope;

/// Given ordering for the success case in a compare-exchange operation, returns the strongest
/// appropriate ordering for the failure case.
#[inline]
fn strongest_failure_ordering(ord: Ordering) -> Ordering {
    use self::Ordering::*;
    match ord {
        Relaxed => Relaxed,
        Release => Relaxed,
        Acquire => Acquire,
        AcqRel => Acquire,
        SeqCst => SeqCst,
        _ => SeqCst,
    }
}

/// Panics if the pointer is not properly unaligned.
#[inline]
fn ensure_aligned<T>(raw: *const T) {
    assert!(raw as usize & low_bits::<T>() == 0, "unaligned pointer");
}

/// Returns a bitmask containing the unused least significant bits of an aligned pointer to `T`.
#[inline]
fn low_bits<T>() -> usize {
    (1 << mem::align_of::<T>().trailing_zeros()) - 1
}

/// Given a tagged pointer `data`, returns the same pointer, but tagged with `tag`.
/// Panics if the tag doesn't fit into the unused bits of the pointer.
#[inline]
fn data_with_tag<T>(data: usize, tag: usize) -> usize {
    let mask = low_bits::<T>();
    assert!(tag <= mask, "tag too large to fit into the unused bits: {} > {}", tag, mask);
    (data & !mask) | tag
}

/// An atomic pointer that can be safely shared between threads.
///
/// The pointer must be properly aligned. Since it is aligned, a tag can be stored into the unused
/// least significant bits of the address.
///
/// Any method that loads the pointer must be passed a reference to a [`Scope`].
///
/// [`Scope`]: struct.Scope.html
#[derive(Debug)]
pub struct Atomic<T> {
    data: AtomicUsize,
    _marker: PhantomData<*mut T>,
}

unsafe impl<T: Send + Sync> Send for Atomic<T> {}
unsafe impl<T: Send + Sync> Sync for Atomic<T> {}

impl<T> Atomic<T> {
    /// Returns a new atomic pointer initialized with the tagged pointer `data`.
    fn from_data(data: usize) -> Self {
        Atomic {
            data: AtomicUsize::new(data),
            _marker: PhantomData,
        }
    }

    /// Returns a new null atomic pointer.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::Atomic;
    ///
    /// let a = Atomic::<i32>::null();
    /// ```
    pub fn null() -> Self {
        Atomic {
            data: AtomicUsize::new(0),
            _marker: PhantomData,
        }
    }

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
        Self::from_owned(Owned::new(value))
    }

    /// Returns a new atomic pointer pointing to `owned`.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{Atomic, Owned};
    ///
    /// let a = Atomic::from_owned(Owned::new(1234));
    /// ```
    pub fn from_owned(owned: Owned<T>) -> Self {
        let data = owned.data;
        mem::forget(owned);
        Self::from_data(data)
    }

    /// Returns a new atomic pointer pointing to `ptr`.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{Atomic, Ptr};
    ///
    /// let a = Atomic::from_ptr(Ptr::<i32>::null());
    /// ```
    pub fn from_ptr(ptr: Ptr<T>) -> Self {
        Self::from_data(ptr.data)
    }

    /// Loads a `Ptr` from the atomic pointer.
    ///
    /// This method takes an [`Ordering`] argument which describes the memory ordering of this
    /// operation.
    ///
    /// [`Ordering`]: https://doc.rust-lang.org/std/sync/atomic/enum.Ordering.html
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::new(1234);
    /// epoch::pin(|scope| {
    ///     let p = a.load(SeqCst, scope);
    /// });
    /// ```
    pub fn load<'scope>(&self, ord: Ordering, _: &'scope Scope) -> Ptr<'scope, T> {
        Ptr::from_data(self.data.load(ord))
    }

    /// Stores a `Ptr` into the atomic pointer.
    ///
    /// This method takes an [`Ordering`] argument which describes the memory ordering of this
    /// operation.
    ///
    /// [`Ordering`]: https://doc.rust-lang.org/std/sync/atomic/enum.Ordering.html
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic, Ptr};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::new(1234);
    /// a.store(Ptr::null(), SeqCst);
    /// ```
    pub fn store(&self, new: Ptr<T>, ord: Ordering) {
        self.data.store(new.data, ord);
    }

    /// Stores an `Owned` into the atomic pointer.
    ///
    /// This method takes an [`Ordering`] argument which describes the memory ordering of this
    /// operation.
    ///
    /// [`Ordering`]: https://doc.rust-lang.org/std/sync/atomic/enum.Ordering.html
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic, Owned};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::null();
    /// a.store_owned(Owned::new(1234), SeqCst);
    /// ```
    pub fn store_owned(&self, new: Owned<T>, ord: Ordering) {
        let data = new.data;
        mem::forget(new);
        self.data.store(data, ord);
    }

    /// Stores a `Ptr` into the atomic pointer, returning the previous `Ptr`.
    ///
    /// This method takes an [`Ordering`] argument which describes the memory ordering of this
    /// operation.
    ///
    /// [`Ordering`]: https://doc.rust-lang.org/std/sync/atomic/enum.Ordering.html
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic, Owned, Ptr};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::new(1234);
    /// epoch::pin(|scope| {
    ///     let p = a.swap(Ptr::null(), SeqCst, scope);
    /// });
    /// ```
    pub fn swap<'scope>(&self, new: Ptr<T>, ord: Ordering, _: &'scope Scope) -> Ptr<'scope, T> {
        Ptr::from_data(self.data.swap(new.data, ord))
    }

    /// Stores `new` into the atomic pointer if the current value is the same as `current`.
    ///
    /// The return value is a result indicating whether the new pointer was written. On failure the
    /// actual current value is returned.
    ///
    /// This method takes an [`Ordering`] argument which describes the memory ordering of this
    /// operation.
    ///
    /// [`Ordering`]: https://doc.rust-lang.org/std/sync/atomic/enum.Ordering.html
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic, Ptr};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::new(1234);
    ///
    /// epoch::pin(|scope| {
    ///     let mut curr = a.load(SeqCst, scope);
    ///     let res = a.compare_and_swap(curr, Ptr::null(), SeqCst, scope);
    /// });
    /// ```
    pub fn compare_and_swap<'scope>(
        &self,
        current: Ptr<T>,
        new: Ptr<T>,
        ord: Ordering,
        _: &'scope Scope,
    ) -> Result<(), Ptr<'scope, T>> {
        let fail_ord = strongest_failure_ordering(ord);
        match self.data.compare_exchange(current.data, new.data, ord, fail_ord) {
            Ok(_) => Ok(()),
            Err(previous) => Err(Ptr::from_data(previous)),
        }
    }

    /// Stores `new` into the atomic pointer if the current value is the same as `current`.
    ///
    /// Unlike [`compare_and_swap`], this method is allowed to spuriously fail even when
    /// comparison succeeds, which can result in more efficient code on some platforms.
    /// The return value is a result indicating whether the new pointer was written. On failure the
    /// actual current value is returned.
    ///
    /// This method takes an [`Ordering`] argument which describes the memory ordering of this
    /// operation.
    ///
    /// [`compare_and_swap`]: struct.Atomic.html#method.compare_and_swap
    /// [`Ordering`]: https://doc.rust-lang.org/std/sync/atomic/enum.Ordering.html
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic, Ptr};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::new(1234);
    ///
    /// epoch::pin(|scope| {
    ///     let mut curr = a.load(SeqCst, scope);
    ///     loop {
    ///         match a.compare_and_swap_weak(curr, Ptr::null(), SeqCst, scope) {
    ///             Ok(()) => break,
    ///             Err(c) => curr = c,
    ///         }
    ///     }
    /// });
    /// ```
    pub fn compare_and_swap_weak<'scope>(
        &self,
        current: Ptr<T>,
        new: Ptr<T>,
        ord: Ordering,
        _: &'scope Scope,
    ) -> Result<(), Ptr<'scope, T>> {
        let fail_ord = strongest_failure_ordering(ord);
        match self.data.compare_exchange_weak(current.data, new.data, ord, fail_ord) {
            Ok(_) => Ok(()),
            Err(previous) => Err(Ptr::from_data(previous)),
        }
    }

    /// Stores `new` into the atomic pointer if the current value is the same as `current`.
    ///
    /// The return value is a result indicating whether the new pointer was written. On success the
    /// pointer that was written is returned. On failure `new` and the actual current value are
    /// returned.
    ///
    /// This method takes an [`Ordering`] argument which describes the memory ordering of this
    /// operation.
    ///
    /// [`Ordering`]: https://doc.rust-lang.org/std/sync/atomic/enum.Ordering.html
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic, Owned};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::new(1234);
    ///
    /// epoch::pin(|scope| {
    ///     let mut curr = a.load(SeqCst, scope);
    ///     let res = a.compare_and_swap_owned(curr, Owned::new(5678), SeqCst, scope);
    /// });
    /// ```
    pub fn compare_and_swap_owned<'scope>(
        &self,
        current: Ptr<T>,
        new: Owned<T>,
        ord: Ordering,
        _: &'scope Scope,
    ) -> Result<Ptr<'scope, T>, (Ptr<'scope, T>, Owned<T>)> {
        let fail_ord = strongest_failure_ordering(ord);
        match self.data.compare_exchange(current.data, new.data, ord, fail_ord) {
            Ok(_) => {
                let data = new.data;
                mem::forget(new);
                Ok(Ptr::from_data(data))
            }
            Err(previous) => Err((Ptr::from_data(previous), new)),
        }
    }

    /// Stores `new` into the atomic pointer if the current value is the same as `current`.
    ///
    /// Unlike [`compare_and_swap_owned`], this method is allowed to spuriously fail even when
    /// comparison succeeds, which can result in more efficient code on some platforms.
    /// The return value is a result indicating whether the new pointer was written. On success the
    /// pointer that was written is returned. On failure `new` and the actual current value are
    /// returned.
    ///
    /// This method takes an [`Ordering`] argument which describes the memory ordering of this
    /// operation.
    ///
    /// [`compare_and_swap_owned`]: struct.Atomic.html#method.compare_and_swap_owned
    /// [`Ordering`]: https://doc.rust-lang.org/std/sync/atomic/enum.Ordering.html
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic, Owned};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::new(1234);
    ///
    /// epoch::pin(|scope| {
    ///     let mut new = Owned::new(5678);
    ///     let mut ptr = a.load(SeqCst, scope);
    ///     loop {
    ///         match a.compare_and_swap_weak_owned(ptr, new, SeqCst, scope) {
    ///             Ok(p) => {
    ///                 ptr = p;
    ///                 break;
    ///             }
    ///             Err((p, n)) => {
    ///                 ptr = p;
    ///                 new = n;
    ///             }
    ///         }
    ///     }
    /// });
    /// ```
    pub fn compare_and_swap_weak_owned<'scope>(
        &self,
        current: Ptr<T>,
        new: Owned<T>,
        ord: Ordering,
        _: &'scope Scope,
    ) -> Result<Ptr<'scope, T>, (Ptr<'scope, T>, Owned<T>)> {
        let fail_ord = strongest_failure_ordering(ord);
        match self.data.compare_exchange_weak(current.data, new.data, ord, fail_ord) {
            Ok(_) => {
                let data = new.data;
                mem::forget(new);
                Ok(Ptr::from_data(data))
            }
            Err(previous) => Err((Ptr::from_data(previous), new)),
        }
    }
}

impl<T> Default for Atomic<T> {
    fn default() -> Self {
        Atomic::null()
    }
}

/// An owned heap-allocated object.
///
/// This type is very similar to `Box<T>`.
///
/// The pointer must be properly aligned. Since it is aligned, a tag can be stored into the unused
/// least significant bits of the address.
#[derive(Debug)]
pub struct Owned<T> {
    data: usize,
    _marker: PhantomData<Box<T>>,
}

impl<T> Owned<T> {
    /// Returns a new owned pointer initialized with the tagged pointer `data`.
    fn from_data(data: usize) -> Self {
        Owned {
            data: data,
            _marker: PhantomData,
        }
    }

    /// Allocates `value` on the heap and returns a new owned pointer pointing to it.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::Owned;
    ///
    /// let o = Owned::new(1234);
    /// ```
    pub fn new(value: T) -> Self {
        Self::from_box(Box::new(value))
    }

    /// Returns a new owned pointer initialized with `b`.
    ///
    /// # Panics
    ///
    /// Panics if the pointer (the `Box`) is not properly aligned.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::Owned;
    ///
    /// let o = unsafe { Owned::from_raw(Box::into_raw(Box::new(1234))) };
    /// ```
    pub fn from_box(b: Box<T>) -> Self {
        unsafe { Self::from_raw(Box::into_raw(b)) }
    }

    /// Returns a new owned pointer initialized with `raw`.
    ///
    /// # Panics
    ///
    /// Panics if `raw` is not properly aligned.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::Owned;
    ///
    /// let o = unsafe { Owned::from_raw(Box::into_raw(Box::new(1234))) };
    /// ```
    pub unsafe fn from_raw(raw: *mut T) -> Self {
        ensure_aligned(raw);
        Self::from_data(raw as usize)
    }

    /// Converts the owned pointer to a [`Ptr`].
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Owned};
    ///
    /// let o = Owned::new(1234);
    /// epoch::pin(|scope| {
    ///     let p = o.into_ptr(scope);
    /// });
    /// ```
    ///
    /// [`Ptr`]: struct.Ptr.html
    pub fn into_ptr<'scope>(self, _: &'scope Scope) -> Ptr<'scope, T> {
        let data = self.data;
        mem::forget(self);
        Ptr::from_data(data)
    }

    /// Returns the tag stored within the pointer.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::Owned;
    ///
    /// assert_eq!(Owned::new(1234).tag(), 0);
    /// ```
    pub fn tag(&self) -> usize {
        self.data & low_bits::<T>()
    }

    /// Returns the same pointer, but tagged with `tag`.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::Owned;
    ///
    /// let o = Owned::new(0u64);
    /// assert_eq!(o.tag(), 0);
    /// let o = o.with_tag(5);
    /// assert_eq!(o.tag(), 5);
    /// ```
    pub fn with_tag(self, tag: usize) -> Self {
        let data = self.data;
        mem::forget(self);
        Self::from_data(data_with_tag::<T>(data, tag))
    }
}

impl<T> Deref for Owned<T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*((self.data & !low_bits::<T>()) as *const T) }
    }
}

impl<T> DerefMut for Owned<T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *((self.data & !low_bits::<T>()) as *mut T) }
    }
}

/// A pointer to an object protected by the epoch GC.
///
/// The pointer is valid for use only within `'scope`.
///
/// The pointer must be properly aligned. Since it is aligned, a tag can be stored into the unused
/// least significant bits of the address.
#[derive(Debug)]
pub struct Ptr<'scope, T: 'scope> {
    data: usize,
    _marker: PhantomData<&'scope T>,
}

impl<'scope, T> Clone for Ptr<'scope, T> {
    fn clone(&self) -> Self {
        Ptr {
            data: self.data,
            _marker: PhantomData,
        }
    }
}

impl<'scope, T> Copy for Ptr<'scope, T> {}

impl<'scope, T> Ptr<'scope, T> {
    /// Returns a new pointer initialized with the tagged pointer `data`.
    fn from_data(data: usize) -> Self {
        Ptr {
            data: data,
            _marker: PhantomData,
        }
    }

    /// Returns a new null pointer.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::Ptr;
    ///
    /// let p = Ptr::<i32>::null();
    /// assert!(p.is_null());
    /// ```
    pub fn null() -> Self {
        Ptr {
            data: 0,
            _marker: PhantomData,
        }
    }

    /// Returns a new pointer initialized with `raw`.
    ///
    /// # Panics
    ///
    /// Panics if `raw` is not properly aligned.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::Ptr;
    ///
    /// let p = unsafe { Ptr::from_raw(Box::into_raw(Box::new(1234))) };
    /// assert!(!p.is_null());
    /// ```
    pub unsafe fn from_raw(raw: *const T) -> Self {
        ensure_aligned(raw);
        Ptr {
            data: raw as usize,
            _marker: PhantomData,
        }
    }

    /// Returns `true` if the pointer is null.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic, Owned};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::null();
    /// epoch::pin(|scope| {
    ///     assert!(a.load(SeqCst, scope).is_null());
    ///     a.store_owned(Owned::new(1234), SeqCst);
    ///     assert!(!a.load(SeqCst, scope).is_null());
    /// });
    /// ```
    pub fn is_null(&self) -> bool {
        self.as_raw().is_null()
    }

    /// Converts the pointer to a raw pointer (without the tag).
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic, Owned};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let o = Owned::new(1234);
    /// let raw = &*o as *const _;
    /// let a = Atomic::from_owned(o);
    ///
    /// epoch::pin(|scope| {
    ///     let p = a.load(SeqCst, scope);
    ///     assert_eq!(p.as_raw(), raw);
    /// });
    /// ```
    pub fn as_raw(&self) -> *const T {
        (self.data & !low_bits::<T>()) as *const T
    }

    /// Dereferences the pointer.
    ///
    /// Returns a reference to the pointee that is valid in `'scope`.
    ///
    /// # Safety
    ///
    /// Dereferencing a pointer to an invalid object is not a concern, since invalid `Ptr`s
    /// can only be constructed via other unsafe functions.
    ///
    /// However, this method doesn't check whether the pointer is null, so dereferencing a null
    /// pointer is unsafe.
    ///
    /// Another source of unsafety is the possibility of unsynchronized reads to the objects.
    /// For example, the following scenario is unsafe:
    ///
    /// * A thread stores a new object: `a.store_owned(Owned::new(10), Relaxed)`
    /// * Another thread reads it: `*a.load(Relaxed, scope).as_ref().unwrap()`
    ///
    /// The problem is that relaxed orderings don't synchronize initialization of the object with
    /// the read from the second thread. This is a data race. A possible solution would be to use
    /// `Release` and `Acquire` orderings (or stronger).
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::new(1234);
    /// epoch::pin(|scope| {
    ///     let p = a.load(SeqCst, scope);
    ///     unsafe {
    ///         assert_eq!(p.deref(), &1234);
    ///     }
    /// });
    /// ```
    pub unsafe fn deref(&self) -> &'scope T {
        &*self.as_raw()
    }

    /// Converts the pointer to a reference.
    ///
    /// Returns `None` if the pointer is null, or else a reference to the object wrapped in `Some`.
    ///
    /// # Safety
    ///
    /// This method checks whether the pointer is null, and if not, assumes that it's pointing to a
    /// valid object. However, this is not considered a source of unsafety because invalid `Ptr`s
    /// can only be constructed via other unsafe functions.
    ///
    /// The only source of unsafety is the possibility of unsynchronized reads to the objects.
    /// For example, the following scenario is unsafe:
    ///
    /// * A thread stores a new object: `a.store_owned(Owned::new(10), Relaxed)`
    /// * Another thread reads it: `*a.load(Relaxed, scope).as_ref().unwrap()`
    ///
    /// The problem is that relaxed orderings don't synchronize initialization of the object with
    /// the read from the second thread. This is a data race. A possible solution would be to use
    /// `Release` and `Acquire` orderings (or stronger).
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::new(1234);
    /// epoch::pin(|scope| {
    ///     let p = a.load(SeqCst, scope);
    ///     unsafe {
    ///         assert_eq!(p.as_ref(), Some(&1234));
    ///     }
    /// });
    /// ```
    pub unsafe fn as_ref(&self) -> Option<&'scope T> {
        self.as_raw().as_ref()
    }

    /// Returns the tag stored within the pointer.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic, Owned};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::from_owned(Owned::new(0u64).with_tag(5));
    /// epoch::pin(|scope| {
    ///     let p = a.load(SeqCst, scope);
    ///     assert_eq!(p.tag(), 5);
    /// });
    /// ```
    pub fn tag(&self) -> usize {
        self.data & low_bits::<T>()
    }

    /// Returns the same pointer, but tagged with `tag`.
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch::{self as epoch, Atomic};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::new(0u64);
    /// epoch::pin(|scope| {
    ///     let p1 = a.load(SeqCst, scope);
    ///     let p2 = p1.with_tag(5);
    ///
    ///     assert_eq!(p1.tag(), 0);
    ///     assert_eq!(p2.tag(), 5);
    ///     assert_eq!(p1.as_raw(), p2.as_raw());
    /// });
    /// ```
    pub fn with_tag(&self, tag: usize) -> Self {
        Self::from_data(data_with_tag::<T>(self.data, tag))
    }
}

impl<'scope, T> Default for Ptr<'scope, T> {
    fn default() -> Self {
        Ptr::null()
    }
}
