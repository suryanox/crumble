#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Column(String),
    Literal(Literal),
    BinaryOp {
        left: Box<Expr>,
        op: BinaryOperator,
        right: Box<Expr>,
    },
    IsNull {
        expr: Box<Expr>,
        negated: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Int(i64),
    Bool(bool),
    String(String),
    Float(f64),
    Null,
}

#[derive(Clone, Debug, PartialEq, Copy)]
pub enum BinaryOperator {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
    Add,
}
