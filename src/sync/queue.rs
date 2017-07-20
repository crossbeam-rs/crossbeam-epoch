//! A wait-free queue.
//!
//! The implementation is based on the following paper:
//!
//! Chaoran Yang and John Mellor-Crummey.  A Wait-free Queue as Fast as Fetch-and-Add.  PPoPP 2016.
//! <sup>[pdf][wfqueue]</sup>
//!
//! [wfqueue]: http://chaoran.me/assets/pdf/wfq-ppopp16.pdf

use std::marker::PhantomData;

use {Scope};

pub struct Queue<T> {
    _marker: PhantomData<T>,
}

impl<T> Queue<T> {
    pub fn new() -> Queue<T> {
        unimplemented!()
    }

    pub fn push(&self, t: T) {
        unimplemented!()
    }

    pub fn try_pop(&self, scope: &Scope) -> Option<T> {
        unimplemented!()
    }

    pub fn append(&self, other: &Queue<T>) {
        unimplemented!()
    }
}
