use crumble_storage::Row;

/**
Why this exists at all? crumble_storage::Row is just Vec<Value>
no column names attached, by design (a table's rows shouldn't each carry a copy of their own schema).
But once execution starts flowing up through Filter/Project,
something has to know index 0 is name, index 1 is age and so on..
to resolve Expr::Column("age") into an actual position.
RowSet is that carrier schema (columns) travels alongside data (rows) as a pair,
at every stage of the plan tree, not just at the leaf.
*/
#[derive(Debug, Clone, PartialEq)]
pub struct RowSet {
    columns: Vec<String>,
    rows: Vec<Row>,
}

impl RowSet {
    pub fn new(columns: Vec<String>, rows: Vec<Row>) -> Self {
        Self { columns, rows }
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|col| col == name)
    }
}
