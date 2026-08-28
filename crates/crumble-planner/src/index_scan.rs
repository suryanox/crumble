use crumble_ir::{BinaryOperator, Expr, JoinKind, Literal, PhysicalPlan};
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
        PhysicalPlan::NestedLoopJoin {
            left,
            right,
            left_table,
            right_table,
            on,
            kind,
        } => {
            let left = plan_index_scans(*left, catalog);

            if matches!(
                kind,
                JoinKind::Inner | JoinKind::Left | JoinKind::Right | JoinKind::FullOuter
            ) {
                if let PhysicalPlan::SeqScan { table: right_real } = right.as_ref() {
                    if let Some(rewrite) = try_index_join(
                        &left,
                        &left_table,
                        &right_table,
                        right_real,
                        &on,
                        &kind,
                        catalog,
                    ) {
                        return rewrite;
                    }
                }
            }

            PhysicalPlan::NestedLoopJoin {
                left: Box::new(left),
                right: Box::new(plan_index_scans(*right, catalog)),
                left_table,
                right_table,
                on,
                kind,
            }
        }
        other => other,
    }
}

fn try_index_join(
    left: &PhysicalPlan,
    left_table: &str,
    right_qualifier: &str,
    right_real: &str,
    on: &Expr,
    kind: &JoinKind,
    catalog: &Catalog,
) -> Option<PhysicalPlan> {
    let Expr::BinaryOp {
        left: on_left,
        op: BinaryOperator::Eq,
        right: on_right,
    } = on
    else {
        return None;
    };

    let (left_col, right_col) = match (on_left.as_ref(), on_right.as_ref()) {
        (Expr::Column(l), Expr::Column(r)) if l.starts_with(&format!("{left_table}.")) => {
            (l.clone(), strip_qualifier(r, right_qualifier)?)
        }
        (Expr::Column(l), Expr::Column(r)) if r.starts_with(&format!("{left_table}.")) => {
            (r.clone(), strip_qualifier(l, right_qualifier)?)
        }
        _ => return None,
    };

    let index_name = catalog.index_for(right_real, &right_col)?;

    Some(PhysicalPlan::IndexNestedLoopJoin {
        left: Box::new(left.clone()),
        left_table: left_table.to_string(),
        right_table_real: right_real.to_string(),
        right_table_qualifier: right_qualifier.to_string(),
        right_index_name: index_name.to_string(),
        left_join_column: left_col,
        kind: kind.clone(),
    })
}

fn strip_qualifier(qualified: &str, expected_qualifier: &str) -> Option<String> {
    qualified
        .strip_prefix(&format!("{expected_qualifier}."))?
        .to_string()
        .into()
}

fn try_rewrite(table: &str, predicate: &Expr, catalog: &Catalog) -> Option<PhysicalPlan> {
    if let Expr::IsNull { expr, negated } = predicate {
        if let Expr::Column(column) = expr.as_ref() {
            let index_name = catalog.index_for(table, column)?;
            return Some(if !negated {
                PhysicalPlan::IndexScan {
                    table: table.to_string(),
                    index_name: index_name.to_string(),
                    key: Literal::Null,
                }
            } else {
                PhysicalPlan::RangeIndexScan {
                    table: table.to_string(),
                    index_name: index_name.to_string(),
                    lower: None,
                    upper: Some((Literal::Null, false)),
                }
            });
        }
    }

    if let Some((column, key)) = equality_on_literal(predicate) {
        if matches!(key, Literal::Null) {
            return None; // `col = NULL` is never rewritten always empty, handled by Filter
        }
        let index_name = catalog.index_for(table, &column)?;
        return Some(PhysicalPlan::IndexScan {
            table: table.to_string(),
            index_name: index_name.to_string(),
            key,
        });
    }

    if let Some((column, bound, inclusive, lower_side)) = comparison_on_literal(predicate) {
        if matches!(bound, Literal::Null) {
            return None; // comparisons against NULL are always unknown — same reasoning
        }
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
