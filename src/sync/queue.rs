//! A wait-free queue.
//!
//! The implementation is based on the following paper:
//!
//! Chaoran Yang and John Mellor-Crummey.  A Wait-free Queue as Fast as Fetch-and-Add.  PPoPP 2016.
//! <sup>[pdf][wfqueue]</sup>
//!
//! [wfqueue]: http://chaoran.me/assets/pdf/wfq-ppopp16.pdf

use std::marker::PhantomData;

use {Namespace, Scope};


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

    pub fn try_pop<N: Namespace>(&self, scope: &Scope<N>) -> Option<T> {
        unimplemented!()
    }

    pub fn try_pop_if<F, N: Namespace>(&self, condition: F, scope: &Scope<N>) -> Option<T>
        where F: Fn(&T) -> bool {
        unimplemented!()
    }
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self::new()
    }
}
