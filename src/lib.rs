#![cfg_attr(feature = "nightly", feature(const_fn))]

#[macro_use(defer)]
extern crate scopeguard;
extern crate boxfnonce;
extern crate arrayvec;

#[macro_use] pub mod util;
mod atomic;
mod registry;
mod epoch;
mod garbage;
mod scope;
mod global;
mod user;
pub mod sync;

pub use self::atomic::{Atomic, CompareAndSetOrdering, Owned, Ptr};
pub use self::scope::{Namespace, Agent, Scope};
pub use self::global::{pin, is_pinned, unprotected, GlobalNamespace};
pub use self::user::{with_namespace};
