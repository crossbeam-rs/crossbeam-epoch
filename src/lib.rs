#![cfg_attr(feature = "nightly", feature(const_fn))]

#[macro_use(defer)]
extern crate scopeguard;

#[macro_use]
pub mod util;
mod atomic;
mod registry;
mod epoch;
mod garbage;
mod scope;
mod global;
pub mod sync;

pub use self::atomic::{Atomic, CompareAndSetOrdering, Owned, Ptr};
pub use self::global::{pin, is_pinned};
pub use self::scope::{Mutator, Scope, unprotected};
