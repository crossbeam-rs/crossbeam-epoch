use std::{mem, ptr};
use std::sync::atomic::Ordering::{Relaxed, Acquire, Release};

use {Atomic, Owned, Ptr, Namespace, Scope, pin};
use util::cache_padded::CachePadded;

/// A Michael-Scott lock-free queue, with support for blocking `pop`s.
///
/// Usable with any number of producers and consumers.
// The representation here is a singly-linked list, with a sentinel
// node at the front. In general the `tail` pointer may lag behind the
// actual tail. Non-sentinel nodes are either all `Data` or all
// `Blocked` (requests for data from blocked threads).
#[derive(Debug)]
pub struct MsQueue<N: Namespace, T> {
    namespace: N,
    head: CachePadded<Atomic<Node<T>>>,
    tail: CachePadded<Atomic<Node<T>>>,
}

#[derive(Debug)]
struct Node<T> {
    data: T,
    next: Atomic<Node<T>>,
}

// Any particular `T` should never accessed concurrently, so no need
// for Sync.
unsafe impl<N:Namespace, T: Send> Sync for MsQueue<N, T> {}
unsafe impl<N:Namespace, T: Send> Send for MsQueue<N, T> {}


impl<N:Namespace, T> MsQueue<N, T> {
    /// Create a new, empty queue.
    pub fn new(namespace: N) -> MsQueue<N, T> {
        let q = MsQueue {
            namespace: namespace,
            head: CachePadded::new(Atomic::null()),
            tail: CachePadded::new(Atomic::null()),
        };
        let sentinel = Owned::new(Node {
            data: unsafe { mem::uninitialized() },
            next: Atomic::null(),
        });
        unsafe {
            namespace.unprotected(|scope| {
                let sentinel = sentinel.into_ptr(scope);
                q.head.store(sentinel, Relaxed);
                q.tail.store(sentinel, Relaxed);
                q
            })
        }
    }

    #[inline(always)]
    /// Attempt to atomically place `n` into the `next` pointer of `onto`.
    ///
    /// If unsuccessful, returns ownership of `n`, possibly updating
    /// the queue's `tail` pointer.
    fn push_internal(&self,
                     onto: Ptr<Node<T>>,
                     new: Owned<Node<T>>,
                     scope: &Scope<N>)
                     -> Result<(), Owned<Node<T>>>
    {
        // is `onto` the actual tail?
        let o = unsafe { onto.deref() };
        let next = o.next.load(Acquire, scope);
        if unsafe { next.as_ref().is_some() } {
            // if not, try to "help" by moving the tail pointer forward
            let _ = self.tail.compare_and_set(onto, next, Release, scope);
            Err(new)
        } else {
            // looks like the actual tail; attempt to link in `n`
            o.next.compare_and_set_owned(Ptr::null(), new, Release, scope)
                .map(|new| {
                    // try to move the tail pointer forward
                    let _ = self.tail.compare_and_set(onto, new, Release, scope);
                })
                .map_err(|(_, new)| { new })
        }
    }

    /// Add `t` to the back of the queue, possibly waking up threads
    /// blocked on `pop`.
    pub fn push(&self, t: T, scope: &Scope<N>) {
        let mut new = Owned::new(Node {
            data: t,
            next: Atomic::null()
        });

        loop {
            // We push onto the tail, so we'll start optimistically by looking
            // there first.
            let tail = self.tail.load(Acquire, scope);

            // Attempt to push onto the `tail` snapshot; fails if
            // `tail.next` has changed, which will always be the case if the
            // queue has transitioned to blocking mode.
            match self.push_internal(tail, new, scope) {
                Ok(_) => break,
                Err(temp) => {
                    // retry
                    new = temp
                }
            }
        }
    }

