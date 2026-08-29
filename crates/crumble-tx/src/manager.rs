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
}

#[derive(Debug)]
pub struct TransactionManager {
    inner: Mutex<Inner>,
    completion: Condvar,
}
/// a Mutex lock can only fail if a different thread panicked while holding it (a "poisoned" lock) a real, exceptional condition worth crashing loudly on for now, not silently working around.
impl TransactionManager {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                next_id: 1,
                statuses: HashMap::new(),
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

    /// Blocks the calling thread until `xid` finishes (commits or aborts).
    /// Returns the final status.
    pub fn wait_for(&self, xid: TransactionId) -> TxStatus {
        let mut inner = self.inner.lock().unwrap();
        loop {
            match inner.statuses.get(&xid) {
                Some(TxStatus::InProgress) => {
                    inner = self.completion.wait(inner).unwrap();
                }
                Some(status) => return *status,
                None => panic!("wait_for called on unknown transaction id {xid}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn wait_for_blocks_until_commit() {
        let manager = Arc::new(TransactionManager::new());
        let xid = manager.begin();

        let waiter_manager = Arc::clone(&manager);
        let waiter = thread::spawn(move || waiter_manager.wait_for(xid));

        // give the waiting thread time to actually reach wait_for and block —
        // not perfectly deterministic, but good enough to catch a broken wait.
        thread::sleep(Duration::from_millis(50));

        manager.commit(xid);

        let result = waiter.join().unwrap();
        assert_eq!(result, TxStatus::Committed);
    }

    #[test]
    fn wait_for_returns_immediately_if_already_finished() {
        let manager = TransactionManager::new();
        let xid = manager.begin();
        manager.abort(xid);

        // no blocking should happen here at all — status already settled.
        let result = manager.wait_for(xid);
        assert_eq!(result, TxStatus::Aborted);
    }
}
