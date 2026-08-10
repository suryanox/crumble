use sqlparser::ast::Statement;

#[derive(Debug)]
pub struct Ast {
    pub statements: Vec<Statement>,
}
