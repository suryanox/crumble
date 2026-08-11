use crate::Literal;
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
}
