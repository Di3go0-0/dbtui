use async_trait::async_trait;
use sqlx::Row;
use sqlx::mysql::{MySqlConnectOptions, MySqlPool};
use tokio::sync::mpsc;

use crate::core::DatabaseAdapter;
use crate::core::adapter::QueryBatch;
use crate::core::error::{DbError, DbResult, friendly_connect_error};
use crate::core::models::*;
use crate::drivers::mysql::exec;
use crate::drivers::sink::{RowSink, empty_result};

pub struct MysqlAdapter {
    pool: MySqlPool,
}

impl MysqlAdapter {
    /// Connect from a saved connection.
    ///
    /// The fields go to the driver one by one instead of being spliced into a
    /// URL, so credentials and database names containing `/ @ : # ? %` reach
    /// the server exactly as typed.
    pub async fn connect_with_config(config: &ConnectionConfig) -> DbResult<Self> {
        let pool = MySqlPool::connect_with(connect_options(config))
            .await
            .map_err(|e| {
                DbError::ConnectionFailed(friendly_connect_error(
                    DatabaseType::MySQL,
                    &e.to_string(),
                ))
            })?;
        Ok(Self { pool })
    }
}

/// Build driver options from a saved connection.
///
/// Mirrors what parsing a URL did: with no database the session starts
/// without a default schema, and an empty username or password is left at the
/// driver's default.
fn connect_options(config: &ConnectionConfig) -> MySqlConnectOptions {
    let mut options = MySqlConnectOptions::new()
        .host(&config.host)
        .port(config.port);
    if !config.username.is_empty() {
        options = options.username(&config.username);
    }
    if !config.password.is_empty() {
        options = options.password(&config.password);
    }
    if let Some(database) = config.database.as_deref().filter(|d| !d.is_empty()) {
        options = options.database(database);
    }
    options
}

/// Backtick-quote an identifier, doubling embedded backticks.
fn quote_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

#[async_trait]
impl DatabaseAdapter for MysqlAdapter {
    fn name(&self) -> &str {
        "MySQL"
    }

    async fn get_table_ddl(&self, schema: &str, table: &str) -> DbResult<String> {
        let query = format!(
            "SHOW CREATE TABLE {}.{}",
            quote_ident(schema),
            quote_ident(table)
        );
        let row: (String, String) = sqlx::query_as(&query)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
        Ok(row.1)
    }

    fn db_type(&self) -> DatabaseType {
        DatabaseType::MySQL
    }

