use inkwell::context::Context;
use inkwell::OptimizationLevel;

/// Proves the LLVM JIT plumbing works at all: compiles a trivial function
/// (add 1 to an i64) to native code and calls it. Nothing to do with our
/// actual query engine yet — this is purely "does the toolchain work."
pub fn smoke_test() -> i64 {
    let context = Context::create();
    let module = context.create_module("smoke_test");
    let builder = context.create_builder();

    let i64_type = context.i64_type();
    let fn_type = i64_type.fn_type(&[i64_type.into()], false);
    let function = module.add_function("add_one", fn_type, None);
    let basic_block = context.append_basic_block(function, "entry");
    builder.position_at_end(basic_block);

    let param = function.get_nth_param(0).unwrap().into_int_value();
    let one = i64_type.const_int(1, false);
    let result = builder.build_int_add(param, one, "result").unwrap();
    builder.build_return(Some(&result)).unwrap();

    let execution_engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .expect("failed to create JIT execution engine");

    type AddOneFn = unsafe extern "C" fn(i64) -> i64;
    let compiled_fn = unsafe {
        execution_engine
            .get_function::<AddOneFn>("add_one")
            .expect("failed to find compiled function")
    };

    unsafe { compiled_fn.call(41) }
}

#[cfg(test)]
mod tests {
    use super::smoke_test;

    #[test]
    fn jit_compiled_add_one_works() {
        assert_eq!(smoke_test(), 42);
    }
}