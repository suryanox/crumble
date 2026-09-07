use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use std::sync::{Condvar, Mutex};

use serde::{Deserialize, Serialize};

pub type TransactionId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxStatus {
    InProgress,
    Committed,
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeadlockDetected;

#[derive(Debug, Clone, Serialize, Deserialize)]
enum TxLogRecord {
    Begin(TransactionId),
    Commit(TransactionId),
    Abort(TransactionId),
    Snapshot {
        next_id: TransactionId,
        statuses: Vec<(TransactionId, TxStatus)>,
    },
}

fn append_record(file: &mut File, record: &TxLogRecord) -> std::io::Result<()> {
    let bytes = bincode::serde::encode_to_vec(record, bincode::config::standard())
        .expect("TxLogRecord encoding cannot fail");
    let len = bytes.len() as u32;
    file.write_all(&len.to_le_bytes())?;
    file.write_all(&bytes)?;
    file.sync_data()?;
    Ok(())
}

fn read_all_records(path: &Path) -> std::io::Result<Vec<TxLogRecord>> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut records = Vec::new();
    loop {
        let mut len_bytes = [0u8; 4];
        match file.read_exact(&mut len_bytes) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let len = u32::from_le_bytes(len_bytes) as usize;

        let mut payload = vec![0u8; len];
        if file.read_exact(&mut payload).is_err() {
            break; // torn record from a mid-write crash — stop cleanly
        }

        match bincode::serde::decode_from_slice::<TxLogRecord, _>(
            &payload,
            bincode::config::standard(),
        ) {
            Ok((record, _)) => records.push(record),
            Err(_) => break,
        }
    }
    Ok(records)
}

#[derive(Debug)]
struct Inner {
    next_id: TransactionId,
    statuses: HashMap<TransactionId, TxStatus>,
    waits_for: HashMap<TransactionId, TransactionId>,
    log_file: File,
    log_path: std::path::PathBuf,
}

#[derive(Debug)]
pub struct TransactionManager {
    inner: Mutex<Inner>,
    completion: Condvar,
}

impl TransactionManager {
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let records = read_all_records(path)?;

        let mut statuses: HashMap<TransactionId, TxStatus> = HashMap::new();
        let mut max_id: TransactionId = 0;

        for record in &records {
            match record {
                TxLogRecord::Snapshot {
                    next_id,
                    statuses: snap,
                } => {
                    statuses = snap.iter().cloned().collect();
                    max_id = max_id.max(next_id.saturating_sub(1));
                }
                TxLogRecord::Begin(xid) => {
                    statuses.entry(*xid).or_insert(TxStatus::InProgress);
                    max_id = max_id.max(*xid);
                }
                TxLogRecord::Commit(xid) => {
                    statuses.insert(*xid, TxStatus::Committed);
                    max_id = max_id.max(*xid);
                }
                TxLogRecord::Abort(xid) => {
                    statuses.insert(*xid, TxStatus::Aborted);
                    max_id = max_id.max(*xid);
                }
            }
        }

        for status in statuses.values_mut() {
            if *status == TxStatus::InProgress {
                *status = TxStatus::Aborted;
            }
        }

        let log_file = OpenOptions::new().append(true).create(true).open(path)?;

