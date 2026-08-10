use std::env;
use std::process::ExitCode;

use crumble_exec::execute;
use crumble_ir::{lower, to_physical};
use crumble_opt::{ConstantFold, OptimizationPass};
use crumble_storage::{Catalog, Row, Value};
use crumble_sql::parse;

fn seeded_catalog() -> Catalog {
    let mut catalog = Catalog::new();
    catalog
        .create_table("users", vec!["name".to_string(), "age".to_string()])
        .expect("seed table should not already exist");

    let users = catalog
        .get_mut("users")
        .expect("just-created table must exist");
    users
        .insert(Row::new(vec![
            Value::String("alice".to_string()),
            Value::Int(35),
        ]))
        .expect("row matches table schema");
    users
        .insert(Row::new(vec![
            Value::String("bob".to_string()),
            Value::Int(22),
        ]))
        .expect("row matches table schema");

    catalog
}

fn main() -> ExitCode {
    let Some(sql) = env::args().nth(1) else {
        eprintln!("usage: crumble \"<SQL query>\"");
        return ExitCode::FAILURE;
    };

    let ast = match parse(&sql) {
        Ok(ast) => ast,
        Err(err) => {
            eprintln!("parse error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let logical = match lower(&ast) {
        Ok(plan) => plan,
        Err(err) => {
            eprintln!("lowering error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let optimized = ConstantFold.apply(logical);
    let physical = to_physical(optimized);

    let catalog = seeded_catalog();

    match execute(&physical, &catalog) {
        Ok(result) => {
            println!("{:?}", result.columns());
            for row in result.rows() {
                println!("{:?}", row.values());
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("execution error: {err}");
            ExitCode::FAILURE
        }
    }
}