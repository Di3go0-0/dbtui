//! Shared data types used across the SQL engine.
//! These are engine-internal — they do not depend on core::models or UI types.

use std::fmt;

/// Row/column location in the source text for diagnostic underlines and completion origins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub row: usize,
    pub col_start: usize,
    pub col_end: usize,
}

/// A possibly-qualified SQL object name (e.g., `hr.employees` or just `employees`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QualifiedName {
    pub schema: Option<String>,
    pub name: String,
}

impl fmt::Display for QualifiedName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.schema {
            Some(s) => write!(f, "{}.{}", s, self.name),
            None => write!(f, "{}", self.name),
        }
    }
}

/// A table/view reference extracted from the query, with optional alias and position.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TableReference {
    pub qualified_name: QualifiedName,
    pub alias: Option<String>,
    pub location: Location,
    /// When this ref is `TABLE(schema.pkg.func(...))`, capture the inner
    /// function call so completion can resolve its return-type columns.
    pub function_call: Option<crate::sql_engine::tokenizer::TableFunctionCall>,
}

/// A column reference extracted from the query (e.g., `e.department_id`).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ColumnReference {
    /// The qualifier before the dot (alias or table name), if present.
    pub table_qualifier: Option<String>,
    pub name: String,
    pub location: Location,
}

/// Resolved column metadata from the database, enriched with table context.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ResolvedColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub is_primary_key: bool,
    pub table_schema: String,
    pub table_name: String,
}

/// Foreign key relationship between two tables.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ForeignKey {
    pub constraint_name: String,
    pub from_schema: String,
    pub from_table: String,
    pub from_columns: Vec<String>,
    pub to_schema: String,
    pub to_table: String,
    pub to_columns: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_name_display() {
        let unqualified = QualifiedName {
            schema: None,
            name: "employees".to_string(),
        };
        assert_eq!(unqualified.to_string(), "employees");

        let qualified = QualifiedName {
            schema: Some("hr".to_string()),
            name: "employees".to_string(),
        };
        assert_eq!(qualified.to_string(), "hr.employees");
    }
}
