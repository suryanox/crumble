use crumble_ir::{BinaryOperator, Expr, Literal, PhysicalPlan};
use crumble_storage::Catalog;

pub fn plan_index_scans(plan: PhysicalPlan, catalog: &Catalog) -> PhysicalPlan {
    match plan {
        PhysicalPlan::Filter { input, predicate } => {
            if let PhysicalPlan::SeqScan { table } = input.as_ref() {
                if let Some((column, key)) = equality_on_literal(&predicate) {
                    if let Some(index_name) = catalog.index_for(table, &column) {
                        return PhysicalPlan::IndexScan {
                            table: table.clone(),
                            index_name: index_name.to_string(),
                            key,
                        };
                    }
                }
            }

            PhysicalPlan::Filter {
                input: Box::new(plan_index_scans(*input, catalog)),
                predicate,
            }
        }
        PhysicalPlan::Project { input, columns } => PhysicalPlan::Project {
            input: Box::new(plan_index_scans(*input, catalog)),
            columns,
        },
        other => other,
    }
}

fn equality_on_literal(expr: &Expr) -> Option<(String, Literal)> {
    let Expr::BinaryOp {
        left,
        op: BinaryOperator::Eq,
        right,
    } = expr
    else {
        return None;
    };

    match (left.as_ref(), right.as_ref()) {
        (Expr::Column(col), Expr::Literal(lit)) => Some((col.clone(), lit.clone())),
        (Expr::Literal(lit), Expr::Column(col)) => Some((col.clone(), lit.clone())),
        _ => None,
    }
}
