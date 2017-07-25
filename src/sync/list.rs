use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

use {Atomic, Owned, Ptr, Namespace, Scope, unprotected};


/// An entry in the linked list.
pub struct ListEntry<T> {
    /// The data in the entry.
    data: T,

    /// The next entry in the linked list.
    /// If the tag is 1, this entry is marked as deleted.
    next: Atomic<ListEntry<T>>,
}

pub struct List<T> {
    head: Atomic<ListEntry<T>>,
}

pub struct Iter<'scope, N, T: 'scope> where
    N: Namespace + 'scope,
{
    /// The scope in which the iterator is operating.
    scope: &'scope Scope<N>,

    /// Pointer from the predecessor to the current entry.
    pred: &'scope Atomic<ListEntry<T>>,

    /// The current entry.
    curr: Ptr<'scope, ListEntry<T>>,
}

pub enum IterResult<'scope, T: 'scope> {
    Some(&'scope T),
    None,
    Abort,
}

impl<T> ListEntry<T> {
    /// Returns the data in this entry.
    pub fn get(&self) -> &T {
        &self.data
    }

    /// Marks this entry as deleted.
    pub fn delete<'scope, N>(&self, scope: &Scope<N>) where
        N: Namespace + 'scope,
    {
        self.next.fetch_or(1, Release, scope);
    }
}

impl<T> List<T> {
    /// Returns a new, empty linked list.
    pub fn new() -> Self {
        List { head: Atomic::null() }
    }

    /// Inserts `data` into the list.
    pub fn insert<'scope, N>(&'scope self, to: &'scope Atomic<ListEntry<T>>, data: T, scope: &'scope Scope<N>) -> Ptr<'scope, ListEntry<T>> where
        N: Namespace + 'scope,
    {
        let mut cur = Owned::new(ListEntry {
            data: data,
            next: Atomic::null(),
        });
        let mut next = to.load(Relaxed, scope);

        loop {
            cur.next.store(next, Relaxed);
            match to.compare_and_set_weak_owned(next, cur, Release, scope) {
                Ok(cur) => return cur,
                Err((n, c)) => {
                    next = n;
                    cur = c;
                }
            }
        }
    }

    pub fn insert_head<'scope, N>(&'scope self, data: T, scope: &'scope Scope<N>) -> Ptr<'scope, ListEntry<T>> where
        N: Namespace + 'scope,
    {
        self.insert(&self.head, data, scope)
    }

    /// Returns an iterator over all data.
    ///
    /// Every datum that is inserted at the moment this function is called and persists at least
    /// until the end of iteration will be returned. Since this iterator traverses a lock-free linked
    /// list that may be concurrently modified, some additional caveats apply:
    ///
    /// 1. If a new datum is inserted during iteration, it may or may not be returned.
    /// 2. If a datum is deleted during iteration, it may or may not be returned.
    /// 3. It may not return all data if a concurrent thread continues to iterate the same list.
    pub fn iter<'scope, N>(&'scope self, scope: &'scope Scope<N>) -> Iter<'scope, N, T> where
        N: Namespace + 'scope,
    {
        let pred = &self.head;
        let curr = pred.load(Acquire, scope);
        Iter { scope, pred, curr }
    }
}

impl<T> Drop for List<T> {
    fn drop(&mut self) {
        unsafe {
            unprotected(|scope| {
                let mut curr = self.head.load(Relaxed, scope);
                while let Some(c) = curr.as_ref() {
                    let succ = c.next.load(Relaxed, scope);
                    scope.defer_free(curr);
                    curr = succ;
                }
            })
        }
    }
}

impl<'scope, N, T> Iter<'scope, N, T> where
    N: Namespace + 'scope,
{
    pub fn next(&mut self) -> IterResult<T> {
        while let Some(c) = unsafe { self.curr.as_ref() } {
            let succ = c.next.load(Acquire, self.scope);

            if succ.tag() == 1 {
                // This entry was removed. Try unlinking it from the list.
                let succ = succ.with_tag(0);

                match self.pred.compare_and_set_weak(self.curr, succ, Acquire, self.scope) {
                    Ok(_) => {
                        unsafe { self.scope.defer_free(self.curr); }
                        self.curr = succ;
                    },
                    Err(_) => {
                        // We lost the race to delete the entry.  Since another thread trying
                        // to iterate the list has won the race, we return early.
                        return IterResult::Abort;
                    }
                }

                continue;
            }

            // Move one step forward.
            self.pred = &c.next;
            self.curr = succ;

            return IterResult::Some(&c.data);
        }

        // We reached the end of the list.
        IterResult::None
    }
}
