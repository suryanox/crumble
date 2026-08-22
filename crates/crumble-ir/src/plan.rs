use crate::Literal;
use crate::column::ColumnDef;
use crate::expr::Expr;

/**
* This tells what the query means
 */
#[derive(Debug, Clone, PartialEq)]
pub enum LogicalPlan {
    Scan {
        table: String,
    },
    Filter {
        input: Box<LogicalPlan>,
        predicate: Expr,
    },
    Project {
        input: Box<LogicalPlan>,
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
}
