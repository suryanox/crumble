use crate::interpret::eval::literal_to_value;
use crate::interpret::order::order_row_values;
use crate::{ExecError, RowSet};
use crumble_ir::Literal;
use crumble_storage::{Catalog, Row, Value, value_to_index_key};
use crumble_tx::TransactionId;

pub(super) fn insert(
    catalog: &Catalog,
    table: &str,
    columns: &[String],
    rows: &[Vec<Literal>],
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    let target_handle = catalog.table(table)?;
    let mut target = target_handle.lock().unwrap();
    let table_columns: Vec<String> = target.columns().iter().map(|c| c.name.clone()).collect();
    let mut inserted = Vec::new();
    for row in rows {
        let values: Vec<Value> = row.iter().map(literal_to_value).collect();
        let ordered = order_row_values(&table_columns, columns, values)?;
        let (page_index, slot) = target.insert(Row::new(ordered.clone()), xid)?;
        inserted.push((page_index, slot, ordered));
    }
    drop(target); // release the table lock before touching index locks below

    let indexed_columns: Vec<(usize, String)> = table_columns
        .iter()
        .enumerate()
        .filter_map(|(i, c)| catalog.index_for(table, c).map(|name| (i, name)))
        .collect();

    for (page_index, slot, values) in inserted {
        for (col_pos, index_name) in &indexed_columns {
            if let Some(key) = value_to_index_key(&values[*col_pos]) {
                let index_handle = catalog.index(index_name)?;
                index_handle.lock().unwrap().insert(key, page_index, slot)?;
            }
        }
    }

    Ok(RowSet::new(Vec::new(), Vec::new()))
}
