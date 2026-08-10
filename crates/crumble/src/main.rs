use std::env;
use std::process::ExitCode;

use crumble_ir::lower;
use crumble_opt::{ConstantFold, OptimizationPass};
use crumble_sql::parse;

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

    let plan = match lower(&ast) {
        Ok(plan) => plan,
        Err(err) => {
            eprintln!("lowering error: {err}");
            return ExitCode::FAILURE;
        }
    };

    println!("logical plan:\n{plan:#?}\n");

    let optimized = ConstantFold.apply(plan);

    println!("optimized plan:\n{optimized:#?}");

    ExitCode::SUCCESS
}