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
mod global;
mod user;
pub mod sync;
pub mod cache_padded;
pub mod scoped;

pub use self::atomic::{Atomic, CompareAndSetOrdering, Owned, Ptr};
pub use self::scope::{Namespace, Agent, Scope};
pub use self::global::{pin, is_pinned, unprotected, GlobalNamespace};
pub use self::user::UserNamespace;