        Ok(Self {
            inner: Mutex::new(Inner {
                next_id: max_id + 1,
                statuses,
                waits_for: HashMap::new(),
                log_file,
                log_path: path.to_path_buf(),
            }),
            completion: Condvar::new(),
        })
    }

    pub fn begin(&self) -> TransactionId {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_id;
        inner.next_id += 1;

        append_record(&mut inner.log_file, &TxLogRecord::Begin(id))
            .expect("transaction log write must succeed");

        inner.statuses.insert(id, TxStatus::InProgress);
        id
    }

    pub fn commit(&self, xid: TransactionId) {
        let mut inner = self.inner.lock().unwrap();
        append_record(&mut inner.log_file, &TxLogRecord::Commit(xid))
            .expect("transaction log write must succeed");
        inner.statuses.insert(xid, TxStatus::Committed);
        drop(inner);
        self.completion.notify_all();
    }

    pub fn abort(&self, xid: TransactionId) {
        let mut inner = self.inner.lock().unwrap();
        append_record(&mut inner.log_file, &TxLogRecord::Abort(xid))
            .expect("transaction log write must succeed");
        inner.statuses.insert(xid, TxStatus::Aborted);
        drop(inner);
        self.completion.notify_all();
    }

    pub fn status(&self, xid: TransactionId) -> Option<TxStatus> {
        let inner = self.inner.lock().unwrap();
        inner.statuses.get(&xid).copied()
    }

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
                    let status = *status;
                    inner.waits_for.remove(&waiter);
                    return Ok(status);
                }
                None => panic!("wait_for called on unknown transaction id {other}"),
            }
        }
    }

    pub fn forget(&self, xid: TransactionId) {
        let mut inner = self.inner.lock().unwrap();
        inner.statuses.remove(&xid);
    }

    pub fn finished_xids(&self) -> Vec<TransactionId> {
        let inner = self.inner.lock().unwrap();
        inner
            .statuses
            .iter()
            .filter(|(_, status)| **status != TxStatus::InProgress)
            .map(|(xid, _)| *xid)
            .collect()
    }

    pub fn has_in_progress(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.statuses.values().any(|s| *s == TxStatus::InProgress)
    }
    pub fn compact(&self) -> std::io::Result<()> {
        let mut inner = self.inner.lock().unwrap();

        let snapshot = TxLogRecord::Snapshot {
            next_id: inner.next_id,
            statuses: inner.statuses.iter().map(|(k, v)| (*k, *v)).collect(),
        };

        let tmp_path = inner.log_path.with_extension("log.tmp");
        let mut tmp_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)?;
        append_record(&mut tmp_file, &snapshot)?;
        drop(tmp_file);

        std::fs::rename(&tmp_path, &inner.log_path)?;

        // the old log_file handle now points at the unlinked, pre-compaction
        // file — reopen a fresh append handle to what's now at this path.
        inner.log_file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&inner.log_path)?;

        Ok(())
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
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(TransactionManager::open(dir.path().join("tx.log")).unwrap());
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
        let dir = tempfile::tempdir().unwrap();
        let manager = TransactionManager::open(dir.path().join("tx.log")).unwrap();
        let waiter_xid = manager.begin();
        let other_xid = manager.begin();
        manager.abort(other_xid);

        let result = manager.wait_for(waiter_xid, other_xid);
        assert_eq!(result, Ok(TxStatus::Aborted));
    }

    #[test]
    fn status_survives_reopening_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("tx.log");

        let (committed_xid, aborted_xid) = {
            let manager = TransactionManager::open(&log_path).unwrap();
            let committed_xid = manager.begin();
            manager.commit(committed_xid);
            let aborted_xid = manager.begin();
            manager.abort(aborted_xid);
            (committed_xid, aborted_xid)
        }; // manager dropped here — simulates the process ending

        let reopened = TransactionManager::open(&log_path).unwrap();
        assert_eq!(reopened.status(committed_xid), Some(TxStatus::Committed));
        assert_eq!(reopened.status(aborted_xid), Some(TxStatus::Aborted));
    }

    #[test]
    fn transaction_still_in_progress_at_crash_becomes_aborted_on_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("tx.log");

        let never_finished_xid = {
            let manager = TransactionManager::open(&log_path).unwrap();
            manager.begin() // never committed or aborted — simulates a crash mid-transaction
        };

        let reopened = TransactionManager::open(&log_path).unwrap();
        assert_eq!(
            reopened.status(never_finished_xid),
            Some(TxStatus::Aborted),
            "a transaction with no commit/abort record after reopening must be treated as aborted"
        );
    }

    #[test]
    fn next_id_resumes_past_previously_used_ids() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("tx.log");

        let highest_seen = {
            let manager = TransactionManager::open(&log_path).unwrap();
            let a = manager.begin();
            let b = manager.begin();
            manager.commit(a);
            manager.commit(b);
            b
        };

        let reopened = TransactionManager::open(&log_path).unwrap();
        let new_xid = reopened.begin();
        assert!(
            new_xid > highest_seen,
            "new transaction ids must never reuse or collide with old ones"
        );
    }
}
