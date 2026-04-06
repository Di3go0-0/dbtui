//! SQL dialect abstraction.
//!
//! Encapsulates database-specific behavior (identifier casing, schema support,
//! builtin functions, reserved words) behind a single trait. Adding a new
//! database means implementing this trait — no if/else scattered across files.

use sqlparser::dialect::{Dialect, GenericDialect, MySqlDialect, PostgreSqlDialect};

use crate::core::models::DatabaseType;

/// SQL dialect-specific behavior for parsing and semantic analysis.
pub trait SqlDialect: Send + Sync {
    /// The sqlparser Dialect to use for syntax validation.
    fn parser_dialect(&self) -> Box<dyn Dialect>;

    /// Normalize an identifier for case-insensitive comparison.
    /// Oracle folds to UPPER, PostgreSQL/MySQL fold to lower.
    fn normalize_identifier(&self, ident: &str) -> String;

    /// Whether this dialect supports schema-qualified names (`schema.table`).
    /// True for Oracle and PostgreSQL; false for MySQL (database = schema).
    fn has_schemas(&self) -> bool;

    /// Whether this dialect supports packages (Oracle only).
    fn has_packages(&self) -> bool {
        false
    }

    /// Whether sqlparser can reliably parse procedural blocks (BEGIN..END, DECLARE).
    /// False for Oracle PL/SQL — the engine falls back to token-based analysis.
    fn supports_procedural_parsing(&self) -> bool {
        false
    }

    /// Dialect-specific builtin functions for completion.
    fn builtin_functions(&self) -> &[&str];

    /// Dialect-specific reserved words beyond standard SQL.
    fn dialect_keywords(&self) -> &[&str];

    /// The bind variable prefix for this dialect (`:` for Oracle, `$` for PG, `?` for MySQL).
    fn bind_prefix(&self) -> &str;
}

// ---------------------------------------------------------------------------
// Oracle
// ---------------------------------------------------------------------------

pub struct OracleDialect;

impl SqlDialect for OracleDialect {
    fn parser_dialect(&self) -> Box<dyn Dialect> {
        // GenericDialect is the most lenient. sqlparser cannot parse PL/SQL
        // (CONNECT BY, PIVOT, %TYPE, package bodies), so strict dialects
        // produce massive false positives for Oracle.
        Box::new(GenericDialect {})
    }

    fn normalize_identifier(&self, ident: &str) -> String {
        ident.to_uppercase()
    }

    fn has_schemas(&self) -> bool {
        true
    }

    fn has_packages(&self) -> bool {
        true
    }

    fn builtin_functions(&self) -> &[&str] {
        &ORACLE_FUNCTIONS
    }

    fn dialect_keywords(&self) -> &[&str] {
        &ORACLE_KEYWORDS
    }

    fn bind_prefix(&self) -> &str {
        ":"
    }
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

pub struct PostgresDialect;

impl SqlDialect for PostgresDialect {
    fn parser_dialect(&self) -> Box<dyn Dialect> {
        Box::new(PostgreSqlDialect {})
    }

    fn normalize_identifier(&self, ident: &str) -> String {
        ident.to_lowercase()
    }

    fn has_schemas(&self) -> bool {
        true
    }

    fn builtin_functions(&self) -> &[&str] {
        &PG_FUNCTIONS
    }

    fn dialect_keywords(&self) -> &[&str] {
        &PG_KEYWORDS
    }

    fn bind_prefix(&self) -> &str {
        "$"
    }
}

// ---------------------------------------------------------------------------
// MySQL
// ---------------------------------------------------------------------------

pub struct MysqlDialect;

impl SqlDialect for MysqlDialect {
    fn parser_dialect(&self) -> Box<dyn Dialect> {
        Box::new(MySqlDialect {})
    }

    fn normalize_identifier(&self, ident: &str) -> String {
        ident.to_lowercase()
    }

    fn has_schemas(&self) -> bool {
        // MySQL "database" == "schema" — the UI normalizes this,
        // but schema-qualified completion is not typical in MySQL workflows.
        false
    }

    fn builtin_functions(&self) -> &[&str] {
        &MYSQL_FUNCTIONS
    }

    fn dialect_keywords(&self) -> &[&str] {
        &MYSQL_KEYWORDS
    }

