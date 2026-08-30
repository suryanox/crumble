use crate::Ast;
use sqlparser::ast::Statement;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionControl {
    Begin,
    Commit,
    Rollback,
}

pub fn transaction_control(ast: &Ast) -> Option<TransactionControl> {
    match ast.statements.first()? {
        Statement::StartTransaction { .. } => Some(TransactionControl::Begin),
        Statement::Commit { .. } => Some(TransactionControl::Commit),
        Statement::Rollback { .. } => Some(TransactionControl::Rollback),
        _ => None,
    }
}
