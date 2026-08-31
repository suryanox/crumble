use sqlparser::ast::{ObjectName, ObjectType};

use crate::{LogicalPlan, LowerError};

pub(super) fn lower_drop(
    object_type: &ObjectType,
    if_exists: bool,
    names: &[ObjectName],
) -> Result<LogicalPlan, LowerError> {
    let name = names
        .first()
        .ok_or_else(|| LowerError::Unsupported("DROP requires a name".to_string()))?
        .to_string();

    if names.len() > 1 {
        return Err(LowerError::Unsupported(
            "dropping multiple objects at once not supported yet".to_string(),
        ));
    }

    match object_type {
        ObjectType::Table => Ok(LogicalPlan::DropTable {
            table: name,
            if_exists,
        }),
        ObjectType::Index => Ok(LogicalPlan::DropIndex {
            index_name: name,
            if_exists,
        }),
        other => Err(LowerError::Unsupported(format!("DROP target: {other:?}"))),
    }
}
