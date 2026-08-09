#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Column(String),
    Literal(Literal),
    BinaryOp {
        left: Box<Expr>,
        op: BinaryOperator,
        right: Box<Expr>,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Int(i64),
    Bool(bool),
    String(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum BinaryOperator {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
}
