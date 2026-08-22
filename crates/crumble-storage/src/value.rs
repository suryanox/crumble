use serde::{Deserialize, Serialize};

/**
Intentional choice to not reuse crumble_ir::Literal even though they look similar today.
Reason: Literal is a syntax-level concept, Value is a runtime concept (What's actually stored)
They will diverge. As storage will eventually need Null, fixed width encoding, maybe Bytes.
IR never will
*/
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Int(i64),
    Bool(bool),
    String(String),
    Float(f64),
    Null,
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::String(s) => write!(f, "{s}"),
            Value::Float(flo) => write!(f, "{flo}"),
            Value::Null => write!(f, "NULL"),
        }
    }
}
