use async_trait::async_trait;
use futures_util::TryStreamExt;
use sqlx::mysql::MySqlPool;
use sqlx::{Column as SqlxColumn, Row, ValueRef};
use tokio::sync::mpsc;

use crate::core::DatabaseAdapter;
use crate::core::adapter::QueryBatch;
use crate::core::error::{DbError, DbResult};
use crate::core::models::*;

pub struct MysqlAdapter {
    pool: MySqlPool,
}

/// Extract a column value as a display string. Handles all MySQL types by
/// trying typed getters and falling back to raw bytes as UTF-8 — this covers
/// DECIMAL, DATE, DATETIME, TIMESTAMP, and any other text-representable type.
/// Extract a column value as a display string. Tries typed decoders first
/// (chrono for dates, native for numbers/strings) then falls back to raw bytes.
fn mysql_value_to_string(row: &sqlx::mysql::MySqlRow, idx: usize) -> String {
    let raw = match row.try_get_raw(idx) {
        Ok(r) => r,
        Err(_) => return "NULL".to_string(),
    };

    if raw.is_null() {
        return "NULL".to_string();
    }

    // String: CHAR, VARCHAR, TEXT, ENUM, SET
    if let Ok(v) = row.try_get::<String, _>(idx) {
        return v;
    }
    // Integers: TINYINT..BIGINT (signed + unsigned)
    if let Ok(v) = row.try_get::<i64, _>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<u64, _>(idx) {
        return v.to_string();
    }
    // Floats: FLOAT, DOUBLE
    if let Ok(v) = row.try_get::<f64, _>(idx) {
        return v.to_string();
    }
    // Bool: TINYINT(1)
    if let Ok(v) = row.try_get::<bool, _>(idx) {
        return if v { "1" } else { "0" }.to_string();
    }
    // DATETIME, TIMESTAMP
    if let Ok(v) = row.try_get::<chrono::NaiveDateTime, _>(idx) {
        return v.format("%Y-%m-%d %H:%M:%S").to_string();
    }
    // DATE
    if let Ok(v) = row.try_get::<chrono::NaiveDate, _>(idx) {
        return v.format("%Y-%m-%d").to_string();
    }
    // TIME
    if let Ok(v) = row.try_get::<chrono::NaiveTime, _>(idx) {
        return v.format("%H:%M:%S").to_string();
    }
    // DECIMAL: raw bytes → UTF-8 (MySQL binary protocol sends DECIMAL as text bytes)
    if let Ok(bytes) = <&[u8] as sqlx::Decode<sqlx::MySql>>::decode(raw) {
        return String::from_utf8_lossy(bytes).into_owned();
    }

    "NULL".to_string()
}

impl MysqlAdapter {
    pub async fn connect(connection_string: &str) -> DbResult<Self> {
        let pool = MySqlPool::connect(connection_string)
            .await
            .map_err(|e| DbError::ConnectionFailed(e.to_string()))?;
        Ok(Self { pool })
    }
}

#[async_trait]
impl DatabaseAdapter for MysqlAdapter {
    fn name(&self) -> &str {
        "MySQL"
    }

    async fn get_table_ddl(&self, schema: &str, table: &str) -> DbResult<String> {
        let query = format!("SHOW CREATE TABLE `{schema}`.`{table}`");
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
        let trimmed = query.trim_start().to_uppercase();
        if !trimmed.starts_with("SELECT") && !trimmed.starts_with("WITH") {
            // Use an explicit transaction to guarantee the DML is committed,
            // regardless of the server's autocommit setting.
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let result = sqlx::query(query)
                .execute(&mut *tx)
                .await
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let affected = result.rows_affected();
            tx.commit()
                .await
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            return Ok(QueryResult {
                columns: vec!["Result".to_string()],
                rows: vec![vec![format!(
                    "Statement executed successfully ({affected} row(s) affected)"
                )]],
                elapsed: None,
            });
        }

        let rows = sqlx::query(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        if rows.is_empty() {
            return Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                elapsed: None,
            });
        }

        let columns: Vec<String> = rows[0]
            .columns()
            .iter()
            .map(|c| c.name().to_string())
            .collect();

        let data: Vec<Vec<String>> = rows
            .iter()
            .map(|row| {
                (0..columns.len())
                    .map(|i| mysql_value_to_string(row, i))
                    .collect()
            })
            .collect();

        Ok(QueryResult {
            columns,
            rows: data,
            elapsed: None,
        })
    }

    async fn execute_streaming(
        &self,
        query: &str,
        tx: mpsc::Sender<DbResult<QueryBatch>>,
    ) -> DbResult<()> {
        const BATCH_SIZE: usize = 500;

        // DDL/DML: execute inside an explicit transaction to guarantee commit
        let trimmed = query.trim_start().to_uppercase();
        if !trimmed.starts_with("SELECT") && !trimmed.starts_with("WITH") {
            let mut db_tx = self
                .pool
                .begin()
                .await
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let result = sqlx::query(query)
                .execute(&mut *db_tx)
                .await
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let affected = result.rows_affected();
            db_tx
                .commit()
                .await
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let msg = format!("Statement executed successfully ({affected} row(s) affected)");
            let _ = tx
                .send(Ok(QueryBatch {
                    columns: vec!["Result".to_string()],
                    rows: vec![vec![msg]],
                    done: true,
                }))
                .await;
            return Ok(());
        }

        let mut stream = sqlx::query(query).fetch(&self.pool);
        let mut columns: Option<Vec<String>> = None;
        let mut batch = Vec::with_capacity(BATCH_SIZE);

        loop {
            let row = match stream.try_next().await {
                Ok(Some(row)) => row,
                Ok(None) => break,
                Err(e) => return Err(DbError::QueryFailed(e.to_string())),
            };

            if columns.is_none() {
                columns = Some(row.columns().iter().map(|c| c.name().to_string()).collect());
            }

            let cols = columns.as_ref().map_or(0, |c| c.len());
            let row_data: Vec<String> = (0..cols).map(|i| mysql_value_to_string(&row, i)).collect();
            batch.push(row_data);

            if batch.len() >= BATCH_SIZE {
                let rows = std::mem::replace(&mut batch, Vec::with_capacity(BATCH_SIZE));
                if tx
                    .send(Ok(QueryBatch {
                        columns: columns.clone().unwrap_or_default(),
                        rows,
                        done: false,
                    }))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
        }

        let _ = tx
            .send(Ok(QueryBatch {
                columns: columns.unwrap_or_default(),
                rows: batch,
                done: true,
            }))
            .await;

        Ok(())
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
