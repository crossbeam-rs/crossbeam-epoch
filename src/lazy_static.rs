/// $NAME() returns a reference to the static object, which is lazily initialized.
#[macro_export]
macro_rules! lazy_static {
    ($VISIBILITY:tt, $NAME:ident, $T:ty, $init:expr) => {
        $VISIBILITY mod $NAME {
            use ::std::sync::atomic::{AtomicUsize, ATOMIC_USIZE_INIT};
            use ::std::sync::atomic::Ordering::{Relaxed, Acquire, Release};
            use super::*;

            static GLOBAL: AtomicUsize = ATOMIC_USIZE_INIT;

            #[inline]
            fn get_raw() -> usize {
                let current = GLOBAL.load(Acquire);
                if current != 0 {
                    return current;
                }

                // Initialize the singleton.
                let raw: *mut $T = Box::into_raw(Box::new($init));
                let new = raw as usize;
                let previous = GLOBAL.compare_and_swap(0, new, Release);

                if previous == 0 {
                    // Ok, we initialized it.
                    new
                } else {
                    // Another thread has already initialized it.
                    unsafe { drop(Box::from_raw(raw)); }
                    previous
                }
            }

            #[inline]
            pub fn get() -> &'static $T {
                unsafe { &*(get_raw() as *const $T) }
            }

            #[inline]
            pub unsafe fn get_unsafe() -> &'static $T {
                &*(GLOBAL.load(Relaxed) as *const $T)
            }
        }
    };
    ($VISIBILITY:tt, $NAME:ident, $T:ty) => {
        lazy_static!($VISIBILITY, $NAME, $T, Default::default());
    };
}

/// $NAME() returns a reference to the static object, which is lazily initialized to null.
#[macro_export]
macro_rules! lazy_static_null {
    ($VISIBILITY:tt, $NAME:ident, $T:ty) => {
        #[inline]
        $VISIBILITY fn $NAME() -> &'static $T {
            use ::std::sync::atomic::{AtomicUsize, ATOMIC_USIZE_INIT};

            static GLOBAL: AtomicUsize = ATOMIC_USIZE_INIT;
            unsafe { &*(&GLOBAL as *const AtomicUsize as *const $T) }
        }
    };
}
