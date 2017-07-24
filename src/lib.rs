#![cfg_attr(feature = "nightly", feature(const_fn))]

#[macro_use(defer)]
extern crate scopeguard;
extern crate boxfnonce;

#[macro_use]
mod lazy_static;
mod atomic;
mod registry;
mod epoch;
mod garbage;
mod scope;
pub mod sync;

pub use self::atomic::{Atomic, CompareAndSetOrdering, Owned, Ptr};
pub use self::scope::{Scope, pin, is_pinned, unprotected};
