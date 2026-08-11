/**
Intentional choice to not reuse crumble_ir::Literal even though they look similar today.
Reason: Literal is a syntax-level concept, Value is a runtime concept (What's actually stored)
They will diverge. As storage will eventually need Null, fixed width encoding, maybe Bytes.
IR never will
*/
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Bool(bool),
    String(String),
    Float(f64),
}
