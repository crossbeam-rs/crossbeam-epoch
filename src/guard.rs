use std::ptr;

use garbage::Garbage;
use internal::Local;

/// A guard that keeps the current thread pinned.
///
/// # Pinning
///
/// The current thread is pinned by calling [`pin`], which returns a new guard:
///
/// ```
/// use crossbeam_epoch as epoch;
///
/// // It is often convenient to prefix a call to `pin` with a `&` in order to create a reference.
/// // This is not really necessary, but makes passing references to the guard a bit easier.
/// let guard = &epoch::pin();
/// ```
///
/// When a guard gets dropped, the current thread is automatically unpinned.
///
/// # Pointers on the stack
///
/// Having a guard allows us to create pointers on the stack to heap-allocated objects.
/// For example:
///
/// ```
/// use crossbeam_epoch::{self as epoch, Atomic, Owned};
/// use std::sync::atomic::Ordering::SeqCst;
///
/// // Create a heap-allocated number.
/// let a = Atomic::new(777);
///
/// // Pin the current thread.
/// let guard = &epoch::pin();
///
/// // Load the heap-allocated object and create pointer `p` on the stack.
/// let p = a.load(SeqCst, guard);
///
/// // Dereference the pointer and print the value:
/// if let Some(num) = unsafe { p.as_ref() } {
///     println!("The number is {}.", num);
/// }
/// ```
///
/// # Multiple guards
///
/// Pinning is reentrant and it is perfectly legal to create multiple guards. In that case, the
/// thread will be pinned only when the first guard is created and unpinned when the last one is
/// dropped:
///
/// ```
/// use crossbeam_epoch as epoch;
///
/// let guard1 = epoch::pin();
/// let guard2 = epoch::pin();
/// assert!(epoch::is_pinned());
/// drop(guard1);
/// assert!(epoch::is_pinned());
/// drop(guard2);
/// assert!(!epoch::is_pinned());
/// ```
///
/// [`pin`]: fn.pin.html
pub struct Guard {
    pub(crate) local: *const Local,
}

impl Guard {
    /// Stores a function so that it can be executed at some point after all currently pinned
    /// threads get unpinned.
    ///
    /// This method first stores `f` into the thread-local (or handle-local) cache. If this cache
    /// becomes full, some functions are moved into the global cache. At the same time, some
    /// functions from both local and global caches may get executed in order to incrementally
    /// clean up the caches as they fill up.
    ///
    /// There is no guarantee when exactly `f` will be executed. The only guarantee is that won't
    /// until all currently pinned threads get unpinned. In theory, `f` might never be deallocated,
    /// but the epoch-based garbage collection will make an effort to execute it reasonably soon.
    ///
    /// If this method is called from an [`unprotected`] guard, the function will simply be
    /// executed immediately.
    ///
    /// # Safety
    ///
    /// The given function must not hold reference onto the stack. It is highly recommended that
    /// the passed function is **always** marked with `move` in order to prevent accidental
    /// borrows.
    ///
    /// ```
    /// use crossbeam_epoch as epoch;
    ///
    /// let guard = &epoch::pin();
    /// let message = "Hello!";
    /// unsafe {
    ///     // ALWAYS use `move` when sending a closure into `defef`.
    ///     guard.defer(move || {
    ///         println!("{}", message);
    ///     });
    /// }
    /// ```
    ///
    /// Apart from that, keep in mind that another thread may execute `f`, so anything accessed
    /// by the closure must be `Send`.
    ///
    /// # Examples
    ///
    /// When a heap-allocated object in a data structure becomes unreachable, it has to be
    /// deallocated. However, the current thread and other threads may be still holding references
    /// on the stack to that same object. Therefore it cannot be deallocated before those
    /// references get dropped. This method can defer deallocation until all those threads get
    /// unpinned and consequently drop all their references on the stack.
    ///
    /// ```rust
    /// use crossbeam_epoch::{self as epoch, Atomic, Owned};
    /// use std::sync::atomic::Ordering::SeqCst;
    ///
    /// let a = Atomic::new("foo");
    ///
    /// // Now suppose that `a` is shared among multiple threads and concurrently
    /// // accessed and modified...
    ///
    /// // Pin the current thread.
    /// let guard = &epoch::pin();
    ///
    /// // Steal the object currently stored in `a` and swap it with another one.
    /// let p = a.swap(Owned::new("bar").into_ptr(guard), SeqCst, guard);
    ///
    /// if !p.is_null() {
    ///     // The object `p` is pointing to is now unreachable.
    ///     // Defer its deallocation until all currently pinned threads get unpinned.
    ///     unsafe {
    ///         // ALWAYS use `move` when sending a closure into `defer`.
    ///         guard.defer(move || {
    ///             println!("{} is now being deallocated.", p.deref());
    ///             // Now we have unique access to the object pointed to by `p` and can turn it
    ///             // into an `Owned`. Dropping the `Owned` will deallocate the object.
    ///             drop(p.into_owned());
    ///         });
    ///     }
    /// }
    /// ```
    ///
    /// [`unprotected`]: fn.unprotected.html
    pub unsafe fn defer<F, R>(&self, f: F)
    where
        F: FnOnce() -> R + Send
    {
        let garbage = Garbage::new(|| drop(f()));

        if let Some(local) = self.local.as_ref() {
            local.defer(garbage, self);
        }
    }

