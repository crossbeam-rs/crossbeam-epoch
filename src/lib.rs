mod atomic;

pub use atomic::{Atomic, Owned, Ptr};

pub struct Scope {
    _private: (),
}

pub fn pin<F, T>(f: F) -> T
where
    F: FnOnce(&Scope) -> T
{
    // TODO: Implement actual pinning.

    let scope = &Scope { _private: () };
    f(scope)
}
