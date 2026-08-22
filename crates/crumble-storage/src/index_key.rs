use crate::value::Value;
use crumble_index::IndexKey;

/// Only Int/String values can be indexed — matches crumble-index's own
/// key scoping (Float excluded for NaN/Ord reasons, Bool too low-cardinality
/// to bother with).
pub fn value_to_index_key(value: &Value) -> Option<IndexKey> {
    match value {
        Value::Int(n) => Some(IndexKey::Int(*n)),
        Value::String(s) => Some(IndexKey::String(s.clone())),
        Value::Null => Some(IndexKey::Null),
        _ => None,
    }
}