    #[inline(always)]
    // Attempt to pop a data node. `Ok(None)` if queue is empty or in blocking
    // mode; `Err(())` if lost race to pop.
    fn pop_internal(&self, scope: &Scope<N>) -> Result<Option<T>, ()> {
        let head = self.head.load(Acquire, scope);
        let h = unsafe { head.deref() };
        let next = h.next.load(Acquire, scope);
        match unsafe { next.as_ref() } {
            None => Ok(None),
            Some(n) => {
                unsafe {
                    if self.head.compare_and_set(head, next, Release, scope).is_ok() {
                        scope.defer_free(head);
                        Ok(Some(ptr::read(&n.data)))
                    } else {
                        Err(())
                    }
                }
            }
        }
    }

    /// Check if this queue is empty.
    pub fn is_empty(&self) -> bool {
        pin(|scope| {
            let head = self.head.load(Acquire, scope);
            let h = unsafe { head.deref() };
            h.next.load(Acquire, scope).is_null()
        })
    }

    /// Attempt to dequeue from the front.
    ///
    /// Returns `None` if the queue is observed to be empty.
    pub fn try_pop(&self, scope: &Scope<N>) -> Option<T> {
        loop {
            if let Ok(r) = self.pop_internal(scope) {
                return r;
            }
        }
    }

    pub fn try_pop_if<F>(&self, condition: F, scope: &Scope<N>) -> Option<T> where
        F: Fn(&T) -> bool,
    {
        loop {
            if let Ok(head) = self.pop_internal(scope) {
                match head {
                    None => return None,
                    Some(h) => {
                        if condition(&h) {
                            return Some(h);
                        } else {
                            mem::forget(h);
                            return None
                        }
                    },
                }
            }
        }
    }
}

impl<N: Namespace, T> Drop for MsQueue<N, T> {
    fn drop(&mut self) {
        unsafe {
            self.namespace.unprotected(|scope| {
                while let Some(_) = self.try_pop(scope) {}

                // Destroy the remaining sentinel node.
                let sentinel = self.head.load(Relaxed, scope).as_raw() as *mut Node<T>;
                drop(Vec::from_raw_parts(sentinel, 0, 1));
            })
        }
    }
}


#[cfg(test)]
mod test {
    use {GlobalNamespace};
    use util::scoped;
    use super::*;

    struct MsQueue<T> {
        queue: super::MsQueue<GlobalNamespace, T>,
    }

    impl<T> MsQueue<T> {
        pub fn new() -> MsQueue<T> {
            MsQueue { queue: super::MsQueue::new(GlobalNamespace::new()) }
        }

        pub fn push(&self, t: T) {
            pin(|scope| { self.queue.push(t, scope) })
        }

        pub fn is_empty(&self) -> bool {
            self.queue.is_empty()
        }

        pub fn try_pop(&self) -> Option<T> {
            pin(|scope| { self.queue.try_pop(scope) })
        }

        pub fn pop(&self) -> T {
            loop {
                match self.try_pop() {
                    None => continue,
                    Some(t) => return t,
                }
            }
        }
    }

    const CONC_COUNT: i64 = 1000000;

    #[test]
    fn push_try_pop_1() {
        let q: MsQueue<i64> = MsQueue::new();
        assert!(q.is_empty());
        q.push(37);
        assert!(!q.is_empty());
        assert_eq!(q.try_pop(), Some(37));
        assert!(q.is_empty());
    }

    #[test]
    fn push_try_pop_2() {
        let q: MsQueue<i64> = MsQueue::new();
        assert!(q.is_empty());
        q.push(37);
        q.push(48);
        assert_eq!(q.try_pop(), Some(37));
        assert!(!q.is_empty());
        assert_eq!(q.try_pop(), Some(48));
        assert!(q.is_empty());
    }

    #[test]
    fn push_try_pop_many_seq() {
        let q: MsQueue<i64> = MsQueue::new();
        assert!(q.is_empty());
        for i in 0..200 {
            q.push(i)
        }
        assert!(!q.is_empty());
        for i in 0..200 {
            assert_eq!(q.try_pop(), Some(i));
        }
        assert!(q.is_empty());
    }

