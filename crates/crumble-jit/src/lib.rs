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
    function: unsafe extern "C" fn(*const i64) -> bool,
}

impl<'ctx> CompiledPredicate<'ctx> {
    /// # Safety
    /// `values` must point to at least as many i64s as `column_index` maps
    /// were built against, in the same order.
    pub unsafe fn call(&self, values: *const i64) -> bool {
        (self.function)(values)
    }
}

/// Attempts to compile `expr` into native code. Returns None if the
/// expression uses anything outside the supported subset (String, Float,
/// NULL, columns not in column_index) — callers must fall back to the
/// tree-walking interpreter in that case, nothing breaks.
pub fn compile_predicate<'ctx>(
    context: &'ctx Context,
    expr: &Expr,
    column_index: &HashMap<String, usize>,
) -> Option<CompiledPredicate<'ctx>> {
    let module = context.create_module("predicate");
    let builder = context.create_builder();

    let i64_type = context.i64_type();
    let bool_type = context.bool_type();
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());

    let fn_type = bool_type.fn_type(&[ptr_type.into()], false);
    let function = module.add_function("predicate", fn_type, None);
    let entry = context.append_basic_block(function, "entry");
    builder.position_at_end(entry);

    let values_ptr = function.get_nth_param(0)?.into_pointer_value();

    let result = codegen_expr(context, &builder, expr, column_index, values_ptr)?;
    builder.build_return(Some(&result)).ok()?;

    let execution_engine = module
        .create_jit_execution_engine(inkwell::OptimizationLevel::Default)
        .ok()?;

    type PredicateFn = unsafe extern "C" fn(*const i64) -> bool;
    let raw_fn = unsafe {
        execution_engine
            .get_function::<PredicateFn>("predicate")
            .ok()?
    };
    let function_ptr = unsafe {
        std::mem::transmute::<_, unsafe extern "C" fn(*const i64) -> bool>(raw_fn.as_raw())
    };

    Some(CompiledPredicate {
        _execution_engine: execution_engine,
        function: function_ptr,
    })
}

fn codegen_expr<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    expr: &Expr,
    column_index: &HashMap<String, usize>,
    values_ptr: PointerValue<'ctx>,
) -> Option<inkwell::values::IntValue<'ctx>> {
    match expr {
        Expr::Column(name) => {
            let idx = *column_index.get(name)?;
            let i64_type = context.i64_type();
            let offset = i64_type.const_int(idx as u64, false);
            let elem_ptr = unsafe {
                builder
                    .build_gep(i64_type, values_ptr, &[offset], "elem_ptr")
                    .ok()?
            };
            builder
                .build_load(i64_type, elem_ptr, "loaded")
                .ok()?
                .into_int_value()
                .into()
        }
        Expr::Literal(Literal::Int(n)) => Some(context.i64_type().const_int(*n as u64, true)),
        Expr::Literal(Literal::Bool(b)) => {
            // represented as i64 0/1 here so it can flow through the same
            // int-typed codegen path as everything else — only converted
            // to a real i1 bool at the final comparison/return point.
            Some(context.i64_type().const_int(*b as u64, false))
        }
        Expr::Literal(_) => None,    // String/Float/Null  outside scope
        Expr::IsNull { .. } => None, // NULL handling not compiled outside scope
        Expr::Like { .. } => None,   // pattern matching not compiled  outside scope
        Expr::BinaryOp { left, op, right } => {
            codegen_binary_op(context, builder, left, *op, right, column_index, values_ptr)
        }
    }
}

