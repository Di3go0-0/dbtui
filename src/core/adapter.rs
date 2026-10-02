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
///
/// The offset is always a char boundary: only ASCII bytes are ever stepped
/// over one at a time outside a comment, and a block comment that is never
/// closed swallows the rest of the text instead of stopping mid-character.
pub fn skip_leading_noise(sql: &str) -> usize {
    let bytes = sql.as_bytes();
    let mut i = 0;
    loop {
        // Whitespace
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return bytes.len();
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
            if depth > 0 {
                // Unterminated comment: nothing but noise is left.
                return bytes.len();
            }
            continue;
        }
        return i;
    }
}

/// First keyword of a statement, upper-cased.
///
/// Leading whitespace, comments and opening parentheses are skipped, so
/// `(SELECT 1) UNION (SELECT 2)` reports `SELECT`. Returns an empty string
/// when the statement does not start with a word.
pub fn leading_keyword(sql: &str) -> String {
    let mut rest = &sql[skip_leading_noise(sql)..];
    while let Some(inner) = rest.strip_prefix('(') {
        rest = &inner[skip_leading_noise(inner)..];
    }
    rest.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Return true if the SQL statement, after skipping leading whitespace,
/// comments and opening parentheses, starts with `SELECT` or `WITH` — i.e.
/// it is a row-producing query in every supported dialect.
///
/// This exists because `trim_start().starts_with("SELECT")` is fooled by
/// leading SQL comments (e.g. a `-- note` line above the query), which would
/// otherwise route a SELECT to the DDL/DML branch and trigger driver errors
/// like Oracle's "could not use 'execute' method for select statements".
///
/// Engine-specific statements that also return rows (`SHOW`, `EXPLAIN`,
/// `EXEC`, ...) are classified by `drivers::statement::is_row_producing`.
pub fn is_row_producing_query(sql: &str) -> bool {
    matches!(leading_keyword(sql).as_str(), "SELECT" | "WITH")
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

    /// Fetch the databases on this server.
    ///
    /// An empty vec means the engine has no catalog level above schemas, and
    /// the sidebar keeps its Connection → Schema shape. SQL Server is the only
    /// driver that reports catalogs; there, every `schema` argument below is
    /// qualified as `database.schema` so one connection can reach them all.
    async fn get_catalogs(&self) -> DbResult<Vec<Catalog>> {
        Ok(vec![])
    }

    /// Fetch the schemas inside one catalog. Defaults to the connection's own
    /// schemas, which is correct for every driver without a catalog level.
    async fn get_schemas_in(&self, _catalog: &str) -> DbResult<Vec<Schema>> {
        self.get_schemas().await
    }

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

    /// Execute a query with the session pointed at `schema`, streaming results.
    ///
    /// Only engines with a session-level default schema can honour this:
    /// PostgreSQL (`search_path`) and Oracle (`CURRENT_SCHEMA`). MySQL's
    /// schema *is* the connection's database, and SQL Server resolves
    /// unqualified names through the login's default schema — a user property,
    /// not a session setting — so both ignore it and the selection stays a
    /// client-side context. The default implementation drops the schema.
    async fn execute_streaming_in_schema(
        &self,
        query: &str,
        _schema: Option<&str>,
        tx: mpsc::Sender<DbResult<QueryBatch>>,
    ) -> DbResult<()> {
        self.execute_streaming(query, tx).await
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

#[cfg(test)]
mod classifier_tests {
    use super::{is_row_producing_query, leading_keyword, skip_leading_noise};

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

    #[test]
    fn parenthesised_select() {
        assert!(is_row_producing_query("(SELECT 1) UNION (SELECT 2)"));
        assert!(is_row_producing_query("( /* a */ ( select 1 ) )"));
    }

    #[test]
    fn keyword_must_be_a_whole_word() {
        assert!(!is_row_producing_query("WITHDRAW 5"));
        assert!(!is_row_producing_query("SELECTED"));
        assert!(is_row_producing_query("SELECT\n1"));
        assert!(is_row_producing_query("select*from t"));
    }

    #[test]
    fn unterminated_block_comment_with_multibyte_tail_does_not_panic() {
        // The scan used to stop one byte short of the end, inside the `é`.
        assert_eq!(skip_leading_noise("/* café"), "/* café".len());
        assert!(!is_row_producing_query("/* café"));
        assert!(!is_row_producing_query("/* ñ"));
        assert!(!is_row_producing_query("/*"));
        assert!(!is_row_producing_query("/* a /* b */ é"));
    }

    #[test]
    fn multibyte_text_around_the_statement() {
        assert!(is_row_producing_query("/* café */ SELECT 'ñ'"));
        assert!(is_row_producing_query("-- año\nSELECT 1"));
        assert_eq!(leading_keyword("é SELECT"), "");
    }

    #[test]
    fn leading_keyword_reports_the_first_word() {
        assert_eq!(leading_keyword("  explain select 1"), "EXPLAIN");
        assert_eq!(leading_keyword("-- c\n((values (1)))"), "VALUES");
        assert_eq!(leading_keyword(""), "");
        assert_eq!(leading_keyword("   "), "");
    }
}
