use crate::interpret::create::create;
use crate::interpret::delete::delete;
use crate::interpret::filter::filter;
use crate::interpret::insert::insert;
use crate::interpret::project::project;
use crate::interpret::seqscan::seqscan;
use crate::interpret::update::update;
use crate::{ExecError, RowSet};
use crumble_ir::PhysicalPlan;
use crumble_storage::Catalog;

mod eval;
mod filter;
mod order;
mod project;
mod seqscan;

mod create;
mod delete;
mod insert;
mod update;
pub fn execute(plan: &PhysicalPlan, catalog: &mut Catalog) -> Result<RowSet, ExecError> {
    match plan {
        PhysicalPlan::SeqScan { table } => seqscan(catalog, table),
        PhysicalPlan::Filter { input, predicate } => filter(catalog, input, predicate),
        PhysicalPlan::Project { input, columns } => project(catalog, input, columns),
        PhysicalPlan::Insert {
            table,
            columns,
            rows,
        } => insert(catalog, table, columns, rows),
        PhysicalPlan::CreateTable { table, columns } => create(catalog, table, columns),
        PhysicalPlan::Delete { table, predicate } => delete(catalog, table, predicate),
        PhysicalPlan::Update {
            table,
            assignments,
            predicate,
        } => update(catalog, table, assignments, predicate),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crumble_ir::{lower, to_physical};
    use crumble_sql::parse;
    use crumble_storage::{Catalog, Row, Value};

    fn seeded_catalog() -> (tempfile::TempDir, Catalog) {
        let dir = tempfile::tempdir().unwrap();
        let mut catalog = Catalog::open(dir.path()).unwrap();
        catalog
            .create_table("users", vec!["name".to_string(), "age".to_string()])
            .unwrap();

        let users = catalog.get_mut("users").unwrap();
        users
            .insert(Row::new(vec![
                Value::String("alice".to_string()),
                Value::Int(35),
            ]))
            .unwrap();
        users
            .insert(Row::new(vec![
                Value::String("bob".to_string()),
                Value::Int(22),
            ]))
            .unwrap();

        catalog
            .create_table("metrics", vec!["label".to_string(), "score".to_string()])
            .unwrap();

        let metrics = catalog.get_mut("metrics").unwrap();
        metrics
            .insert(Row::new(vec![
                Value::String("a".to_string()),
                Value::Float(4.0),
            ]))
            .unwrap();

        (dir, catalog)
    }

    #[test]
    fn executes_filtered_projection() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, mut catalog) = seeded_catalog();

        let ast = parse("SELECT name FROM users WHERE age > 30")?;
        let logical = lower(&ast)?;
        let physical = to_physical(logical);

        let result = execute(&physical, &mut catalog)?;

        assert_eq!(result.columns(), &["name".to_string()]);
        assert_eq!(
            result.rows(),
            &[Row::new(vec![Value::String("alice".to_string())])]
        );
        Ok(())
    }

    #[test]
    fn errors_on_unknown_column() {
        let (_dir, mut catalog) = seeded_catalog();

        let ast = parse("SELECT ghost FROM users").unwrap();
        let logical = lower(&ast).unwrap();
        let physical = to_physical(logical);

        let result = execute(&physical, &mut catalog);

        assert!(matches!(result, Err(ExecError::ColumnNotFound(col)) if col == "ghost"));
    }

    #[test]
    fn inserts_then_reads_back() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, mut catalog) = seeded_catalog();

        let insert_ast = parse("INSERT INTO users (name, age) VALUES ('eve', 41)")?;
        let insert_physical = to_physical(lower(&insert_ast)?);
        execute(&insert_physical, &mut catalog)?;

        let select_ast = parse("SELECT name FROM users WHERE age > 40")?;
        let select_physical = to_physical(lower(&select_ast)?);
        let result = execute(&select_physical, &mut catalog)?;

        assert_eq!(
            result.rows(),
            &[Row::new(vec![Value::String("eve".to_string())])]
        );
        Ok(())
    }

    #[test]
    fn filters_float_values() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, mut catalog) = seeded_catalog();

        let ast = parse("SELECT label FROM metrics WHERE score > 3.0")?;
        let logical = lower(&ast)?;
        let physical = to_physical(logical);
        let result = execute(&physical, &mut catalog)?;

        assert_eq!(result.rows(), &[Row::new(vec![Value::String("a".into())])]);
        Ok(())
    }

    #[test]
    fn executes_int_addition() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, mut catalog) = seeded_catalog();
        let ast = parse("SELECT name FROM users WHERE age > 20 + 1")?;
        let logical = lower(&ast)?;
        let physical = to_physical(logical);
        let result = execute(&physical, &mut catalog)?;

        assert_eq!(result.rows().len(), 2);
        Ok(())
    }
}
