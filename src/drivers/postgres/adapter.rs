use async_trait::async_trait;
use sqlx::Row;
use sqlx::postgres::{PgConnectOptions, PgPool};
use tokio::sync::mpsc;

use crate::core::DatabaseAdapter;
use crate::core::adapter::QueryBatch;
use crate::core::error::{DbError, DbResult, friendly_connect_error};
use crate::core::models::*;
use crate::drivers::postgres::ddl::{
    COLUMNS_SQL, DDL_COLUMNS_SQL, DdlColumn, FOREIGN_KEYS_SQL, PRIMARY_KEY_SQL, build_table_ddl,
};
use crate::drivers::postgres::exec;
use crate::drivers::sink::{RowSink, empty_result};

pub struct PostgresAdapter {
    pool: PgPool,
}

impl PostgresAdapter {
    /// Connect from a `postgres://` URL (the `DBTUI_POSTGRES_URL` shortcut).
    pub async fn connect(connection_string: &str) -> DbResult<Self> {
        let pool = PgPool::connect(connection_string)
            .await
            .map_err(connect_error)?;
        Ok(Self { pool })
    }

    /// Connect from a saved connection.
    ///
    /// The fields go to the driver one by one instead of being spliced into a
    /// URL, so credentials and database names containing `/ @ : # ? %` reach
    /// the server exactly as typed.
    pub async fn connect_with_config(config: &ConnectionConfig) -> DbResult<Self> {
        let pool = PgPool::connect_with(connect_options(config))
            .await
            .map_err(connect_error)?;
        Ok(Self { pool })
    }
}

fn connect_error(err: sqlx::Error) -> DbError {
    DbError::ConnectionFailed(friendly_connect_error(
        DatabaseType::PostgreSQL,
        &err.to_string(),
    ))
}

/// Build driver options from a saved connection.
///
/// Mirrors what parsing a URL did: the database defaults to `postgres`, and
/// an empty username or password is left unset so the driver's own defaults
/// (`PGUSER`, `PGPASSWORD`) still apply. The options start without a pgpass
/// lookup because sqlx performs it against its *default* host, which could
/// hand another server's password to this connection.
fn connect_options(config: &ConnectionConfig) -> PgConnectOptions {
    let database = config
        .database
        .as_deref()
        .filter(|d| !d.is_empty())
        .unwrap_or("postgres");
    let mut options = PgConnectOptions::new_without_pgpass()
        .host(&config.host)
        .port(config.port)
        .database(database);
    if !config.username.is_empty() {
        options = options.username(&config.username);
    }
    if !config.password.is_empty() {
        options = options.password(&config.password);
    }
    options
}

#[async_trait]
impl DatabaseAdapter for PostgresAdapter {
    fn name(&self) -> &str {
        "PostgreSQL"
    }

    async fn get_table_ddl(&self, schema: &str, table: &str) -> DbResult<String> {
        let rows: Vec<(String, String, bool, Option<String>)> = sqlx::query_as(DDL_COLUMNS_SQL)
            .bind(schema)
            .bind(table)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        if rows.is_empty() {
            return Ok(format!("-- No columns found for {schema}.{table}"));
        }

        let columns: Vec<DdlColumn> = rows
            .into_iter()
            .map(|(name, data_type, not_null, default)| DdlColumn {
                name,
                data_type,
                not_null,
                default,
            })
            .collect();

        let primary_key: Vec<(String,)> = sqlx::query_as(PRIMARY_KEY_SQL)
            .bind(schema)
            .bind(table)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
        let primary_key: Vec<String> = primary_key.into_iter().map(|(name,)| name).collect();

        Ok(build_table_ddl(schema, table, &columns, &primary_key))
    }

    fn db_type(&self) -> DatabaseType {
        DatabaseType::PostgreSQL
    }

