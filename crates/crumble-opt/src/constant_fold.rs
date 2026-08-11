use crate::pass::OptimizationPass;
use crumble_ir::{BinaryOperator, Expr, Literal, LogicalPlan};

pub struct ConstantFold;

impl OptimizationPass for ConstantFold {
    fn apply(&self, plan: LogicalPlan) -> LogicalPlan {
        fold_plan(plan)
    }
}

fn fold_plan(plan: LogicalPlan) -> LogicalPlan {
    match plan {
        LogicalPlan::Scan { table } => LogicalPlan::Scan { table },
        LogicalPlan::Filter { input, predicate } => LogicalPlan::Filter {
            input: Box::new(fold_plan(*input)),
            predicate: fold_expr(predicate),
        },
        LogicalPlan::Project { input, columns } => LogicalPlan::Project {
            input: Box::new(fold_plan(*input)),
            columns,
        },
        LogicalPlan::Insert {
            table,
            columns,
            rows,
        } => LogicalPlan::Insert {
            table,
            columns,
            rows,
        },
    }
}

fn fold_expr(expr: Expr) -> Expr {
    match expr {
        Expr::BinaryOp { left, op, right } => {
            let left = fold_expr(*left);
            let right = fold_expr(*right);

            match fold_binary_op(&left, op, &right) {
                Some(literal) => Expr::Literal(literal),
                None => Expr::BinaryOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
            }
        }
        other => other,
    }
}

/**
folding binary op returns option<Literal> bcz none means can't fold due to type mismatch or not possible
not an error. Actually an optimizer pass should never fail.
*/
fn fold_binary_op(left: &Expr, op: BinaryOperator, right: &Expr) -> Option<Literal> {
    let (Expr::Literal(left), Expr::Literal(right)) = (left, right) else {
        return None;
    };

    match (left, right) {
        (Literal::Int(l), Literal::Int(r)) => fold_int(*l, op, *r),
        (Literal::Bool(l), Literal::Bool(r)) => fold_bool(*l, op, *r),
        (Literal::String(l), Literal::String(r)) => fold_string(l, op, r),
        (Literal::Float(l), Literal::Float(r)) => fold_float(*l, op, *r),
        _ => None,
    }
}

fn fold_int(l: i64, op: BinaryOperator, r: i64) -> Option<Literal> {
    match op {
        BinaryOperator::Eq => Some(Literal::Bool(l == r)),
        BinaryOperator::NotEq => Some(Literal::Bool(l != r)),
        BinaryOperator::Lt => Some(Literal::Bool(l < r)),
        BinaryOperator::LtEq => Some(Literal::Bool(l <= r)),
        BinaryOperator::Gt => Some(Literal::Bool(l > r)),
        BinaryOperator::GtEq => Some(Literal::Bool(l >= r)),
        BinaryOperator::And => None,
        BinaryOperator::Or => None,
    }
}

fn fold_bool(l: bool, op: BinaryOperator, r: bool) -> Option<Literal> {
    match op {
        BinaryOperator::Eq => Some(Literal::Bool(l == r)),
        BinaryOperator::NotEq => Some(Literal::Bool(l != r)),
        BinaryOperator::Lt => None,
        BinaryOperator::LtEq => None,
        BinaryOperator::Gt => None,
        BinaryOperator::GtEq => None,
        BinaryOperator::And => Some(Literal::Bool(l && r)),
        BinaryOperator::Or => Some(Literal::Bool(l || r)),
    }
}

fn fold_string(l: &str, op: BinaryOperator, r: &str) -> Option<Literal> {
    match op {
        BinaryOperator::Eq => Some(Literal::Bool(l == r)),
        BinaryOperator::NotEq => Some(Literal::Bool(l != r)),
        BinaryOperator::Lt => Some(Literal::Bool(l < r)),
        BinaryOperator::LtEq => Some(Literal::Bool(l <= r)),
        BinaryOperator::Gt => Some(Literal::Bool(l > r)),
        BinaryOperator::GtEq => Some(Literal::Bool(l >= r)),
        BinaryOperator::And => None,
        BinaryOperator::Or => None,
    }
}

fn fold_float(l: f64, op: BinaryOperator, r: f64) -> Option<Literal> {
    match op {
        BinaryOperator::Eq => Some(Literal::Bool(l == r)),
        BinaryOperator::NotEq => Some(Literal::Bool(l != r)),
        BinaryOperator::Lt => Some(Literal::Bool(l < r)),
        BinaryOperator::LtEq => Some(Literal::Bool(l <= r)),
        BinaryOperator::Gt => Some(Literal::Bool(l > r)),
        BinaryOperator::GtEq => Some(Literal::Bool(l >= r)),
        BinaryOperator::And => None,
        BinaryOperator::Or => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crumble_ir::{BinaryOperator, Expr, Literal, LogicalPlan};

    #[test]
    fn folds_true_predicate() {
        let plan = LogicalPlan::Filter {
            input: Box::new(LogicalPlan::Scan {
                table: "users".to_string(),
            }),
            predicate: Expr::BinaryOp {
                left: Box::new(Expr::Literal(Literal::Int(1))),
                op: BinaryOperator::Eq,
                right: Box::new(Expr::Literal(Literal::Int(1))),
            },
        };

        let folded = ConstantFold.apply(plan);

        assert_eq!(
            folded,
            LogicalPlan::Filter {
                input: Box::new(LogicalPlan::Scan {
                    table: "users".to_string()
                }),
                predicate: Expr::Literal(Literal::Bool(true)),
            }
        );
    }

    #[test]
    fn leaves_column_reference_unfolded() {
        let plan = LogicalPlan::Filter {
            input: Box::new(LogicalPlan::Scan {
                table: "users".to_string(),
            }),
            predicate: Expr::BinaryOp {
                left: Box::new(Expr::Column("age".to_string())),
                op: BinaryOperator::Gt,
                right: Box::new(Expr::Literal(Literal::Int(30))),
            },
        };

        let folded = ConstantFold.apply(plan.clone());

        assert_eq!(folded, plan);
    }
}
