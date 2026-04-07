use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::error::DbResult;
use crate::core::models::*;

/// A batch of rows streamed from a query.
pub struct QueryBatch {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub done: bool,
}

/// Skip leading whitespace and SQL comments (both `-- line` and `/* block */`,
/// nested supported) and return the byte offset of the first "real" token.
fn skip_leading_noise(sql: &str) -> usize {
    let bytes = sql.as_bytes();
    let mut i = 0;
    loop {
        // Whitespace
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return i;
        }
        // Line comment: -- ... \n
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment: /* ... */  (supports nesting)
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1usize;
            while i + 1 < bytes.len() && depth > 0 {
                if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        return i;
    }
}

/// Return true if the SQL statement, after skipping leading whitespace and
/// comments, starts with `SELECT` or `WITH` — i.e. it's a row-producing query
/// that must go through the driver's `.query()` path rather than `.execute()`.
///
/// This exists because `trim_start().starts_with("SELECT")` is fooled by
/// leading SQL comments (e.g. a `-- note` line above the query), which would
/// otherwise route a SELECT to the DDL/DML branch and trigger driver errors
/// like Oracle's "could not use 'execute' method for select statements".
pub fn is_row_producing_query(sql: &str) -> bool {
    let start = skip_leading_noise(sql);
    let rest = &sql[start..];
    let upper: String = rest
        .chars()
        .take(8) // enough for "SELECT " / "WITH "
        .flat_map(|c| c.to_uppercase())
        .collect();
    upper.starts_with("SELECT") || upper.starts_with("WITH")
}

#[cfg(test)]
mod classifier_tests {
    use super::is_row_producing_query;

    #[test]
    fn plain_select() {
        assert!(is_row_producing_query("SELECT * FROM t"));
        assert!(is_row_producing_query("select * from t"));
    }

    #[test]
    fn plain_with() {
        assert!(is_row_producing_query(
            "WITH x AS (SELECT 1) SELECT * FROM x"
        ));
    }

    #[test]
    fn leading_line_comment() {
        assert!(is_row_producing_query("-- note\nSELECT * FROM t"));
        assert!(is_row_producing_query(
            "-- a\n-- b\n  SELECT * FROM t ORDER BY x DESC"
        ));
    }

    #[test]
    fn leading_block_comment() {
        assert!(is_row_producing_query("/* hello */ SELECT 1"));
        assert!(is_row_producing_query("/* /* nested */ */\nSELECT 1"));
    }

    #[test]
    fn mixed_comments_and_whitespace() {
        assert!(is_row_producing_query(
            "\n  -- c1\n/* c2 */\n  SELECT * FROM t"
        ));
    }

    #[test]
    fn dml_not_row_producing() {
        assert!(!is_row_producing_query("INSERT INTO t VALUES (1)"));
        assert!(!is_row_producing_query("UPDATE t SET x = 1"));
        assert!(!is_row_producing_query("DELETE FROM t"));
        assert!(!is_row_producing_query("-- sneaky\nUPDATE t SET x = 1"));
    }

    #[test]
    fn ddl_not_row_producing() {
        assert!(!is_row_producing_query("CREATE TABLE t (id INT)"));
        assert!(!is_row_producing_query("BEGIN NULL; END;"));
    }
}

#[allow(dead_code)]
#[async_trait]
pub trait DatabaseAdapter: Send + Sync {
    /// Database engine name (e.g., "Oracle", "PostgreSQL", "MySQL")
    fn name(&self) -> &str;

    /// Database type enum variant
    fn db_type(&self) -> DatabaseType;

    /// Fetch all schemas (or databases for MySQL)
    async fn get_schemas(&self) -> DbResult<Vec<Schema>>;

    /// Fetch tables in a schema
    async fn get_tables(&self, schema: &str) -> DbResult<Vec<Table>>;

    /// Fetch views in a schema
    async fn get_views(&self, schema: &str) -> DbResult<Vec<View>>;

    /// Fetch procedures in a schema
    async fn get_procedures(&self, schema: &str) -> DbResult<Vec<Procedure>>;

    /// Fetch functions in a schema
    async fn get_functions(&self, schema: &str) -> DbResult<Vec<Function>>;

    /// Fetch column metadata for a table
    async fn get_columns(&self, schema: &str, table: &str) -> DbResult<Vec<Column>>;

    /// Execute an arbitrary SQL query
    async fn execute(&self, query: &str) -> DbResult<QueryResult>;

