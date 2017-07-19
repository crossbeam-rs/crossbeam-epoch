use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

use {Atomic, Owned, Ptr, Scope};

/// An entry in the linked list.
pub struct Entry<T> {
    /// The data in the entry.
    data: T,

    /// The next entry in the linked list.
    /// If the tag is 1, this entry is marked as deleted.
    next: Atomic<Entry<T>>,
}

pub struct List<T> {
    head: Atomic<Entry<T>>,
}

pub struct Iter<'scope, T: 'scope> {
    /// The scope in which the iterator is operating.
    scope: &'scope Scope,

    /// Pointer from the predecessor to the current entry.
    pred: &'scope Atomic<Entry<T>>,

    /// The current entry.
    curr: Ptr<'scope, Entry<T>>,
}

impl<T> Entry<T> {
    /// Returns the data in this entry.
    pub fn get(&self) -> &T {
        &self.data
    }

    /// Marks this entry as deleted.
    pub fn delete(&self, scope: &Scope) {
        self.next.fetch_or(1, Release, scope);
    }
}

impl<T> List<T> {
    /// Returns a new, empty linked list.
    pub fn new() -> Self {
        List { head: Atomic::null() }
    }

    /// Inserts `data` into the list.
    pub fn insert<'scope>(&'scope self, mut to: &'scope Atomic<Entry<T>>, data: T, scope: &'scope Scope) -> Ptr<'scope, Entry<T>> {
        let mut cur = Owned::new(Entry {
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

    pub fn insert_head<'scope>(&'scope self, data: T, scope: &'scope Scope) -> Ptr<'scope, Entry<T>> {
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
    /// 3. Any datum that gets returned may be returned multiple times.
    pub fn iter<'scope>(&'scope self, scope: &'scope Scope) -> Iter<'scope, T> {
        let pred = &self.head;
        let curr = pred.load(Acquire, scope);
        Iter { scope, pred, curr }
    }
}

impl<'scope, T> Iterator for Iter<'scope, T> {
    type Item = &'scope T;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(c) = unsafe { self.curr.as_ref() } {
            let succ = c.next.load(Acquire, self.scope);

            if succ.tag() == 1 {
                // This entry was removed. Try unlinking it from the list.
                let succ = succ.with_tag(0);

                match self.pred.compare_and_set_weak(self.curr, succ, Acquire, self.scope) {
                    Ok(_) => {
                        self.curr = succ;
                    },
                    Err(c) => {
                        self.curr = c;
                    }
                }

                // FIXME(jeehoonkang): call `drop` for the unlinked entry.

                continue;
            }

            // Move one step forward.
            self.pred = &c.next;
            self.curr = succ;

            return Some(&c.data);
        }

        // We reached the end of the list.
        None
    }
}