    /// Clears up the thread-local cache of deferred functions by executing them or moving into the
    /// global cache.
    ///
    /// Call this method after deferring execution of a function if you want to get it executed as
    /// soon as possible. Flushing will make sure it is residing in in the global cache, so that
    /// any thread has a chance of taking the function and executing it.
    ///
    /// If this method is called from an [`unprotected`] guard, it is a no-op (nothing happens).
    ///
    /// # Examples
    ///
    /// ```
    /// use crossbeam_epoch as epoch;
    ///
    /// let guard = &epoch::pin();
    /// unsafe {
    ///     guard.defer(move || {
    ///         println!("This better be printed as soon as possible!");
    ///     });
    /// }
    /// guard.flush();
    /// ```
    ///
    /// [`unprotected`]: fn.unprotected.html
    pub fn flush(&self) {
        if let Some(local) = unsafe { self.local.as_ref() } {
            local.flush(self);
        }
    }
}

impl Drop for Guard {
    #[inline]
    fn drop(&mut self) {
        if let Some(local) = unsafe { self.local.as_ref() } {
            Local::unpin(local);
        }
    }
}

/// Creates a dummy guard that doesn't really pin the current thread.
///
/// This is a function for special uses only. It creates a guard that can be used for loading
/// [`Atomic`]s, but will not pin or unpin the current thread. Calling [`defer`] with a dummy guard
/// will simply execute the function immediately.
///
/// # Safety
///
/// Loading and dereferencing data from an [`Atomic`] using a dummy guard is safe only if the
/// [`Atomic`] is not being concurrently modified by other threads.
///
/// # Examples
///
/// ```
/// use crossbeam_epoch as epoch;
///
/// unsafe {
///     let guard = &epoch::unprotected();
///     guard.defer(move || {
///         println!("This gets executed immediately.");
///     });
/// }
/// ```
///
/// The most common use of this function is when constructing or destructing a data structure.
///
/// For example, we can use a dummy guard in the destructor of a Treiber stack because at that
/// point no other thread could concurrently modify the [`Atomic`]s we are accessing:
///
/// ```
/// use crossbeam_epoch::{self as epoch, Atomic};
/// use std::mem::ManuallyDrop;
/// use std::sync::atomic::Ordering::Relaxed;
///
/// struct Stack<T> {
///     head: Atomic<Node<T>>,
/// }
///
/// struct Node<T> {
///     data: ManuallyDrop<T>,
///     next: Atomic<Node<T>>,
/// }
///
/// impl<T> Drop for Stack<T> {
///     fn drop(&mut self) {
///         unsafe {
///             // Create a dummy guard.
///             let guard = &epoch::unprotected();
///
///             let mut node = self.head.load(Relaxed, guard);
///
///             while let Some(n) = node.as_ref() {
///                 let next = n.next.load(Relaxed, guard);
///
///                 // Take ownership of the node, then drop its data and deallocate it.
///                 let mut o = node.into_owned();
///                 ManuallyDrop::drop(&mut o.data);
///                 drop(o);
///
///                 node = next;
///             }
///         }
///     }
/// }
/// ```
///
/// Really pinning the current thread would only unnecessarily delay garbage collection and incur
/// some performance cost, so in cases like these `unprotected` is of great help.
///
/// [`Atomic`]: struct.Atomic.html
/// [`defer`]: struct.Guard.html#method.defer
#[inline]
pub unsafe fn unprotected() -> Guard {
    Guard { local: ptr::null() }
}
