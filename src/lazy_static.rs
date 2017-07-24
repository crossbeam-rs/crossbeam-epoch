/// $NAME() returns a reference to the static object, which is lazily initialized.
#[macro_export]
macro_rules! lazy_static {
    ($VISIBILITY:tt, $NAME:ident, $T:ty, $init:expr) => {
        #[inline]
        $VISIBILITY fn $NAME() -> &'static $T {
            use ::std::sync::atomic::{AtomicUsize, ATOMIC_USIZE_INIT};
            use ::std::sync::atomic::Ordering::{Acquire, Release};

            static GLOBAL: AtomicUsize = ATOMIC_USIZE_INIT;

            fn get() -> usize {
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

            unsafe { &*(get() as *const $T) }
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
