use std::collections::HashMap;

use crumble_ir::{BinaryOperator, Expr, Literal};
use inkwell::IntPredicate;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::execution_engine::ExecutionEngine;
use inkwell::values::{IntValue, PointerValue};

/// Owns the LLVM context + compiled code's memory. Must stay alive for as
/// long as `function` might be called — dropping this while `function` is
/// still in use would be undefined behavior, since the compiled machine
/// code physically lives inside memory this struct owns.
pub struct CompiledPredicate<'ctx> {
    _execution_engine: ExecutionEngine<'ctx>,
    function: unsafe extern "C" fn(*const i64, *const i64) -> bool,
}

impl<'ctx> CompiledPredicate<'ctx> {
    /// # Safety
    /// `values` and `nulls` must each point to at least as many i64s as
    /// column_index maps were built against, in the same order.
    pub unsafe fn call(&self, values: *const i64, nulls: *const i64) -> bool {
        unsafe { (self.function)(values, nulls) }
    }
}

pub fn compile_predicate<'ctx>(
    context: &'ctx Context,
    expr: &Expr,
    column_index: &HashMap<String, (usize, ColumnKind)>,
) -> Option<CompiledPredicate<'ctx>> {
    let module = context.create_module("predicate");
    let builder = context.create_builder();

    let i64_type = context.i64_type();
    let bool_type = context.bool_type();
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());

    let fn_type = bool_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
    let function = module.add_function("predicate", fn_type, None);
    let entry = context.append_basic_block(function, "entry");
    builder.position_at_end(entry);

    let values_ptr = function.get_nth_param(0)?.into_pointer_value();
    let nulls_ptr = function.get_nth_param(1)?.into_pointer_value();

    let tristate = codegen_predicate(context, &builder, expr, column_index, values_ptr, nulls_ptr)?;

    let true_val = i64_type.const_int(TRISTATE_TRUE, false);
    let is_true = builder
        .build_int_compare(IntPredicate::EQ, tristate, true_val, "final_bool")
        .ok()?;
    builder.build_return(Some(&is_true)).ok()?;

    let execution_engine = module
        .create_jit_execution_engine(inkwell::OptimizationLevel::Default)
        .ok()?;

    type PredicateFn = unsafe extern "C" fn(*const i64, *const i64) -> bool;
    let raw_fn = unsafe {
        execution_engine
            .get_function::<PredicateFn>("predicate")
            .ok()?
    };
    let function_ptr = unsafe {
        std::mem::transmute::<_, unsafe extern "C" fn(*const i64, *const i64) -> bool>(
            raw_fn.as_raw(),
        )
    };

    Some(CompiledPredicate {
        _execution_engine: execution_engine,
        function: function_ptr,
    })
}

use inkwell::FloatPredicate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Int,
    Float,
}

const TRISTATE_FALSE: u64 = 0;
const TRISTATE_TRUE: u64 = 1;
const TRISTATE_NULL: u64 = 2;

/// Loads a value-producing expression's raw bits + null flag + declared
/// type. Column reads pull from both the values and nulls arrays; literals
/// are never null except Literal::Null itself.
fn codegen_value<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    expr: &Expr,
    column_index: &HashMap<String, (usize, ColumnKind)>,
    values_ptr: PointerValue<'ctx>,
    nulls_ptr: PointerValue<'ctx>,
) -> Option<(IntValue<'ctx>, IntValue<'ctx>, ColumnKind)> {
    let i64_type = context.i64_type();
    let zero = i64_type.const_int(0, false);

    match expr {
        Expr::Column(name) => {
            let &(idx, kind) = column_index.get(name)?;
            let offset = i64_type.const_int(idx as u64, false);

            let value_ptr = unsafe {
                builder
                    .build_gep(i64_type, values_ptr, &[offset], "val_ptr")
                    .ok()?
            };
            let raw_value = builder
                .build_load(i64_type, value_ptr, "raw_val")
                .ok()?
                .into_int_value();

            let null_ptr = unsafe {
                builder
                    .build_gep(i64_type, nulls_ptr, &[offset], "null_ptr")
                    .ok()?
            };
            let is_null = builder
                .build_load(i64_type, null_ptr, "is_null")
                .ok()?
                .into_int_value();

            Some((raw_value, is_null, kind))
        }
        Expr::Literal(Literal::Int(n)) => {
            Some((i64_type.const_int(*n as u64, true), zero, ColumnKind::Int))
        }
        Expr::Literal(Literal::Bool(b)) => {
            Some((i64_type.const_int(*b as u64, false), zero, ColumnKind::Int))
        }
        Expr::Literal(Literal::Float(f)) => {
            let bits = builder
                .build_bit_cast(context.f64_type().const_float(*f), i64_type, "float_bits")
                .ok()?
                .into_int_value();
            Some((bits, zero, ColumnKind::Float))
        }
        Expr::Literal(Literal::Null) => Some((zero, i64_type.const_int(1, false), ColumnKind::Int)),
        Expr::Literal(Literal::String(_)) => None,
        _ => None, // BinaryOp/And/Or/IsNull are predicate-shaped, handled below
    }
}

