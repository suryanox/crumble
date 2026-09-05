use crate::interpret::aggregate::aggregate;
use crate::interpret::create::{create, create_index, vacuum_table};
use crate::interpret::delete::delete;
use crate::interpret::drop_stmt::{drop_index, drop_table};
use crate::interpret::filter::filter;
use crate::interpret::indexnestedloopjoin::indexnestedloopjoin;
use crate::interpret::indexscan::{indexscan, rangeindexscan};
use crate::interpret::insert::insert;
use crate::interpret::nestedloopjoin::nestedloopjoin;
use crate::interpret::project::project;
use crate::interpret::seqscan::seqscan;
use crate::interpret::update::update;
use crate::{ExecError, RowSet};
use crumble_ir::PhysicalPlan;
use crumble_storage::Catalog;
use crumble_tx::TransactionId;

mod eval;
mod filter;
mod order;
mod project;
mod seqscan;

mod aggregate;
mod create;
mod delete;
mod drop_stmt;
mod indexnestedloopjoin;
mod indexscan;
mod insert;
mod nestedloopjoin;
mod update;

pub fn execute(
    plan: &PhysicalPlan,
    catalog: &Catalog,
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    match plan {
        PhysicalPlan::SeqScan { table } => seqscan(catalog, table, xid),
        PhysicalPlan::Filter { input, predicate } => filter(catalog, input, predicate, xid),
        PhysicalPlan::Project { input, columns } => project(catalog, input, columns, xid),
        PhysicalPlan::Insert {
            table,
            columns,
            rows,
        } => insert(catalog, table, columns, rows, xid),
        PhysicalPlan::CreateTable { table, columns } => create(catalog, table, columns),
        PhysicalPlan::Delete { table, predicate } => delete(catalog, table, predicate, xid),
        PhysicalPlan::Update {
            table,
            assignments,
            predicate,
        } => update(catalog, table, assignments, predicate, xid),
        PhysicalPlan::CreateIndex {
            index_name,
            table,
            column,
        } => create_index(catalog, index_name, table, column),
        PhysicalPlan::IndexScan {
            table,
            index_name,
            key,
        } => indexscan(catalog, table, index_name, key, xid),
        PhysicalPlan::RangeIndexScan {
            table,
            index_name,
            lower,
            upper,
        } => rangeindexscan(catalog, table, index_name, lower, upper, xid),
        PhysicalPlan::NestedLoopJoin {
            left,
            right,
            left_table,
            right_table,
            on,
            kind,
        } => nestedloopjoin(catalog, left, right, left_table, right_table, on, kind, xid),
        PhysicalPlan::IndexNestedLoopJoin {
            left,
            left_table,
            right_table_real,
            right_table_qualifier,
            right_index_name,
            left_join_column,
            kind,
        } => indexnestedloopjoin(
            catalog,
            left,
            left_table,
            right_table_real,
            right_table_qualifier,
            right_index_name,
            left_join_column,
            kind,
            xid,
        ),
        PhysicalPlan::DropTable { table, if_exists } => drop_table(catalog, table, *if_exists),
        PhysicalPlan::DropIndex {
            index_name,
            if_exists,
        } => drop_index(catalog, index_name, *if_exists),
        PhysicalPlan::Aggregate {
            input,
            group_by,
            aggregates,
        } => aggregate(catalog, input, group_by, aggregates, xid),
        PhysicalPlan::VacuumTable { table } => vacuum_table(catalog, table),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crumble_ir::{lower, to_physical};
    use crumble_sql::parse;
    use crumble_storage::{Catalog, ColumnType, Row, Value, col};
    use crumble_tx::TransactionManager;
    use std::sync::Arc;

    fn seeded_catalog() -> (tempfile::TempDir, Catalog, u64) {
        let dir = tempfile::tempdir().unwrap();
        let tx_manager = Arc::new(TransactionManager::new());
        let catalog = Catalog::open(dir.path(), Arc::clone(&tx_manager)).unwrap();
        let xid = tx_manager.begin();

        catalog
            .create_table(
                "users",
                vec![col("name", ColumnType::String), col("age", ColumnType::Int)],
            )
            .unwrap();
        let users_handle = catalog.table("users").unwrap();
        let mut users = users_handle.lock().unwrap();
        users
            .insert(
                Row::new(vec![Value::String("alice".to_string()), Value::Int(35)]),
                xid,
            )
            .unwrap();
        users
            .insert(
                Row::new(vec![Value::String("bob".to_string()), Value::Int(22)]),
                xid,
            )
            .unwrap();
        drop(users);

        catalog
            .create_table(
                "metrics",
                vec![
                    col("label", ColumnType::String),
                    col("score", ColumnType::Float),
                ],
            )
            .unwrap();
        let metrics_handle = catalog.table("metrics").unwrap();
        let mut metrics = metrics_handle.lock().unwrap();
        metrics
            .insert(
                Row::new(vec![Value::String("a".to_string()), Value::Float(4.0)]),
                xid,
            )
            .unwrap();
        drop(metrics);

        (dir, catalog, xid)
    }

    #[test]
    fn executes_filtered_projection() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, catalog, xid) = seeded_catalog();

        let ast = parse("SELECT name FROM users WHERE age > 30")?;
        let logical = lower(&ast)?;
        let physical = to_physical(logical);

        let result = execute(&physical, &catalog, xid)?;

        assert_eq!(result.columns(), &["name".to_string()]);
        assert_eq!(
            result.rows(),
            &[Row::new(vec![Value::String("alice".to_string())])]
        );
        Ok(())
    }

    #[test]
    fn errors_on_unknown_column() {
        let (_dir, catalog, xid) = seeded_catalog();

        let ast = parse("SELECT ghost FROM users").unwrap();
        let logical = lower(&ast).unwrap();
        let physical = to_physical(logical);

        let result = execute(&physical, &catalog, xid);

        assert!(matches!(result, Err(ExecError::ColumnNotFound(col)) if col == "ghost"));
    }

    #[test]
    fn inserts_then_reads_back() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, catalog, xid) = seeded_catalog();

        let insert_ast = parse("INSERT INTO users (name, age) VALUES ('eve', 41)")?;
        let insert_physical = to_physical(lower(&insert_ast)?);
        execute(&insert_physical, &catalog, xid)?;

        let select_ast = parse("SELECT name FROM users WHERE age > 40")?;
        let select_physical = to_physical(lower(&select_ast)?);
        let result = execute(&select_physical, &catalog, xid)?;

        assert_eq!(
            result.rows(),
            &[Row::new(vec![Value::String("eve".to_string())])]
        );
        Ok(())
    }

    #[test]
    fn filters_float_values() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, catalog, xid) = seeded_catalog();

        let ast = parse("SELECT label FROM metrics WHERE score > 3.0")?;
        let physical = to_physical(lower(&ast)?);
        let result = execute(&physical, &catalog, xid)?;

        assert_eq!(result.rows(), &[Row::new(vec![Value::String("a".into())])]);
        Ok(())
    }

    #[test]
    fn executes_int_addition() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, catalog, xid) = seeded_catalog();
        let ast = parse("SELECT name FROM users WHERE age > 20 + 1")?;
        let physical = to_physical(lower(&ast)?);
        let result = execute(&physical, &catalog, xid)?;

        assert_eq!(result.rows().len(), 2);
        Ok(())
    }

    fn seeded_catalog_with_orders() -> (tempfile::TempDir, Catalog, u64) {
        let dir = tempfile::tempdir().unwrap();
        let tx_manager = Arc::new(TransactionManager::new());
        let catalog = Catalog::open(dir.path(), Arc::clone(&tx_manager)).unwrap();
        let xid = tx_manager.begin();

        catalog
            .create_table(
                "users",
                vec![col("id", ColumnType::Int), col("name", ColumnType::String)],
            )
            .unwrap();
        let users_handle = catalog.table("users").unwrap();
        let mut users = users_handle.lock().unwrap();
        users
            .insert(
                Row::new(vec![Value::Int(1), Value::String("alice".to_string())]),
                xid,
            )
            .unwrap();
        users
            .insert(
                Row::new(vec![Value::Int(2), Value::String("bob".to_string())]),
                xid,
            )
            .unwrap();
        users
            .insert(
                Row::new(vec![Value::Int(3), Value::String("carol".to_string())]),
                xid,
            )
            .unwrap();
        drop(users);

        catalog
            .create_table(
                "orders",
                vec![
                    col("id", ColumnType::Int),
                    col("user_id", ColumnType::Int),
                    col("total", ColumnType::Int),
                ],
            )
            .unwrap();
        let orders_handle = catalog.table("orders").unwrap();
        let mut orders = orders_handle.lock().unwrap();
        orders
            .insert(
                Row::new(vec![Value::Int(100), Value::Int(1), Value::Int(50)]),
                xid,
            )
            .unwrap();
        orders
            .insert(
                Row::new(vec![Value::Int(101), Value::Int(1), Value::Int(30)]),
                xid,
            )
            .unwrap();
        orders
            .insert(
                Row::new(vec![Value::Int(102), Value::Int(2), Value::Int(20)]),
                xid,
            )
            .unwrap();
        orders
            .insert(
                Row::new(vec![Value::Int(103), Value::Int(99), Value::Int(500)]),
                xid,
            )
            .unwrap();
        drop(orders);
        // note: carol (id 3) has no orders; order 103 (user_id 99) has no matching user —
        // both are deliberate, exercising the "unmatched" side of LEFT/RIGHT joins.

        (dir, catalog, xid)
    }

    fn run(sql: &str, catalog: &Catalog, xid: u64) -> Result<RowSet, Box<dyn std::error::Error>> {
        let ast = parse(sql)?;
        let physical = to_physical(lower(&ast)?);
        Ok(execute(&physical, catalog, xid)?)
    }

    #[test]
    fn inner_join_returns_only_matched_pairs() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, catalog, xid) = seeded_catalog_with_orders();

        let result = run(
            "SELECT users.name, orders.total FROM users JOIN orders ON users.id = orders.user_id",
            &catalog,
            xid,
        )?;

        assert_eq!(
            result.columns(),
            &["users.name".to_string(), "orders.total".to_string()]
        );
        assert_eq!(
            result.rows().len(),
            3,
            "carol (no orders) and the orphaned order must both be excluded"
        );

        let names: Vec<&Value> = result.rows().iter().map(|r| &r.values()[0]).collect();
        assert!(
            !names.contains(&&Value::String("carol".to_string())),
            "carol has no orders, should not appear"
        );

        Ok(())
    }

    #[test]
    fn left_join_pads_unmatched_left_row_with_null() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, catalog, xid) = seeded_catalog_with_orders();

        let result = run(
            "SELECT users.name, orders.total FROM users LEFT JOIN orders ON users.id = orders.user_id",
            &catalog,
            xid,
        )?;

        assert_eq!(
            result.rows().len(),
            4,
            "3 matched orders + 1 unmatched user (carol)"
        );

        let carol_row = result
            .rows()
            .iter()
            .find(|r| r.values()[0] == Value::String("carol".to_string()))
            .expect("carol must appear, padded with NULL");

        assert_eq!(carol_row.values()[1], Value::Null);
        Ok(())
    }

    #[test]
    fn right_join_pads_unmatched_right_row_with_null() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, catalog, xid) = seeded_catalog_with_orders();

        let result = run(
            "SELECT users.name, orders.total FROM users RIGHT JOIN orders ON users.id = orders.user_id",
            &catalog,
            xid,
        )?;

        assert_eq!(
            result.rows().len(),
            4,
            "3 matched orders + 1 unmatched order (user_id 99)"
        );

        let orphan_row = result
            .rows()
            .iter()
            .find(|r| r.values()[1] == Value::Int(500))
            .expect("the orphaned order (total=500) must appear, padded with NULL");

        assert_eq!(orphan_row.values()[0], Value::Null);
        Ok(())
    }

    #[test]
    fn where_filters_correctly_on_top_of_a_join() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, catalog, xid) = seeded_catalog_with_orders();

        let result = run(
            "SELECT users.name FROM users JOIN orders ON users.id = orders.user_id WHERE orders.total > 25",
            &catalog,
            xid,
        )?;

        assert_eq!(result.rows().len(), 2);
        for row in result.rows() {
            assert_eq!(row.values()[0], Value::String("alice".to_string()));
        }
        Ok(())
    }

    #[test]
    fn right_join_does_not_duplicate_matched_rows() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, catalog, xid) = seeded_catalog_with_orders();

        let result = run(
            "SELECT users.name, orders.total FROM users RIGHT JOIN orders ON users.id = orders.user_id",
            &catalog,
            xid,
        )?;

        let alice_count = result
            .rows()
            .iter()
            .filter(|r| r.values()[0] == Value::String("alice".to_string()))
            .count();
        assert_eq!(
            alice_count, 2,
            "alice's two real orders must each appear exactly once, no more"
        );
        Ok(())
    }

    #[test]
    fn full_outer_join_pads_both_unmatched_sides() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, catalog, xid) = seeded_catalog_with_orders();

        let result = run(
            "SELECT users.name, orders.total FROM users FULL JOIN orders ON users.id = orders.user_id",
            &catalog,
            xid,
        )?;

        assert_eq!(
            result.rows().len(),
            5,
            "3 matched + carol (unmatched left) + order 103 (unmatched right)"
        );

        let carol_row = result
            .rows()
            .iter()
            .find(|r| r.values()[0] == Value::String("carol".to_string()))
            .expect("carol must appear, padded with NULL on the right");
        assert_eq!(carol_row.values()[1], Value::Null);

        let orphan_row = result
            .rows()
            .iter()
            .find(|r| r.values()[1] == Value::Int(500))
            .expect("the orphaned order must appear, padded with NULL on the left");
        assert_eq!(orphan_row.values()[0], Value::Null);

        let alice_count = result
            .rows()
            .iter()
            .filter(|r| r.values()[0] == Value::String("alice".to_string()))
            .count();
        assert_eq!(
            alice_count, 2,
            "matched rows must not be duplicated by either padding pass"
        );

        Ok(())
    }
}