    fn bind_prefix(&self) -> &str {
        "?"
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Create the appropriate SqlDialect for a DatabaseType.
pub fn dialect_for(db_type: DatabaseType) -> Box<dyn SqlDialect> {
    match db_type {
        DatabaseType::Oracle => Box::new(OracleDialect),
        DatabaseType::PostgreSQL => Box::new(PostgresDialect),
        DatabaseType::MySQL => Box::new(MysqlDialect),
    }
}

// ---------------------------------------------------------------------------
// Keyword and function lists
// ---------------------------------------------------------------------------

const ORACLE_FUNCTIONS: [&str; 42] = [
    // Null handling
    "NVL",
    "NVL2",
    "DECODE",
    "COALESCE",
    // Conversion
    "TO_CHAR",
    "TO_DATE",
    "TO_NUMBER",
    "TO_TIMESTAMP",
    "TO_CLOB",
    "CAST",
    // String
    "SUBSTR",
    "INSTR",
    "LENGTH",
    "TRIM",
    "LTRIM",
    "RTRIM",
    "UPPER",
    "LOWER",
    "INITCAP",
    "LPAD",
    "RPAD",
    "REPLACE",
    "TRANSLATE",
    "CONCAT",
    // Numeric
    "ROUND",
    "TRUNC",
    "MOD",
    "ABS",
    "CEIL",
    "FLOOR",
    // Date
    "SYSDATE",
    "SYSTIMESTAMP",
    "ADD_MONTHS",
    "MONTHS_BETWEEN",
    // Aggregate / analytic
    "LISTAGG",
    "ROW_NUMBER",
    "RANK",
    "DENSE_RANK",
    // Regex
    "REGEXP_LIKE",
    "REGEXP_REPLACE",
    "REGEXP_SUBSTR",
    // Pseudo-columns
    "ROWNUM",
];

const ORACLE_KEYWORDS: [&str; 20] = [
    // Hierarchical
    "CONNECT",
    "PRIOR",
    "START",
    "LEVEL",
    "NOCYCLE",
    // Oracle-specific
    "ROWID",
    "MINUS",
    "PACKAGE",
    "BODY",
    "SYNONYM",
    "TABLESPACE",
    // PL/SQL
    "PIPELINED",
    "DETERMINISTIC",
    "RESULT_CACHE",
    "PARALLEL_ENABLE",
    "AUTHID",
    "DEFINER",
    "CURRENT_USER",
    "AUTONOMOUS_TRANSACTION",
    "SERIALLY_REUSABLE",
];

const PG_FUNCTIONS: [&str; 22] = [
    "COALESCE",
    "NULLIF",
    "GREATEST",
    "LEAST",
    "NOW",
    "CURRENT_DATE",
    "CURRENT_TIMESTAMP",
    "EXTRACT",
    "AGE",
    "STRING_AGG",
    "ARRAY_AGG",
    "JSON_BUILD_OBJECT",
    "JSONB_AGG",
    "GENERATE_SERIES",
    "PG_SLEEP",
    "TO_TIMESTAMP",
    "TO_CHAR",
    "TO_NUMBER",
    "ROW_NUMBER",
    "RANK",
    "DENSE_RANK",
    "LAG",
];

const PG_KEYWORDS: [&str; 8] = [
    "ILIKE",
    "RETURNING",
    "LATERAL",
    "MATERIALIZED",
    "CONCURRENTLY",
    "SERIAL",
    "BIGSERIAL",
    "BOOLEAN",
];

const MYSQL_FUNCTIONS: [&str; 17] = [
    "IFNULL",
    "IF",
    "CONCAT_WS",
    "GROUP_CONCAT",
    "DATE_FORMAT",
    "STR_TO_DATE",
    "NOW",
    "CURDATE",
    "CURTIME",
    "JSON_EXTRACT",
    "JSON_OBJECT",
    "JSON_ARRAY",
    "UUID",
    "LAST_INSERT_ID",
    "ROW_NUMBER",
    "RANK",
    "DENSE_RANK",
];

const MYSQL_KEYWORDS: [&str; 8] = [
    "AUTO_INCREMENT",
    "ENGINE",
    "CHARSET",
    "COLLATE",
    "UNSIGNED",
    "ENUM",
    "SHOW",
    "DESCRIBE",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oracle_normalizes_to_uppercase() {
        let d = OracleDialect;
        assert_eq!(d.normalize_identifier("employees"), "EMPLOYEES");
    }

    #[test]
    fn postgres_normalizes_to_lowercase() {
        let d = PostgresDialect;
        assert_eq!(d.normalize_identifier("EMPLOYEES"), "employees");
    }

    #[test]
    fn mysql_normalizes_to_lowercase() {
        let d = MysqlDialect;
        assert_eq!(d.normalize_identifier("Employees"), "employees");
    }

    #[test]
    fn dialect_for_factory() {
        let d = dialect_for(DatabaseType::Oracle);
        assert!(d.has_packages());
        assert!(d.has_schemas());

        let d = dialect_for(DatabaseType::PostgreSQL);
        assert!(!d.has_packages());
        assert!(d.has_schemas());

        let d = dialect_for(DatabaseType::MySQL);
        assert!(!d.has_packages());
        assert!(!d.has_schemas());
    }
}
