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
        columns: Projection,
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
    Join {
        left: Box<LogicalPlan>,
        right: Box<LogicalPlan>,
        left_table: String,
        right_table: String,
        on: Expr,
        kind: JoinKind,
    },
    DropTable {
        table: String,
        if_exists: bool,
    },
    DropIndex {
        index_name: String,
        if_exists: bool,
    },
    Aggregate {
        input: Box<LogicalPlan>,
        group_by: Vec<String>,
        aggregates: Vec<AggregateExpr>,
    },
    VacuumTable {
        table: String,
    },
    VacuumAll,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JoinKind {
    Inner,
    Left,
    Right,
    FullOuter,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Projection {
    All,
    Columns(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum AggFunc {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AggregateExpr {
    pub func: AggFunc,
    /// None only valid for Count — that's COUNT(*), counting rows not values.
    pub column: Option<String>,
    /// output column name, e.g. "COUNT(*)" or an explicit AS alias.
    pub alias: String,
}
