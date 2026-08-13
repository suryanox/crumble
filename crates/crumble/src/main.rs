use std::io;
use std::io::Write;
use std::process::ExitCode;

use crumble_exec::execute;
use crumble_ir::{lower, to_physical};
use crumble_opt::{ConstantFold, OptimizationPass};
use crumble_sql::parse;
use crumble_storage::{Catalog, Row, StorageError, Value};

fn seeded_catalog() -> Result<Catalog, StorageError> {
    let mut catalog = Catalog::open("./crumble-data")?;

    if catalog.get("users").is_err() {
        catalog.create_table("users", vec!["name".to_string(), "age".to_string()])?;

        let users = catalog.get_mut("users")?;
        users.insert(Row::new(vec![
            Value::String("alice".to_string()),
            Value::Int(5),
        ]))?;
        users.insert(Row::new(vec![
            Value::String("bob".to_string()),
            Value::Int(2),
        ]))?;
    }

    Ok(catalog)
}
const RESET: &str = "\x1b[0m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const DIM: &str = "\x1b[2m";

fn main() -> ExitCode {
    let mut catalog = match seeded_catalog() {
        Ok(catalog) => catalog,
        Err(err) => {
            eprintln!("{RED}storage error:{RESET} {err}");
            return ExitCode::FAILURE;
        }
    };
    loop {
        print!("{CYAN}crumble>{RESET} ");
        io::stdout().flush().unwrap();

        let mut sql = String::new();

        match io::stdin().read_line(&mut sql) {
            Ok(0) => break,
            Ok(_) => {}
            Err(err) => {
                eprintln!("{RED}read error:{RESET} {err}");
                continue;
            }
        }

        let sql = sql.trim();

        if sql.is_empty() {
            continue;
        }

        if sql.eq_ignore_ascii_case("quit") || sql.eq_ignore_ascii_case("exit") {
            println!("{DIM}bye!{RESET}");
            break;
        }

        let ast = match parse(sql) {
            Ok(ast) => ast,
            Err(err) => {
                eprintln!("{RED}parse error:{RESET} {err}");
                continue;
            }
        };

        let logical = match lower(&ast) {
            Ok(plan) => plan,
            Err(err) => {
                eprintln!("{RED}lowering error:{RESET} {err}");
                continue;
            }
        };

        println!("{YELLOW}Logical Plan:{RESET}");
        println!("{logical:?}");

        let optimized = ConstantFold.apply(logical);
        let physical = to_physical(optimized);

        println!("{YELLOW}Physical Plan:{RESET}");
        println!("{physical:?}");

        match execute(&physical, &mut catalog) {
            Ok(result) if result.columns().is_empty() => {
                println!("{GREEN}✓ 1 row inserted{RESET}");
            }

            Ok(result) => {
                println!("{GREEN}{:?}{RESET}", result.columns());

                for row in result.rows() {
                    println!("{GREEN}{:?}{RESET}", row.values());
                }
            }

            Err(err) => {
                eprintln!("{RED}execution error:{RESET} {err}");
            }
        }
    }

    ExitCode::SUCCESS
}
