use crate::LogicalPlan;
use crate::physical::PhysicalPlan;

/** every Scan becomes a SeqScan, nothing else changes.
That triviality is fine; the value is that this function is now the one single place that decides physical strategy.
*/
pub fn to_physical(plan: LogicalPlan) -> PhysicalPlan {
    match plan {
        LogicalPlan::Scan { table } => PhysicalPlan::SeqScan { table },
        LogicalPlan::Filter { input, predicate } => PhysicalPlan::Filter {
            input: Box::new(to_physical(*input)),
            predicate,
        },
        LogicalPlan::Project { input, columns } => PhysicalPlan::Project {
            input: Box::new(to_physical(*input)),
            columns,
        },
        LogicalPlan::Insert {
            table,
            columns,
            rows,
        } => PhysicalPlan::Insert {
            table,
            columns,
            rows,
        },
        LogicalPlan::CreateTable { table, columns } => PhysicalPlan::CreateTable { table, columns },
        LogicalPlan::Delete { table, predicate } => PhysicalPlan::Delete { table, predicate },
        LogicalPlan::Update {
            table,
            assignments,
            predicate,
        } => PhysicalPlan::Update {
            table,
            assignments,
            predicate,
        },
    }
}