    #[test]
    fn push_pop_1() {
        let q: MsQueue<i64> = MsQueue::new();
        assert!(q.is_empty());
        q.push(37);
        assert!(!q.is_empty());
        assert_eq!(q.pop(), 37);
        assert!(q.is_empty());
    }

    #[test]
    fn push_pop_2() {
        let q: MsQueue<i64> = MsQueue::new();
        q.push(37);
        q.push(48);
        assert_eq!(q.pop(), 37);
        assert_eq!(q.pop(), 48);
    }

    #[test]
    fn push_pop_many_seq() {
        let q: MsQueue<i64> = MsQueue::new();
        assert!(q.is_empty());
        for i in 0..200 {
            q.push(i)
        }
        assert!(!q.is_empty());
        for i in 0..200 {
            assert_eq!(q.pop(), i);
        }
        assert!(q.is_empty());
    }

    #[test]
    fn push_try_pop_many_spsc() {
        let q: MsQueue<i64> = MsQueue::new();
        assert!(q.is_empty());

        scoped::scope(|scope| {
            scope.spawn(|| {
                let mut next = 0;

                while next < CONC_COUNT {
                    if let Some(elem) = q.try_pop() {
                        assert_eq!(elem, next);
                        next += 1;
                    }
                }
            });

            for i in 0..CONC_COUNT {
                q.push(i)
            }
        });
    }

    #[test]
    fn push_try_pop_many_spmc() {
        fn recv(_t: i32, q: &MsQueue<i64>) {
            let mut cur = -1;
            for _i in 0..CONC_COUNT {
                if let Some(elem) = q.try_pop() {
                    assert!(elem > cur);
                    cur = elem;

                    if cur == CONC_COUNT - 1 { break }
                }
            }
        }

        let q: MsQueue<i64> = MsQueue::new();
        assert!(q.is_empty());
        let qr = &q;
        scoped::scope(|scope| {
            for i in 0..3 {
                scope.spawn(move || recv(i, qr));
            }

            scope.spawn(|| {
                for i in 0..CONC_COUNT {
                    q.push(i);
                }
            })
        });
    }

    #[test]
    fn push_try_pop_many_mpmc() {
        enum LR { Left(i64), Right(i64) }

        let q: MsQueue<LR> = MsQueue::new();
        assert!(q.is_empty());

        scoped::scope(|scope| {
            for _t in 0..2 {
                scope.spawn(|| {
                    for i in CONC_COUNT-1..CONC_COUNT {
                        q.push(LR::Left(i))
                    }
                });
                scope.spawn(|| {
                    for i in CONC_COUNT-1..CONC_COUNT {
                        q.push(LR::Right(i))
                    }
                });
                scope.spawn(|| {
                    let mut vl = vec![];
                    let mut vr = vec![];
                    for _i in 0..CONC_COUNT {
                        match q.try_pop() {
                            Some(LR::Left(x)) => vl.push(x),
                            Some(LR::Right(x)) => vr.push(x),
                            _ => {}
                        }
                    }

                    let mut vl2 = vl.clone();
                    let mut vr2 = vr.clone();
                    vl2.sort();
                    vr2.sort();

                    assert_eq!(vl, vl2);
                    assert_eq!(vr, vr2);
                });
            }
        });
    }

    #[test]
    fn push_pop_many_spsc() {
        let q: MsQueue<i64> = MsQueue::new();

        scoped::scope(|scope| {
            scope.spawn(|| {
                let mut next = 0;
                while next < CONC_COUNT {
                    assert_eq!(q.pop(), next);
                    next += 1;
                }
            });

            for i in 0..CONC_COUNT {
                q.push(i)
            }
        });
        assert!(q.is_empty());
    }

    #[test]
    fn is_empty_dont_pop() {
        let q: MsQueue<i64> = MsQueue::new();
        q.push(20);
        q.push(20);
        assert!(!q.is_empty());
        assert!(!q.is_empty());
        assert!(q.try_pop().is_some());
    }
}
