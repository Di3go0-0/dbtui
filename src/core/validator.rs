use std::sync::Arc;

use sqlparser::dialect::{GenericDialect, MySqlDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;

use crate::core::adapter::DatabaseAdapter;
use crate::core::error::DbResult;
use crate::core::models::DatabaseType;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub line: usize,
    pub col: usize,
    pub message: String,
    pub severity: ErrorSeverity,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum ErrorSeverity {
    Syntax,
    Reference,
    Compilation,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub is_valid: bool,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<String>,
}

impl ValidationReport {
    pub fn ok() -> Self {
        Self {
            is_valid: true,
            errors: vec![],
            warnings: vec![],
        }
    }

    pub fn with_error(error: ValidationError) -> Self {
        Self {
            is_valid: false,
            errors: vec![error],
            warnings: vec![],
        }
    }

    pub fn error_summary(&self) -> String {
        self.errors
            .iter()
            .map(|e| {
                if e.line > 0 {
                    format!("Line {}: {}", e.line, e.message)
                } else {
                    e.message.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[allow(dead_code)]
pub struct SqlValidator {
    db_type: DatabaseType,
}

impl SqlValidator {
    pub fn new(db_type: DatabaseType) -> Self {
        Self { db_type }
    }

    /// Quick syntax-only validation using sqlparser.
    /// For Oracle PL/SQL (packages, procedures), sqlparser has limited support,
    /// so this performs best-effort validation - syntax errors from the parser
    /// are reported but a clean parse is not guaranteed for complex PL/SQL.
    pub fn validate_syntax(&self, content: &str) -> ValidationReport {
        if content.trim().is_empty() {
            return ValidationReport::ok();
        }

        // Oracle PL/SQL: sqlparser cannot parse CREATE OR REPLACE PACKAGE BODY,
        // DECLARE blocks, %TYPE references, etc. Use GenericDialect for best-effort.
        // For PostgreSQL/MySQL, use their specific dialects.
        let result = match self.db_type {
            DatabaseType::PostgreSQL => {
                let dialect = PostgreSqlDialect {};
                Parser::parse_sql(&dialect, content)
            }
            DatabaseType::MySQL => {
                let dialect = MySqlDialect {};
                Parser::parse_sql(&dialect, content)
            }
            DatabaseType::Oracle => {
                // GenericDialect is most lenient for Oracle PL/SQL
                let dialect = GenericDialect {};
                Parser::parse_sql(&dialect, content)
            }
        };

        match result {
            Ok(_) => ValidationReport::ok(),
            Err(e) => {
                let msg = e.to_string();
                // Try to extract line/col from error message
                let (line, col) = parse_error_position(&msg);
                ValidationReport::with_error(ValidationError {
                    line,
                    col,
                    message: msg,
                    severity: ErrorSeverity::Syntax,
                })
            }
        }
    }

    /// Thorough validation: syntax + reference checking against database.
    /// Extracts table/view references from SQL and verifies they exist.
    pub async fn validate_thorough(
        &self,
        schema: &str,
        content: &str,
        adapter: &Arc<dyn DatabaseAdapter>,
    ) -> ValidationReport {
        if content.trim().is_empty() {
            return ValidationReport::ok();
        }

        let mut report = ValidationReport::ok();

        // Step 1: Try syntax validation (best-effort for Oracle)
        let syntax = self.validate_syntax(content);
        if !syntax.is_valid {
            // For Oracle, syntax errors from sqlparser may be false positives
            // on valid PL/SQL. Add as warnings instead of hard errors.
            if self.db_type == DatabaseType::Oracle {
                for err in &syntax.errors {
                    report
                        .warnings
                        .push(format!("Parser warning: {}", err.message));
                }
            } else {
                return syntax;
            }
        }

        // Step 2: Extract table references and validate against database
        let refs = extract_table_references(content);
        if refs.is_empty() {
            return report;
        }

        // Fetch available tables and views for referenced schemas
        let mut known_objects: std::collections::HashSet<String> = std::collections::HashSet::new();

        // Collect unique schemas referenced
        let mut schemas_to_check: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        schemas_to_check.insert(schema.to_uppercase());
        for (ref_schema, _) in &refs {
            if let Some(s) = ref_schema {
                schemas_to_check.insert(s.to_uppercase());
            }
        }

        // Fetch tables and views for each schema
        for check_schema in &schemas_to_check {
            if let Ok(tables) = adapter.get_tables(check_schema).await {
                for t in tables {
                    known_objects.insert(format!("{}.{}", check_schema, t.name.to_uppercase()));
                }
            }
            if let Ok(views) = adapter.get_views(check_schema).await {
                for v in views {
                    known_objects.insert(format!("{}.{}", check_schema, v.name.to_uppercase()));
                }
            }
        }

        // Validate each reference
        for (ref_schema, ref_name) in &refs {
            let full_name = if let Some(s) = ref_schema {
                format!("{}.{}", s.to_uppercase(), ref_name.to_uppercase())
            } else {
                format!("{}.{}", schema.to_uppercase(), ref_name.to_uppercase())
            };

            if !known_objects.contains(&full_name) {
                let display = if let Some(s) = ref_schema {
                    format!("{s}.{ref_name}")
                } else {
                    ref_name.clone()
                };
                report.errors.push(ValidationError {
                    line: 0,
                    col: 0,
                    message: format!(
                        "Table or view '{display}' not found or insufficient privileges"
                    ),
                    severity: ErrorSeverity::Reference,
                });
                report.is_valid = false;
            }
        }

        report
    }

    /// Compile SQL to database. Handles transaction semantics per DB type.
    /// Oracle: DDL auto-commits, no rollback possible.
    /// PostgreSQL: DDL is transactional, supports rollback.
    /// MySQL: DDL auto-commits.
    #[allow(dead_code)]
    pub async fn compile_to_db(
        &self,
        sql: &str,
        adapter: &Arc<dyn DatabaseAdapter>,
    ) -> DbResult<()> {
        match self.db_type {
            DatabaseType::PostgreSQL => {
                // PostgreSQL supports transactional DDL
                adapter.execute("BEGIN").await?;
                match adapter.execute(sql).await {
                    Ok(_) => {
                        adapter.execute("COMMIT").await?;
                        Ok(())
                    }
                    Err(e) => {
                        let _ = adapter.execute("ROLLBACK").await;
                        Err(e)
                    }
                }
            }
            DatabaseType::Oracle | DatabaseType::MySQL => {
                // Oracle/MySQL: DDL auto-commits, no transaction wrapping
                adapter.execute(sql).await?;
                Ok(())
            }
        }
    }
}

/// Try to parse line/column from sqlparser error messages
fn parse_error_position(msg: &str) -> (usize, usize) {
    // sqlparser errors look like: "Expected ..., found: ... at Line: 5, Column: 10"
    let mut line = 0;
    let mut col = 0;
    if let Some(pos) = msg.find("Line: ")
        && let Some(num_str) = msg[pos + 6..].split(',').next()
    {
        line = num_str.trim().parse().unwrap_or(0);
    }
    if let Some(pos) = msg.find("Column: ")
        && let Some(num_str) = msg[pos + 8..].split(|c: char| !c.is_ascii_digit()).next()
    {
        col = num_str.trim().parse().unwrap_or(0);
    }
    (line, col)
}

/// Extract table/view references from SQL content.
/// Returns Vec<(Option<schema>, table_name)>.
/// Uses simple keyword-based extraction (not full AST) to handle PL/SQL.
fn extract_table_references(content: &str) -> Vec<(Option<String>, String)> {
    let mut refs = Vec::new();
    let upper = content.to_uppercase();
    let tokens: Vec<&str> = upper.split_whitespace().collect();

    // Keywords after which a table/view name typically follows
    let table_keywords = ["FROM", "JOIN", "INTO", "UPDATE", "TABLE"];

    for (i, token) in tokens.iter().enumerate() {
        let clean = token.trim_end_matches([',', ';', '(']);
        if table_keywords.contains(&clean)
            && let Some(next) = tokens.get(i + 1)
        {
            let name = next
                .trim_matches(|c: char| c == ',' || c == ';' || c == '(' || c == ')' || c == '"');
            // Skip SQL keywords that might follow
            if is_sql_keyword(name) || name.is_empty() {
                continue;
            }
            // Check for schema.table pattern
            if let Some((schema, table)) = name.split_once('.') {
                let table = table.trim_end_matches([',', ';', '(', ')']);
                if !table.is_empty() && !is_sql_keyword(table) {
                    refs.push((Some(schema.to_string()), table.to_string()));
                }
            } else {
                refs.push((None, name.to_string()));
            }
        }
    }

    // Deduplicate
    refs.sort();
    refs.dedup();
    refs
}

fn is_sql_keyword(word: &str) -> bool {
    matches!(
        word,
        "SELECT"
            | "FROM"
            | "WHERE"
            | "INSERT"
            | "INTO"
            | "UPDATE"
            | "DELETE"
            | "SET"
            | "JOIN"
            | "LEFT"
            | "RIGHT"
            | "INNER"
            | "OUTER"
            | "FULL"
            | "CROSS"
            | "ON"
            | "AND"
            | "OR"
            | "NOT"
            | "IN"
            | "IS"
            | "NULL"
            | "LIKE"
            | "BETWEEN"
            | "EXISTS"
            | "AS"
            | "ORDER"
            | "BY"
            | "GROUP"
            | "HAVING"
            | "LIMIT"
            | "OFFSET"
            | "DISTINCT"
            | "UNION"
            | "ALL"
            | "CREATE"
            | "ALTER"
            | "DROP"
            | "TABLE"
            | "INDEX"
            | "VIEW"
            | "BEGIN"
            | "END"
            | "COMMIT"
            | "ROLLBACK"
            | "DECLARE"
            | "CURSOR"
            | "CASE"
            | "WHEN"
            | "THEN"
            | "ELSE"
            | "VALUES"
            | "WITH"
            | "RECURSIVE"
            | "REPLACE"
            | "PACKAGE"
            | "BODY"
            | "FUNCTION"
            | "PROCEDURE"
            | "RETURN"
            | "IF"
            | "LOOP"
            | "FOR"
            | "WHILE"
            | "EXCEPTION"
    )
}