/// Produces a tristate result (0=false, 1=true, 2=null) for boolean-shaped
/// expressions — this is what Filter actually walks the tree with.
fn codegen_predicate<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    expr: &Expr,
    column_index: &HashMap<String, (usize, ColumnKind)>,
    values_ptr: PointerValue<'ctx>,
    nulls_ptr: PointerValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    let i64_type = context.i64_type();

    match expr {
        Expr::IsNull {
            expr: inner,
            negated,
        } => {
            let (_, is_null, _) =
                codegen_value(context, builder, inner, column_index, values_ptr, nulls_ptr)?;
            if *negated {
                let zero = i64_type.const_int(0, false);
                let one = i64_type.const_int(1, false);
                let not_null = builder
                    .build_int_compare(IntPredicate::EQ, is_null, zero, "not_null")
                    .ok()?;
                Some(
                    builder
                        .build_select(not_null, one, zero, "is_not_null")
                        .ok()?
                        .into_int_value(),
                )
            } else {
                Some(is_null) // IS NULL result IS the null flag itself, already 0/1
            }
        }

        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            let l = codegen_predicate(context, builder, left, column_index, values_ptr, nulls_ptr)?;
            let r =
                codegen_predicate(context, builder, right, column_index, values_ptr, nulls_ptr)?;
            tristate_and(context, builder, l, r)
        }
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Or,
            right,
        } => {
            let l = codegen_predicate(context, builder, left, column_index, values_ptr, nulls_ptr)?;
            let r =
                codegen_predicate(context, builder, right, column_index, values_ptr, nulls_ptr)?;
            tristate_or(context, builder, l, r)
        }
        Expr::BinaryOp { left, op, right } => {
            let (lv, l_null, lk) =
                codegen_value(context, builder, left, column_index, values_ptr, nulls_ptr)?;
            let (rv, r_null, rk) =
                codegen_value(context, builder, right, column_index, values_ptr, nulls_ptr)?;
            if lk != rk {
                return None; // mismatched types — outside scope
            }

            let zero = i64_type.const_int(0, false);
            let l_is_null = builder
                .build_int_compare(IntPredicate::NE, l_null, zero, "l_null")
                .ok()?;
            let r_is_null = builder
                .build_int_compare(IntPredicate::NE, r_null, zero, "r_null")
                .ok()?;
            let either_null = builder.build_or(l_is_null, r_is_null, "either_null").ok()?;

            let cmp = codegen_comparison(context, builder, lv, rv, *op, lk)?;
            let cmp_as_tristate = builder
                .build_int_z_extend(cmp, i64_type, "cmp_tristate")
                .ok()?;
            let null_val = i64_type.const_int(TRISTATE_NULL, false);

            Some(
                builder
                    .build_select(either_null, null_val, cmp_as_tristate, "cmp_or_null")
                    .ok()?
                    .into_int_value(),
            )
        }
        _ => None,
    }
}

fn codegen_comparison<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    lv: IntValue<'ctx>,
    rv: IntValue<'ctx>,
    op: BinaryOperator,
    kind: ColumnKind,
) -> Option<IntValue<'ctx>> {
    match kind {
        ColumnKind::Int => {
            let predicate = match op {
                BinaryOperator::Eq => IntPredicate::EQ,
                BinaryOperator::NotEq => IntPredicate::NE,
                BinaryOperator::Lt => IntPredicate::SLT,
                BinaryOperator::LtEq => IntPredicate::SLE,
                BinaryOperator::Gt => IntPredicate::SGT,
                BinaryOperator::GtEq => IntPredicate::SGE,
                _ => return None,
            };
            builder.build_int_compare(predicate, lv, rv, "icmp").ok()
        }
        ColumnKind::Float => {
            let f64_type = context.f64_type();
            let lf = builder
                .build_bit_cast(lv, f64_type, "lf")
                .ok()?
                .into_float_value();
            let rf = builder
                .build_bit_cast(rv, f64_type, "rf")
                .ok()?
                .into_float_value();
            let predicate = match op {
                BinaryOperator::Eq => FloatPredicate::OEQ,
                BinaryOperator::NotEq => FloatPredicate::ONE,
                BinaryOperator::Lt => FloatPredicate::OLT,
                BinaryOperator::LtEq => FloatPredicate::OLE,
                BinaryOperator::Gt => FloatPredicate::OGT,
                BinaryOperator::GtEq => FloatPredicate::OGE,
                _ => return None,
            };
            builder.build_float_compare(predicate, lf, rf, "fcmp").ok()
        }
    }
}

