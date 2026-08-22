#[derive(Debug, Clone, PartialEq)]
pub enum ColumnType {
    Int,
    Bool,
    Text,
    Float,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub name: String,
    pub typ: ColumnType,
}