    /// MySQL databases are normalized to schemas for UI consistency.
    async fn get_schemas(&self) -> DbResult<Vec<Schema>> {
        let rows = sqlx::query(
            "SELECT schema_name FROM information_schema.schemata \
             WHERE schema_name NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys') \
             ORDER BY schema_name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| Schema {
                name: r.get::<String, _>(0),
            })
            .collect())
    }

    async fn get_tables(&self, schema: &str) -> DbResult<Vec<Table>> {
        let rows = sqlx::query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = ? AND table_type = 'BASE TABLE' \
             ORDER BY table_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| Table {
                name: r.get::<String, _>(0),
                schema: schema.to_string(),
                privilege: ObjectPrivilege::Unknown,
            })
            .collect())
    }

    async fn get_views(&self, schema: &str) -> DbResult<Vec<View>> {
        let rows = sqlx::query(
            "SELECT table_name FROM information_schema.views \
             WHERE table_schema = ? \
             ORDER BY table_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| View {
                name: r.get::<String, _>(0),
                schema: schema.to_string(),
                valid: true,
                privilege: ObjectPrivilege::Unknown,
            })
            .collect())
    }

    async fn get_procedures(&self, schema: &str) -> DbResult<Vec<Procedure>> {
        let rows = sqlx::query(
            "SELECT routine_name FROM information_schema.routines \
             WHERE routine_schema = ? AND routine_type = 'PROCEDURE' \
             ORDER BY routine_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| Procedure {
                name: r.get::<String, _>(0),
                schema: schema.to_string(),
                valid: true,
                privilege: ObjectPrivilege::Unknown,
            })
            .collect())
    }

    async fn get_functions(&self, schema: &str) -> DbResult<Vec<Function>> {
        let rows = sqlx::query(
            "SELECT routine_name FROM information_schema.routines \
             WHERE routine_schema = ? AND routine_type = 'FUNCTION' \
             ORDER BY routine_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| Function {
                name: r.get::<String, _>(0),
                schema: schema.to_string(),
                valid: true,
                privilege: ObjectPrivilege::Unknown,
            })
            .collect())
    }

    async fn get_indexes(&self, schema: &str) -> DbResult<Vec<Index>> {
        let rows = sqlx::query(
            "SELECT DISTINCT index_name \
             FROM information_schema.statistics \
             WHERE table_schema = ? \
             ORDER BY index_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| Index {
                name: r.get::<String, _>(0),
                schema: schema.to_string(),
            })
            .collect())
    }

    async fn get_triggers(&self, schema: &str) -> DbResult<Vec<Trigger>> {
        let rows = sqlx::query(
            "SELECT trigger_name \
             FROM information_schema.triggers \
             WHERE trigger_schema = ? \
             ORDER BY trigger_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| Trigger {
                name: r.get::<String, _>(0),
                schema: schema.to_string(),
            })
            .collect())
    }

    async fn get_events(&self, schema: &str) -> DbResult<Vec<DbEvent>> {
        let rows = sqlx::query(
            "SELECT event_name \
             FROM information_schema.events \
             WHERE event_schema = ? \
             ORDER BY event_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| DbEvent {
                name: r.get::<String, _>(0),
                schema: schema.to_string(),
            })
            .collect())
    }

    async fn get_columns(&self, schema: &str, table: &str) -> DbResult<Vec<Column>> {
        let rows = sqlx::query(
            "SELECT c.column_name, c.column_type, c.is_nullable, c.column_key \
             FROM information_schema.columns c \
             WHERE c.table_schema = ? AND c.table_name = ? \
             ORDER BY c.ordinal_position",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| {
                let nullable_str: String = r.get::<String, _>(2);
                let key: String = r.get::<String, _>(3);
                Column {
                    name: r.get::<String, _>(0),
                    data_type: r.get::<String, _>(1),
                    nullable: nullable_str == "YES",
                    is_primary_key: key == "PRI",
                }
            })
            .collect())
    }

    async fn execute(&self, query: &str) -> DbResult<QueryResult> {
        let mut result = empty_result();
        exec::run(&self.pool, query, RowSink::Collect(&mut result)).await?;
        Ok(result)
    }

    async fn execute_streaming(
        &self,
        query: &str,
        tx: mpsc::Sender<DbResult<QueryBatch>>,
    ) -> DbResult<()> {
        exec::run(&self.pool, query, RowSink::Stream(&tx)).await
    }

    async fn get_foreign_keys(&self, schema: &str, table: &str) -> DbResult<Vec<ForeignKeyInfo>> {
        let rows = sqlx::query(
            "SELECT constraint_name, column_name, \
                    referenced_table_schema, referenced_table_name, \
                    referenced_column_name \
             FROM information_schema.KEY_COLUMN_USAGE \
             WHERE table_schema = ? \
               AND table_name = ? \
               AND referenced_table_name IS NOT NULL \
             ORDER BY constraint_name, ordinal_position",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| ForeignKeyInfo {
                constraint_name: r.get::<String, _>(0),
                column_name: r.get::<String, _>(1),
                referenced_schema: r.get::<String, _>(2),
                referenced_table: r.get::<String, _>(3),
                referenced_column: r.get::<String, _>(4),
            })
            .collect())
    }

    async fn compile_check(&self, sql: &str) -> DbResult<Vec<CompileDiagnostic>> {
        // MySQL PREPARE requires a string literal, not a direct statement
        // Use a session variable to hold the SQL
        let set_sql = format!("SET @_dbtui_check = '{}'", sql.replace('\'', "''"));
        if let Err(e) = sqlx::query(&set_sql).execute(&self.pool).await {
            return Ok(vec![CompileDiagnostic {
                line: 1,
                col: 1,
                message: e.to_string(),
                severity: "ERROR".to_string(),
            }]);
        }

        let prepare_result = sqlx::query("PREPARE _dbtui_check FROM @_dbtui_check")
            .execute(&self.pool)
            .await;

        match prepare_result {
            Ok(_) => {
                let _ = sqlx::query("DEALLOCATE PREPARE _dbtui_check")
                    .execute(&self.pool)
                    .await;
                Ok(vec![])
            }
            Err(e) => Ok(vec![CompileDiagnostic {
                line: 1,
                col: 1,
                message: e.to_string(),
                severity: "ERROR".to_string(),
            }]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(database: Option<&str>) -> ConnectionConfig {
        ConnectionConfig {
            name: "test".to_string(),
            db_type: DatabaseType::MySQL,
            host: "db.internal".to_string(),
            port: 3307,
            username: "app@corp".to_string(),
            password: "p/a:s@s#w?o%r+d==".to_string(),
            database: database.map(str::to_string),
            group: "Default".to_string(),
        }
    }

    #[test]
    fn options_carry_fields_verbatim() {
        let options = connect_options(&config(Some("my/db?x#1")));
        assert_eq!(options.get_host(), "db.internal");
        assert_eq!(options.get_port(), 3307);
        assert_eq!(options.get_username(), "app@corp");
        assert_eq!(options.get_database(), Some("my/db?x#1"));
    }

    #[test]
    fn no_database_connects_without_one() {
        assert_eq!(connect_options(&config(None)).get_database(), None);
        assert_eq!(connect_options(&config(Some(""))).get_database(), None);
    }

    #[test]
    fn identifiers_double_their_backticks() {
        assert_eq!(quote_ident("orders"), "`orders`");
        assert_eq!(quote_ident("we`ird"), "`we``ird`");
        assert_eq!(
            quote_ident("a`; DROP TABLE t; --"),
            "`a``; DROP TABLE t; --`"
        );
    }
}
