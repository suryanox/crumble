use crate::expr::Expr;
use crate::{ColumnDef, Literal};

/**
* This tells how to actually run it which strategy
 */
#[derive(Debug, Clone, PartialEq)]
pub enum PhysicalPlan {
    /**
    read every row in the table start to finish, no index, no shortcuts - Simplest way
    */
    SeqScan { table: String },
    Filter {
        input: Box<PhysicalPlan>,
        predicate: Expr,
    },
    Project {
        input: Box<PhysicalPlan>,
        columns: Vec<String>,
    },
    Insert {
        table: String,
        columns: Vec<String>,
        rows: Vec<Vec<Literal>>,
    },
    CreateTable {
        table: String,
        columns: Vec<ColumnDef>,
    },
    Delete {
        table: String,
        predicate: Option<Expr>,
    },
    Update {
        table: String,
        assignments: Vec<(String, Literal)>,
        predicate: Option<Expr>,
    },
    CreateIndex {
        index_name: String,
        table: String,
        column: String,
    },
    IndexScan {
        table: String,
        index_name: String,
        key: Literal,
    },
    RangeIndexScan {
        table: String,
        index_name: String,
        lower: Option<(Literal, bool)>,
        upper: Option<(Literal, bool)>,
    },
}
