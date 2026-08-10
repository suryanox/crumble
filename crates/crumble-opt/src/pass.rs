use crumble_ir::LogicalPlan;
pub trait OptimizationPass {
    fn apply(&self, plan: LogicalPlan) -> LogicalPlan;
}
