use crumble_ir::{BinaryOperator, Expr, Literal, PhysicalPlan};
use crumble_storage::Catalog;

pub fn plan_index_scans(plan: PhysicalPlan, catalog: &Catalog) -> PhysicalPlan {
    match plan {
        PhysicalPlan::Filter { input, predicate } => {
            if let PhysicalPlan::SeqScan { table } = input.as_ref() {
                if let Some(rewrite) = try_rewrite(table, &predicate, catalog) {
                    return rewrite;
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

fn try_rewrite(table: &str, predicate: &Expr, catalog: &Catalog) -> Option<PhysicalPlan> {
    if let Some((column, key)) = equality_on_literal(predicate) {
        let index_name = catalog.index_for(table, &column)?;
        return Some(PhysicalPlan::IndexScan {
            table: table.to_string(),
            index_name: index_name.to_string(),
            key,
        });
    }

    if let Some((column, bound, inclusive, lower_side)) = comparison_on_literal(predicate) {
        let index_name = catalog.index_for(table, &column)?;
        let (lower, upper) = if lower_side {
            (Some((bound, inclusive)), None)
        } else {
            (None, Some((bound, inclusive)))
        };

        return Some(PhysicalPlan::RangeIndexScan {
            table: table.to_string(),
            index_name: index_name.to_string(),
            lower,
            upper,
        });
    }
    None
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

/// Returns (column, bound, inclusive, is_lower_bound).
/// Handles both operand orders: `age > 30` and `30 < age` are equivalent.
fn comparison_on_literal(expr: &Expr) -> Option<(String, Literal, bool, bool)> {
    let Expr::BinaryOp { left, op, right } = expr else {
        return None;
    };

    match (left.as_ref(), right.as_ref()) {
        (Expr::Column(col), Expr::Literal(lit)) => match op {
            BinaryOperator::Gt => Some((col.clone(), lit.clone(), false, true)),
            BinaryOperator::GtEq => Some((col.clone(), lit.clone(), true, true)),
            BinaryOperator::Lt => Some((col.clone(), lit.clone(), false, false)),
            BinaryOperator::LtEq => Some((col.clone(), lit.clone(), true, false)),
            _ => None,
        },
        (Expr::Literal(lit), Expr::Column(col)) => match op {
            // operands flipped, so the direction flips too: `30 < age` means age > 30
            BinaryOperator::Lt => Some((col.clone(), lit.clone(), false, true)),
            BinaryOperator::LtEq => Some((col.clone(), lit.clone(), true, true)),
            BinaryOperator::Gt => Some((col.clone(), lit.clone(), false, false)),
            BinaryOperator::GtEq => Some((col.clone(), lit.clone(), true, false)),
            _ => None,
        },
        _ => None,
    }
}
