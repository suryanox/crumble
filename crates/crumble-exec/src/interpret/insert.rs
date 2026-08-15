use crate::interpret::eval::literal_to_value;
use crate::interpret::order::order_row_values;
use crate::{ExecError, RowSet};
use crumble_ir::Literal;
use crumble_storage::{Catalog, Row, Value};

pub(super) fn insert(
    catalog: &mut Catalog,
    table: &String,
    columns: &Vec<String>,
    rows: &Vec<Vec<Literal>>,
) -> Result<RowSet, ExecError> {
    let target = catalog.get_mut(table)?;
    let table_columns = target.columns().to_vec();

    for row in rows {
        let values: Vec<Value> = row.iter().map(literal_to_value).collect();
        let ordered = order_row_values(&table_columns, columns, values)?;
        target.insert(Row::new(ordered))?;
    }

    Ok(RowSet::new(Vec::new(), Vec::new()))
}