fn codegen_binary_op<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    left: &Expr,
    op: BinaryOperator,
    right: &Expr,
    column_index: &HashMap<String, usize>,
    values_ptr: PointerValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    match op {
        BinaryOperator::And | BinaryOperator::Or => {
            // And/Or combine two BOOLEAN results (0/1), not two raw column
            // values — codegen each side as its own top-level i1 predicate,
            // not through the shared int path Column/Literal use.
            let l = codegen_bool_expr(context, builder, left, column_index, values_ptr)?;
            let r = codegen_bool_expr(context, builder, right, column_index, values_ptr)?;
            let combined = match op {
                BinaryOperator::And => builder.build_and(l, r, "and_result").ok()?,
                BinaryOperator::Or => builder.build_or(l, r, "or_result").ok()?,
                _ => unreachable!(),
            };
            Some(
                builder
                    .build_int_z_extend(combined, context.i64_type(), "zext")
                    .ok()?,
            )
        }
        _ => {
            let l = codegen_expr(context, builder, left, column_index, values_ptr)?;
            let r = codegen_expr(context, builder, right, column_index, values_ptr)?;
            let predicate = match op {
                BinaryOperator::Eq => IntPredicate::EQ,
                BinaryOperator::NotEq => IntPredicate::NE,
                BinaryOperator::Lt => IntPredicate::SLT,
                BinaryOperator::LtEq => IntPredicate::SLE,
                BinaryOperator::Gt => IntPredicate::SGT,
                BinaryOperator::GtEq => IntPredicate::SGE,
                BinaryOperator::Add => return None, // arithmetic, not a predicate result outside scope here
                BinaryOperator::And | BinaryOperator::Or => unreachable!(),
            };
            let cmp = builder.build_int_compare(predicate, l, r, "cmp").ok()?;
            Some(
                builder
                    .build_int_z_extend(cmp, context.i64_type(), "zext")
                    .ok()?,
            )
        }
    }
}

/// Like codegen_expr, but produces a real i1 (LLVM's boolean type) instead
/// of the i64-everywhere convention codegen_expr uses — needed specifically
/// for And/Or's operands, which must be true booleans to use LLVM's actual
/// and/or instructions correctly.
fn codegen_bool_expr<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    expr: &Expr,
    column_index: &HashMap<String, usize>,
    values_ptr: PointerValue<'ctx>,
) -> Option<inkwell::values::IntValue<'ctx>> {
    let as_i64 = codegen_expr(context, builder, expr, column_index, values_ptr)?;
    let zero = context.i64_type().const_int(0, false);
    builder
        .build_int_compare(IntPredicate::NE, as_i64, zero, "as_bool")
        .ok()
}

#[cfg(test)]
mod predicate_tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn compiles_and_evaluates_simple_comparison() {
        let context = Context::create();
        let mut columns = HashMap::new();
        columns.insert("age".to_string(), 0);

        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Column("age".to_string())),
            op: BinaryOperator::Gt,
            right: Box::new(Expr::Literal(Literal::Int(30))),
        };

        let compiled = compile_predicate(&context, &expr, &columns)
            .expect("age > 30 is within the supported subset");

        let row_35 = [35i64];
        let row_20 = [20i64];
        unsafe {
            assert!(compiled.call(row_35.as_ptr()), "35 > 30 should be true");
            assert!(!compiled.call(row_20.as_ptr()), "20 > 30 should be false");
        }
    }

    #[test]
    fn compiles_and_evaluates_and_combinator() {
        let context = Context::create();
        let mut columns = HashMap::new();
        columns.insert("age".to_string(), 0);

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
            compile_predicate(&context, &expr, &columns).expect("within supported subset");

        unsafe {
            assert!(compiled.call([50i64].as_ptr()), "50 is between 20 and 100");
            assert!(!compiled.call([10i64].as_ptr()), "10 is not > 20");
            assert!(!compiled.call([500i64].as_ptr()), "500 is not < 100");
        }
    }

    #[test]
    fn returns_none_for_unsupported_expressions() {
        let context = Context::create();
        let columns = HashMap::new();

        let string_expr = Expr::BinaryOp {
            left: Box::new(Expr::Column("name".to_string())),
            op: BinaryOperator::Eq,
            right: Box::new(Expr::Literal(Literal::String("alice".to_string()))),
        };

        assert!(
            compile_predicate(&context, &string_expr, &columns).is_none(),
            "String comparisons are outside the supported subset — must fall back to the interpreter, not fail or panic"
        );
    }
}
