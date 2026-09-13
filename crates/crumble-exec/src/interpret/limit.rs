use crumble_ir::PhysicalPlan;
use crumble_storage::Catalog;
use crumble_tx::TransactionId;

use crate::error::ExecError;
use crate::execute;
use crate::row_set::RowSet;

pub(super) fn limit(
    catalog: &Catalog,
    input: &Box<PhysicalPlan>,
    limit: &Option<u64>,
    offset: &Option<u64>,
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    let input_result = execute(input, catalog, xid)?;

    let start = offset.unwrap_or(0) as usize;
    let rows: Vec<_> = match limit {
        Some(n) => input_result
            .rows()
            .iter()
            .skip(start)
            .take(*n as usize)
            .cloned()
            .collect(),
        None => input_result.rows().iter().skip(start).cloned().collect(),
    };

    Ok(RowSet::new(input_result.columns().to_vec(), rows))
}
