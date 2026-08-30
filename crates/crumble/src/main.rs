use crumble_exec::execute;
use crumble_ir::{lower, to_physical};
use crumble_opt::{ConstantFold, OptimizationPass};
use crumble_planner::plan_index_scans;
use crumble_storage::{Catalog, Row, StorageError};
use crumble_tx::TransactionManager;
use std::io;
use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;

use crumble_sql::{TransactionControl, parse, transaction_control};
use crumble_tx::TransactionId;

fn seeded_catalog() -> Result<Catalog, StorageError> {
    let tx_manager = Arc::new(TransactionManager::new());
    Catalog::open("./crumble-data", tx_manager)
}

const RESET: &str = "\x1b[0m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const DIM: &str = "\x1b[2m";

fn print_table(columns: &[String], rows: &[Row]) {
    if columns.is_empty() {
        println!("{GREEN}OK{RESET}");
        return;
    }

    let rendered: Vec<Vec<String>> = rows
        .iter()
        .map(|row| row.values().iter().map(|v| v.to_string()).collect())
        .collect();

    let mut widths: Vec<usize> = columns.iter().map(|c| c.len()).collect();
    for row in &rendered {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }

    let separator = || {
        let line = widths
            .iter()
            .map(|w| "-".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("+");
        println!("+{line}+");
    };

    let print_row = |cells: &[String]| {
        let line = cells
            .iter()
            .enumerate()
            .map(|(i, cell)| format!(" {cell:<width$} ", width = widths[i]))
            .collect::<Vec<_>>()
            .join("|");
        println!("|{line}|");
    };

    separator();
    print_row(columns);
    separator();
    for row in &rendered {
        print_row(row);
    }
    separator();

    println!(
        "{DIM}({} row{}){RESET}",
        rendered.len(),
        if rendered.len() == 1 { "" } else { "s" }
    );
}

fn main() -> ExitCode {
    let catalog = match seeded_catalog() {
        Ok(catalog) => catalog,
        Err(err) => {
            eprintln!("{RED}storage error:{RESET} {err}");
            return ExitCode::FAILURE;
        }
    };

    let mut session_xid: Option<TransactionId> = None;

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

        if let Some(control) = transaction_control(&ast) {
            match control {
                TransactionControl::Begin => {
                    if session_xid.is_some() {
                        println!("{YELLOW}warning: already in a transaction{RESET}");
                    } else {
                        session_xid = Some(catalog.tx_manager.begin());
                        println!("{GREEN}BEGIN{RESET}");
                    }
                }
                TransactionControl::Commit => match session_xid.take() {
                    Some(xid) => {
                        catalog.tx_manager.commit(xid);
                        println!("{GREEN}COMMIT{RESET}");
                    }
                    None => println!("{YELLOW}warning: no transaction in progress{RESET}"),
                },
                TransactionControl::Rollback => match session_xid.take() {
                    Some(xid) => {
                        catalog.tx_manager.abort(xid);
                        println!("{GREEN}ROLLBACK{RESET}");
                    }
                    None => println!("{YELLOW}warning: no transaction in progress{RESET}"),
                },
            }
            continue;
        }

        let logical = match lower(&ast) {
            Ok(plan) => plan,
            Err(err) => {
                eprintln!("{RED}lowering error:{RESET} {err}");
                continue;
            }
        };

        let optimized = ConstantFold.apply(logical);
        let physical = to_physical(optimized);
        let physical = plan_index_scans(physical, &catalog);

        let (xid, auto_commit) = match session_xid {
            Some(xid) => (xid, false),
            None => (catalog.tx_manager.begin(), true),
        };

        let result = execute(&physical, &catalog, xid);

        if auto_commit {
            match &result {
                Ok(_) => catalog.tx_manager.commit(xid),
                Err(_) => catalog.tx_manager.abort(xid),
            }
        } else if result.is_err() {
            catalog.tx_manager.abort(xid);
            session_xid = None;
        }

        match result {
            Ok(result) => print_table(result.columns(), result.rows()),
            Err(err) => eprintln!("{RED}execution error:{RESET} {err}"),
        }
    }

    ExitCode::SUCCESS
}
