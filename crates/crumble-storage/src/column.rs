use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ColumnType {
    Int,
    Bool,
    String,
    Float,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnDef {
    pub name: String,
    pub ty: ColumnType,
}

impl ColumnType {
    pub fn matches(&self, value: &crate::value::Value) -> bool {
        matches!(
            (self, value),
            (ColumnType::Int, crate::value::Value::Int(_))
                | (ColumnType::Bool, crate::value::Value::Bool(_))
                | (ColumnType::String, crate::value::Value::String(_))
                | (ColumnType::Float, crate::value::Value::Float(_))
        )
    }
}

pub fn col(name: &str, ty: ColumnType) -> ColumnDef {
    ColumnDef {
        name: name.to_string(),
        ty,
    }
}
