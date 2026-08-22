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
    /// SQL default: nullable unless NOT NULL was declared.
    pub nullable: bool,
}

impl ColumnDef {
    pub fn matches(&self, value: &crate::value::Value) -> bool {
        if matches!(value, crate::value::Value::Null) {
            return self.nullable;
        }
        self.ty.matches(value)
    }
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

/// deliberately matches real SQL's default (nullable unless NOT NULL), not an arbitrary choice. Every existing test using col(...) keeps working unchanged.
pub fn col(name: &str, ty: ColumnType) -> ColumnDef {
    ColumnDef {
        name: name.to_string(),
        ty,
        nullable: true,
    }
}
