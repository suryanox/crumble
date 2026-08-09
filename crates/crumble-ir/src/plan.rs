use crate::expr::Expr;

#[derive(Debug, Clone, PartialEq)]
pub enum LogicalPlan {
    Scan {
        table: String,
    },
    Filter {
        input: Box<LogicalPlan>,
        predicate: Expr
    },
    Project {
        input: Box<LogicalPlan>,
        columns: Vec<String>,
    }
}