fn tristate_and<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    l: IntValue<'ctx>,
    r: IntValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    let i64_type = context.i64_type();
    let (false_v, true_v, null_v) = (
        i64_type.const_int(TRISTATE_FALSE, false),
        i64_type.const_int(TRISTATE_TRUE, false),
        i64_type.const_int(TRISTATE_NULL, false),
    );

    let either_false = builder
        .build_or(
            builder
                .build_int_compare(IntPredicate::EQ, l, false_v, "lf")
                .ok()?,
            builder
                .build_int_compare(IntPredicate::EQ, r, false_v, "rf")
                .ok()?,
            "either_false",
        )
        .ok()?;
    let either_null = builder
        .build_or(
            builder
                .build_int_compare(IntPredicate::EQ, l, null_v, "ln")
                .ok()?,
            builder
                .build_int_compare(IntPredicate::EQ, r, null_v, "rn")
                .ok()?,
            "either_null",
        )
        .ok()?;

    let null_or_true = builder
        .build_select(either_null, null_v, true_v, "null_or_true")
        .ok()?
        .into_int_value();
    Some(
        builder
            .build_select(either_false, false_v, null_or_true, "and_result")
            .ok()?
            .into_int_value(),
    )
}

fn tristate_or<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    l: IntValue<'ctx>,
    r: IntValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    let i64_type = context.i64_type();
    let (false_v, true_v, null_v) = (
        i64_type.const_int(TRISTATE_FALSE, false),
        i64_type.const_int(TRISTATE_TRUE, false),
        i64_type.const_int(TRISTATE_NULL, false),
    );

    let either_true = builder
        .build_or(
            builder
                .build_int_compare(IntPredicate::EQ, l, true_v, "lt")
                .ok()?,
            builder
                .build_int_compare(IntPredicate::EQ, r, true_v, "rt")
                .ok()?,
            "either_true",
        )
        .ok()?;
    let either_null = builder
        .build_or(
            builder
                .build_int_compare(IntPredicate::EQ, l, null_v, "ln")
                .ok()?,
            builder
                .build_int_compare(IntPredicate::EQ, r, null_v, "rn")
                .ok()?,
            "either_null",
        )
        .ok()?;

    let null_or_false = builder
        .build_select(either_null, null_v, false_v, "null_or_false")
        .ok()?
        .into_int_value();
    Some(
        builder
            .build_select(either_true, true_v, null_or_false, "or_result")
            .ok()?
            .into_int_value(),
    )
}

#[cfg(test)]
mod predicate_tests {
    use super::*;
    use std::collections::HashMap;

    fn columns() -> HashMap<String, (usize, ColumnKind)> {
        HashMap::from([("age".to_string(), (0, ColumnKind::Int))])
    }

    #[test]
    fn compiles_and_evaluates_simple_comparison() {
        let context = Context::create();
        let columns = columns();

        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Column("age".to_string())),
            op: BinaryOperator::Gt,
            right: Box::new(Expr::Literal(Literal::Int(30))),
        };

        let compiled =
            compile_predicate(&context, &expr, &columns).expect("age > 30 should compile");

        let row_35 = [35i64];
        let row_20 = [20i64];

        let no_nulls = [0i64];

        unsafe {
            assert!(
                compiled.call(row_35.as_ptr(), no_nulls.as_ptr()),
                "35 > 30 should be true"
            );

            assert!(
                !compiled.call(row_20.as_ptr(), no_nulls.as_ptr()),
                "20 > 30 should be false"
            );
        }
    }

    #[test]
    fn compiles_and_evaluates_and_combinator() {
        let context = Context::create();
        let columns = columns();

        // age > 20 AND age < 100
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Column("age".to_string())),
                op: BinaryOperator::Gt,
                right: Box::new(Expr::Literal(Literal::Int(20))),
            }),
            op: BinaryOperator::And,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Column("age".to_string())),
                op: BinaryOperator::Lt,
                right: Box::new(Expr::Literal(Literal::Int(100))),
            }),
        };

        let compiled =
            compile_predicate(&context, &expr, &columns).expect("AND expression should compile");

        let no_nulls = [0i64];

        unsafe {
            assert!(
                compiled.call([50i64].as_ptr(), no_nulls.as_ptr()),
                "50 is between 20 and 100"
            );

            assert!(
                !compiled.call([10i64].as_ptr(), no_nulls.as_ptr()),
                "10 is not > 20"
            );

            assert!(
                !compiled.call([500i64].as_ptr(), no_nulls.as_ptr()),
                "500 is not < 100"
            );
        }
    }

    #[test]
    fn returns_none_for_unsupported_expressions() {
        let context = Context::create();

        let columns = HashMap::from([("name".to_string(), (0, ColumnKind::Int))]);

        let string_expr = Expr::BinaryOp {
            left: Box::new(Expr::Column("name".to_string())),
            op: BinaryOperator::Eq,
            right: Box::new(Expr::Literal(Literal::String("alice".to_string()))),
        };

        assert!(
            compile_predicate(&context, &string_expr, &columns).is_none(),
            "String comparisons should fall back to the interpreter"
        );
    }
}
