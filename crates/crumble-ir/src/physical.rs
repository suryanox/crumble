use crate::Literal;
use crate::expr::Expr;

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
}
