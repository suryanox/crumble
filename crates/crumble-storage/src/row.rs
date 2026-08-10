use crate::value::Value;

/**
NewType instead of values: Vec<Value> as for MVCC and WAL. will need rowID and version/visibility
metadata.
*/
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    values: Vec<Value>,
}

impl Row {
    pub fn new(values: Vec<Value>) -> Self {
        Self { values }
    }

    pub fn values(&self) -> &[Value] {
        &self.values
    }
}
