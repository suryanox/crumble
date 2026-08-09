use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("failed to parse SQL: {0}")]
    Sql(#[from] sqlparser::parser::ParserError),
}