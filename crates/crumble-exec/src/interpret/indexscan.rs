use crumble_ir::Literal;
use crumble_storage::{Catalog, Row};
use crumble_tx::TransactionId;

use crate::error::ExecError;
use crate::row_set::RowSet;

pub(super) fn indexscan(
    catalog: &Catalog,
    table: &str,
    index_name: &str,
    key: &Literal,
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    let index_key = literal_to_index_key(key).ok_or(ExecError::TypeMismatch)?;

    let index_handle = catalog.index(index_name)?;
    let locations = index_handle.lock().unwrap().search(&index_key)?;

    let target_handle = catalog.table(table)?;
    let mut target = target_handle.lock().unwrap();
    let columns: Vec<String> = target.columns().iter().map(|c| c.name.clone()).collect();
    let mut rows: Vec<Row> = Vec::new();
    for (page_index, slot) in locations {
        if let Some(row) = target.row_at(page_index, slot, xid)? {
            rows.push(row);
        }
    }

    Ok(RowSet::new(columns, rows))
}

pub(super) fn rangeindexscan(
    catalog: &Catalog,
    table: &str,
    index_name: &str,
    lower: &Option<(Literal, bool)>,
    upper: &Option<(Literal, bool)>,
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    let lower_key = match lower {
        Some((lit, inc)) => {
            let key = literal_to_index_key(lit).ok_or(ExecError::TypeMismatch)?;
            Some((key, *inc))
        }
        None => None,
    };
    let upper_key = match upper {
        Some((lit, inc)) => {
            let key = literal_to_index_key(lit).ok_or(ExecError::TypeMismatch)?;
            Some((key, *inc))
        }
        None => None,
    };

    let index_handle = catalog.index(index_name)?;
    let locations = index_handle.lock().unwrap().range_search(
        lower_key.as_ref().map(|(k, inc)| (k, *inc)),
        upper_key.as_ref().map(|(k, inc)| (k, *inc)),
    )?;

    let target_handle = catalog.table(table)?;
    let mut target = target_handle.lock().unwrap();
    let columns: Vec<String> = target.columns().iter().map(|c| c.name.clone()).collect();
    let mut rows = Vec::new();
    for (page_index, slot) in locations {
        if let Some(row) = target.row_at(page_index, slot, xid)? {
            rows.push(row);
        }
    }

    Ok(RowSet::new(columns, rows))
}

fn literal_to_index_key(literal: &Literal) -> Option<crumble_index::IndexKey> {
    match literal {
        Literal::Int(n) => Some(crumble_index::IndexKey::Int(*n)),
        Literal::String(s) => Some(crumble_index::IndexKey::String(s.clone())),
        Literal::Null => Some(crumble_index::IndexKey::Null),
        Literal::Bool(_) | Literal::Float(_) => None,
    }
}
