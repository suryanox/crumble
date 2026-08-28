use crate::manager::{TransactionId, TransactionManager, TxStatus};

/// Row versioning fields, decoupled from crumble_storage::Row itself —
/// crumble-tx shouldn't depend on crumble-storage, same layering discipline
/// as everywhere else (crumble-wal not depending on crumble-storage, etc).
/// crumble-storage will call this with its own Row's xmin/xmax fields.
pub fn is_visible(
    xmin: TransactionId,
    xmax: Option<TransactionId>,
    reader: TransactionId,
    manager: &TransactionManager,
) -> bool {
    let xmin_visible = if xmin == 0 {
        // 0 is the "pre-transactional" placeholder from before BEGIN/COMMIT
        // wiring exists treated as always-committed, same idea as
        // Postgres's frozen xids for rows old enough that visibility is
        // no longer even in question.
        true
    } else if xmin == reader {
        true // reading your own not-yet-committed write
    } else {
        matches!(manager.status(xmin), Some(TxStatus::Committed))
    };

    if !xmin_visible {
        return false;
    }

    match xmax {
        None => true,
        Some(xmax) if xmax == reader => false, // this transaction deleted it itself
        Some(xmax) => match manager.status(xmax) {
            Some(TxStatus::Committed) => false, // really deleted, gone
            Some(TxStatus::InProgress) => true, // not committed yet. read committed still sees it
            Some(TxStatus::Aborted) => true,    // the delete never actually happened
            None => true, // unknown xmax. conservatively still visible, not silently hidden
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::TransactionManager;

    #[test]
    fn legacy_zero_xmin_always_visible() {
        let manager = TransactionManager::new();
        assert!(is_visible(0, None, 999, &manager));
    }

    #[test]
    fn own_uncommitted_insert_is_visible_to_self() {
        let manager = TransactionManager::new();
        let xid = manager.begin();
        assert!(is_visible(xid, None, xid, &manager));
    }

    #[test]
    fn other_uncommitted_insert_is_invisible() {
        let manager = TransactionManager::new();
        let writer = manager.begin();
        let reader = manager.begin();
        assert!(!is_visible(writer, None, reader, &manager));
    }

    #[test]
    fn committed_insert_is_visible_to_everyone() {
        let manager = TransactionManager::new();
        let writer = manager.begin();
        manager.commit(writer);
        let reader = manager.begin();
        assert!(is_visible(writer, None, reader, &manager));
    }

    #[test]
    fn own_uncommitted_delete_hides_row_from_self() {
        let manager = TransactionManager::new();
        let xid = manager.begin();
        assert!(!is_visible(0, Some(xid), xid, &manager));
    }

    #[test]
    fn other_uncommitted_delete_does_not_hide_row_read_committed() {
        let manager = TransactionManager::new();
        let deleter = manager.begin();
        let reader = manager.begin();
        assert!(is_visible(0, Some(deleter), reader, &manager));
    }

    #[test]
    fn committed_delete_hides_row_from_everyone() {
        let manager = TransactionManager::new();
        let deleter = manager.begin();
        manager.commit(deleter);
        let reader = manager.begin();
        assert!(!is_visible(0, Some(deleter), reader, &manager));
    }

    #[test]
    fn aborted_delete_leaves_row_visible() {
        let manager = TransactionManager::new();
        let deleter = manager.begin();
        manager.abort(deleter);
        let reader = manager.begin();
        assert!(is_visible(0, Some(deleter), reader, &manager));
    }
}
