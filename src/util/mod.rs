#![cfg_attr(feature = "nightly", feature(const_fn))]

#[macro_use]
pub mod lazy_static;
pub mod cache_padded;
pub mod scoped;
pub mod atomic_option;