    /// Execute a query and stream results in batches via the provided sender.
    /// Default implementation falls back to `execute()` and sends a single batch.
    async fn execute_streaming(
        &self,
        query: &str,
        tx: mpsc::Sender<DbResult<QueryBatch>>,
    ) -> DbResult<()> {
        let result = self.execute(query).await?;
        let _ = tx
            .send(Ok(QueryBatch {
                columns: result.columns,
                rows: result.rows,
                done: true,
            }))
            .await;
        Ok(())
    }

    /// Fetch packages in a schema. Returns empty vec if not supported.
    async fn get_packages(&self, _schema: &str) -> DbResult<Vec<Package>> {
        Ok(vec![])
    }

    /// Fetch package declaration and body. Returns None if not supported.
    async fn get_package_content(
        &self,
        _schema: &str,
        _name: &str,
    ) -> DbResult<Option<PackageContent>> {
        Ok(None)
    }

    /// Fetch materialized views in a schema. Returns empty vec if not supported.
    async fn get_materialized_views(&self, _schema: &str) -> DbResult<Vec<MaterializedView>> {
        Ok(vec![])
    }

    /// Fetch indexes in a schema. Returns empty vec if not supported.
    async fn get_indexes(&self, _schema: &str) -> DbResult<Vec<Index>> {
        Ok(vec![])
    }

    /// Fetch sequences in a schema. Returns empty vec if not supported.
    async fn get_sequences(&self, _schema: &str) -> DbResult<Vec<Sequence>> {
        Ok(vec![])
    }

    /// Fetch types in a schema. Returns empty vec if not supported.
    async fn get_types(&self, _schema: &str) -> DbResult<Vec<DbType>> {
        Ok(vec![])
    }

    /// Fetch triggers in a schema. Returns empty vec if not supported.
    async fn get_triggers(&self, _schema: &str) -> DbResult<Vec<Trigger>> {
        Ok(vec![])
    }

    /// Fetch events in a schema (MySQL). Returns empty vec if not supported.
    async fn get_events(&self, _schema: &str) -> DbResult<Vec<DbEvent>> {
        Ok(vec![])
    }

    /// Fetch type attributes. Returns (columns, rows) as a QueryResult.
    async fn get_type_attributes(&self, _schema: &str, _name: &str) -> DbResult<QueryResult> {
        Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            elapsed: None,
        })
    }

    /// Fetch type methods. Returns (columns, rows) as a QueryResult.
    async fn get_type_methods(&self, _schema: &str, _name: &str) -> DbResult<QueryResult> {
        Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            elapsed: None,
        })
    }

    /// Fetch trigger column info. Returns (columns, rows) as a QueryResult.
    async fn get_trigger_info(&self, _schema: &str, _name: &str) -> DbResult<QueryResult> {
        Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            elapsed: None,
        })
    }

    /// Fetch DDL for a table. Returns empty string if not supported.
    async fn get_table_ddl(&self, _schema: &str, _table: &str) -> DbResult<String> {
        Ok(String::new())
    }

    /// Fetch source code for a stored object. Returns empty string if not supported.
    async fn get_source_code(
        &self,
        _schema: &str,
        _name: &str,
        _obj_type: &str,
    ) -> DbResult<String> {
        Ok(String::new())
    }

    /// Fetch foreign key constraints for a table. Returns empty vec if not supported.
    async fn get_foreign_keys(&self, _schema: &str, _table: &str) -> DbResult<Vec<ForeignKeyInfo>> {
        Ok(vec![])
    }

    /// Compile/validate SQL on the server without executing it.
    /// Returns diagnostics from the server (e.g., Oracle USER_ERRORS, PG PREPARE errors).
    async fn compile_check(&self, _sql: &str) -> DbResult<Vec<CompileDiagnostic>> {
        Ok(vec![])
    }

    /// Resolve the pseudo-columns a PL/SQL function returns when used inside
    /// `TABLE(...)` in a FROM clause — i.e. the attributes of the `TABLE OF
    /// <object_type>` the function returns. `schema` and `package` are
    /// optional for top-level functions. Returns an empty vec if the driver
    /// does not support table functions (Postgres/MySQL).
    async fn get_function_return_columns(
        &self,
        _schema: Option<&str>,
        _package: Option<&str>,
        _function: &str,
    ) -> DbResult<Vec<Column>> {
        Ok(vec![])
    }
}
