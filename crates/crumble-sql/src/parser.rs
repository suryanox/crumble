use crate::ParseError;
use crate::ast::Ast;
use sqlparser::{dialect::GenericDialect, parser::Parser};

pub fn parse(sql: &str) -> Result<Ast, ParseError> {
    let dialect = GenericDialect {};
    let statements = Parser::parse_sql(&dialect, sql)?;
    Ok(Ast { statements })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_select() -> Result<(), ParseError> {
        let ast = parse("SELECT 1")?;

        assert_eq!(ast.statements.len(), 1);

        Ok(())
    }
}
