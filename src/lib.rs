#![cfg_attr(feature = "nightly", feature(const_fn))]

#[macro_use(defer)]
extern crate scopeguard;
extern crate boxfnonce;

mod atomic;
mod garbage;
mod registry;
mod global;
mod scope;
pub mod sync;

pub use self::atomic::{Atomic, CompareAndSetOrdering, Owned, Ptr};
pub use self::scope::{Scope, pin, is_pinned, unprotected};
