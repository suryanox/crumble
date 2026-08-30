use std::collections::HashMap;
use std::sync::{Condvar, Mutex};

pub type TransactionId = u64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TxStatus {
    InProgress,
    Committed,
    Aborted,
}

#[derive(Debug)]
struct Inner {
    next_id: TransactionId,
    statuses: HashMap<TransactionId, TxStatus>,
    waits_for: HashMap<TransactionId, TransactionId>,
}

#[derive(Debug)]
pub struct TransactionManager {
    inner: Mutex<Inner>,
    completion: Condvar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeadlockDetected;

/// a Mutex lock can only fail if a different thread panicked while holding it (a "poisoned" lock) a real, exceptional condition worth crashing loudly on for now, not silently working around.
impl TransactionManager {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                next_id: 1,
                statuses: HashMap::new(),
                waits_for: HashMap::new(),
            }),
            completion: Condvar::new(),
        }
    }

    pub fn begin(&self) -> TransactionId {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_id;
        inner.next_id += 1;
        inner.statuses.insert(id, TxStatus::InProgress);
        id
    }

    pub fn commit(&self, xid: TransactionId) {
        let mut inner = self.inner.lock().unwrap();
        inner.statuses.insert(xid, TxStatus::Committed);
        drop(inner); // releasing the lock before waking waiters, not after. If we notified while still holding the lock, woken threads would immediately re-block trying to reacquire it
        self.completion.notify_all();
    }

    pub fn abort(&self, xid: TransactionId) {
        let mut inner = self.inner.lock().unwrap();
        inner.statuses.insert(xid, TxStatus::Aborted);
        drop(inner);
        self.completion.notify_all();
    }

    pub fn status(&self, xid: TransactionId) -> Option<TxStatus> {
        let inner = self.inner.lock().unwrap();
        inner.statuses.get(&xid).copied()
    }

    /// Blocks the calling transaction waiter until other finishes,
    /// UNLESS doing so would create a cycle in the wait-for graph in
    /// that case, returns Err immediately without ever blocking.
    pub fn wait_for(
        &self,
        waiter: TransactionId,
        other: TransactionId,
    ) -> Result<TxStatus, DeadlockDetected> {
        let mut inner = self.inner.lock().unwrap();

        match inner.statuses.get(&other) {
            Some(TxStatus::InProgress) => {}
            Some(status) => return Ok(*status),
            None => panic!("wait_for called on unknown transaction id {other}"),
        }

        let mut current = other;

        loop {
            if current == waiter {
                return Err(DeadlockDetected);
            }
            match inner.waits_for.get(&current) {
                Some(&next) => current = next,
                None => break,
            }
        }

        inner.waits_for.insert(waiter, other);

        loop {
            match inner.statuses.get(&other) {
                Some(TxStatus::InProgress) => {
                    inner = self.completion.wait(inner).unwrap();
                }
                Some(status) => {
                    let status = *status; // copy out now, so the borrow of `inner` ends here
                    inner.waits_for.remove(&waiter);
                    return Ok(status);
                }
                None => panic!("wait_for called on unknown transaction id {other}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{TransactionManager, TxStatus};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn wait_for_blocks_until_commit() {
        let manager = Arc::new(TransactionManager::new());
        let waiter_xid = manager.begin();
        let other_xid = manager.begin();

        let waiter_manager = Arc::clone(&manager);
        let waiter = thread::spawn(move || waiter_manager.wait_for(waiter_xid, other_xid));

        thread::sleep(Duration::from_millis(50));
        manager.commit(other_xid);

        let result = waiter.join().unwrap();
        assert_eq!(result, Ok(TxStatus::Committed));
    }

    #[test]
    fn wait_for_returns_immediately_if_already_finished() {
        let manager = TransactionManager::new();
        let waiter_xid = manager.begin();
        let other_xid = manager.begin();
        manager.abort(other_xid);

        let result = manager.wait_for(waiter_xid, other_xid);
        assert_eq!(result, Ok(TxStatus::Aborted));
    }
}