    async fn get_schemas(&self) -> DbResult<Vec<Schema>> {
        let rows = sqlx::query(
            "SELECT schema_name FROM information_schema.schemata \
             WHERE schema_name NOT IN ('pg_catalog', 'information_schema', 'pg_toast') \
             ORDER BY schema_name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| Schema {
                name: r.get("schema_name"),
            })
            .collect())
    }

    async fn get_tables(&self, schema: &str) -> DbResult<Vec<Table>> {
        let rows = sqlx::query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = $1 AND table_type = 'BASE TABLE' \
             ORDER BY table_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| Table {
                name: r.get("table_name"),
                schema: schema.to_string(),
                privilege: ObjectPrivilege::Full,
            })
            .collect())
    }

    async fn get_views(&self, schema: &str) -> DbResult<Vec<View>> {
        let rows = sqlx::query(
            "SELECT table_name FROM information_schema.views \
             WHERE table_schema = $1 \
             ORDER BY table_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| View {
                name: r.get("table_name"),
                schema: schema.to_string(),
                valid: true,
                privilege: ObjectPrivilege::Full,
            })
            .collect())
    }

    async fn get_procedures(&self, schema: &str) -> DbResult<Vec<Procedure>> {
        let rows = sqlx::query(
            "SELECT routine_name FROM information_schema.routines \
             WHERE routine_schema = $1 AND routine_type = 'PROCEDURE' \
             ORDER BY routine_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| Procedure {
                name: r.get("routine_name"),
                schema: schema.to_string(),
                valid: true,
                privilege: ObjectPrivilege::Full,
            })
            .collect())
    }

    async fn get_functions(&self, schema: &str) -> DbResult<Vec<Function>> {
        let rows = sqlx::query(
            "SELECT routine_name FROM information_schema.routines \
             WHERE routine_schema = $1 AND routine_type = 'FUNCTION' \
             ORDER BY routine_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| Function {
                name: r.get("routine_name"),
                schema: schema.to_string(),
                valid: true,
                privilege: ObjectPrivilege::Full,
            })
            .collect())
    }

    async fn get_materialized_views(&self, schema: &str) -> DbResult<Vec<MaterializedView>> {
        let rows = sqlx::query(
            "SELECT matviewname FROM pg_matviews \
             WHERE schemaname = $1 ORDER BY matviewname",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| MaterializedView {
                name: r.get("matviewname"),
                schema: schema.to_string(),
                valid: true,
                privilege: ObjectPrivilege::Full,
            })
            .collect())
    }

    async fn get_indexes(&self, schema: &str) -> DbResult<Vec<Index>> {
        let rows = sqlx::query(
            "SELECT indexname FROM pg_indexes \
             WHERE schemaname = $1 ORDER BY indexname",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| Index {
                name: r.get("indexname"),
                schema: schema.to_string(),
            })
            .collect())
    }

    async fn get_sequences(&self, schema: &str) -> DbResult<Vec<Sequence>> {
        let rows = sqlx::query(
            "SELECT sequence_name FROM information_schema.sequences \
             WHERE sequence_schema = $1 ORDER BY sequence_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| Sequence {
                name: r.get("sequence_name"),
                schema: schema.to_string(),
            })
            .collect())
    }

    async fn get_triggers(&self, schema: &str) -> DbResult<Vec<Trigger>> {
        let rows = sqlx::query(
            "SELECT DISTINCT trigger_name \
             FROM information_schema.triggers \
             WHERE trigger_schema = $1 ORDER BY trigger_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| Trigger {
                name: r.get("trigger_name"),
                schema: schema.to_string(),
            })
            .collect())
    }

    async fn get_columns(&self, schema: &str, table: &str) -> DbResult<Vec<Column>> {
        let rows = sqlx::query(COLUMNS_SQL)
            .bind(schema)
            .bind(table)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| {
                let nullable_str: String = r.get("is_nullable");
                Column {
                    name: r.get("column_name"),
                    data_type: r.get("data_type"),
                    nullable: nullable_str == "YES",
                    is_primary_key: r.get::<bool, _>("is_pk"),
                }
            })
            .collect())
    }

    async fn execute(&self, query: &str) -> DbResult<QueryResult> {
        let mut result = empty_result();
        exec::run(&self.pool, query, None, RowSink::Collect(&mut result)).await?;
        Ok(result)
    }

    async fn execute_streaming(
        &self,
        query: &str,
        tx: mpsc::Sender<DbResult<QueryBatch>>,
    ) -> DbResult<()> {
        exec::run(&self.pool, query, None, RowSink::Stream(&tx)).await
    }

    async fn execute_streaming_in_schema(
        &self,
        query: &str,
        schema: Option<&str>,
        tx: mpsc::Sender<DbResult<QueryBatch>>,
    ) -> DbResult<()> {
        exec::run(&self.pool, query, schema, RowSink::Stream(&tx)).await
    }

    async fn get_foreign_keys(&self, schema: &str, table: &str) -> DbResult<Vec<ForeignKeyInfo>> {
        let rows = sqlx::query(FOREIGN_KEYS_SQL)
            .bind(schema)
            .bind(table)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| ForeignKeyInfo {
                constraint_name: r.get("constraint_name"),
                column_name: r.get("column_name"),
                referenced_schema: r.get("ref_schema"),
                referenced_table: r.get("ref_table"),
                referenced_column: r.get("ref_column"),
            })
            .collect())
    }

    async fn compile_check(&self, sql: &str) -> DbResult<Vec<CompileDiagnostic>> {
        // Use PREPARE/DEALLOCATE in a transaction that gets rolled back
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        let prepare_sql = format!("PREPARE _dbtui_check AS {sql}");
        let result = sqlx::query(&prepare_sql).execute(&mut *tx).await;

        match result {
            Ok(_) => {
                let _ = sqlx::query("DEALLOCATE _dbtui_check")
                    .execute(&mut *tx)
                    .await;
                let _ = tx.rollback().await;
                Ok(vec![])
            }
            Err(e) => {
                let _ = tx.rollback().await;
                let msg = e.to_string();
                Ok(vec![CompileDiagnostic {
                    line: 1,
                    col: 1,
                    message: msg,
                    severity: "ERROR".to_string(),
                }])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(password: &str, database: Option<&str>) -> ConnectionConfig {
        ConnectionConfig {
            name: "test".to_string(),
            db_type: DatabaseType::PostgreSQL,
            host: "db.internal".to_string(),
            port: 6543,
            username: "app@corp".to_string(),
            password: password.to_string(),
            database: database.map(str::to_string),
            group: "Default".to_string(),
        }
    }

    #[test]
    fn options_carry_fields_verbatim() {
        // A base64 password with every character a URL would mis-parse.
        let options = connect_options(&config("p/a:s@s#w?o%r+d==", Some("my/db?x#1")));
        assert_eq!(options.get_host(), "db.internal");
        assert_eq!(options.get_port(), 6543);
        assert_eq!(options.get_username(), "app@corp");
        assert_eq!(options.get_database(), Some("my/db?x#1"));
    }

    #[test]
    fn database_defaults_to_postgres() {
        assert_eq!(
            connect_options(&config("x", None)).get_database(),
            Some("postgres")
        );
        assert_eq!(
            connect_options(&config("x", Some(""))).get_database(),
            Some("postgres")
        );
    }
}
