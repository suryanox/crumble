use crate::{LogicalPlan, LowerError};
use sqlparser::ast::CreateIndex;

pub(super) fn lower_create_index(create_index: &CreateIndex) -> Result<LogicalPlan, LowerError> {
    let index_name = create_index
        .name
        .as_ref()
        .map(|n| n.to_string())
        .ok_or_else(|| LowerError::Unsupported("CREATE INDEX requires a name".to_string()))?;

    let table = create_index.table_name.to_string();

    let column = create_index
        .columns
        .first()
        .ok_or_else(|| LowerError::Unsupported("CREATE INDEX requires a column".to_string()))?
        .to_string();

    if create_index.columns.len() > 1 {
        return Err(LowerError::Unsupported(
            "multi-column indexes not supported yet".to_string(),
        ));
    }

    Ok(LogicalPlan::CreateIndex {
        index_name,
        table,
        column,
    })
}
