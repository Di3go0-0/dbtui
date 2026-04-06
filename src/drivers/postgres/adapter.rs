use async_trait::async_trait;
use futures_util::TryStreamExt;
use sqlx::postgres::PgPool;
use sqlx::{Column as SqlxColumn, Row, TypeInfo, ValueRef};
use tokio::sync::mpsc;

use crate::core::DatabaseAdapter;
use crate::core::adapter::QueryBatch;
use crate::core::error::{DbError, DbResult};
use crate::core::models::*;

pub struct PostgresAdapter {
    pool: PgPool,
}

/// Extract a column value as a display string. Covers PG's type zoo by trying
/// typed getters and falling back to raw bytes → UTF-8 for types like NUMERIC,
/// MONEY, UUID, JSONB, INET, arrays, etc.
fn pg_value_to_string(row: &sqlx::postgres::PgRow, idx: usize) -> String {
    if let Ok(raw) = row.try_get_raw(idx)
        && raw.is_null()
    {
        return "NULL".to_string();
    }
    if let Ok(v) = row.try_get::<String, _>(idx) {
        return v;
    }
    if let Ok(v) = row.try_get::<i64, _>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<i32, _>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<i16, _>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<f64, _>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<f32, _>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<bool, _>(idx) {
        return v.to_string();
    }
    // TIMESTAMP / TIMESTAMPTZ
    if let Ok(v) = row.try_get::<chrono::NaiveDateTime, _>(idx) {
        return v.format("%Y-%m-%d %H:%M:%S").to_string();
    }
    if let Ok(v) = row.try_get::<chrono::DateTime<chrono::Utc>, _>(idx) {
        return v.format("%Y-%m-%d %H:%M:%S%z").to_string();
    }
    // DATE
    if let Ok(v) = row.try_get::<chrono::NaiveDate, _>(idx) {
        return v.format("%Y-%m-%d").to_string();
    }
    // TIME
    if let Ok(v) = row.try_get::<chrono::NaiveTime, _>(idx) {
        return v.format("%H:%M:%S").to_string();
    }
    // BYTEA: show as hex
    if let Ok(raw) = row.try_get_raw(idx)
        && raw.type_info().name() == "BYTEA"
        && let Ok(bytes) = row.try_get::<Vec<u8>, _>(idx)
    {
        return if bytes.len() <= 32 {
            format!("\\x{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
        } else {
            format!(
                "\\x{}...",
                bytes[..32].iter().map(|b| format!("{b:02x}")).collect::<String>()
            )
        };
    }
    // Last resort: raw bytes as UTF-8 (NUMERIC, UUID, INET, etc.)
    if let Ok(bytes) = row.try_get::<Vec<u8>, _>(idx)
        && let Ok(s) = String::from_utf8(bytes)
    {
        return s;
    }
    "NULL".to_string()
}

impl PostgresAdapter {
    pub async fn connect(connection_string: &str) -> DbResult<Self> {
        let pool = PgPool::connect(connection_string)
            .await
            .map_err(|e| DbError::ConnectionFailed(e.to_string()))?;
        Ok(Self { pool })
    }
}

#[async_trait]
impl DatabaseAdapter for PostgresAdapter {
    fn name(&self) -> &str {
        "PostgreSQL"
    }

    async fn get_table_ddl(&self, schema: &str, table: &str) -> DbResult<String> {
        // Build DDL from information_schema columns + constraints
        let col_rows: Vec<(String, String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT column_name, data_type, is_nullable, \
                    COALESCE(character_maximum_length::text, \
                             numeric_precision::text || COALESCE(',' || numeric_scale::text, ''), ''), \
                    column_default \
             FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 \
             ORDER BY ordinal_position",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        if col_rows.is_empty() {
            return Ok(format!("-- No columns found for {schema}.{table}"));
        }

        let mut ddl = format!("CREATE TABLE {schema}.{table} (\n");
        for (i, (name, dtype, nullable, size, default)) in col_rows.iter().enumerate() {
            let type_str = if size.is_empty() {
                dtype.clone()
            } else {
                format!("{dtype}({size})")
            };
            let null_str = if nullable == "NO" { " NOT NULL" } else { "" };
            let default_str = match default {
                Some(d) => format!(" DEFAULT {d}"),
                None => String::new(),
            };
            let comma = if i + 1 < col_rows.len() { "," } else { "" };
            ddl.push_str(&format!(
                "    {name} {type_str}{null_str}{default_str}{comma}\n"
            ));
        }

        // Primary key
        let pk_row: Option<(String,)> = sqlx::query_as(
            "SELECT string_agg(kcu.column_name, ', ' ORDER BY kcu.ordinal_position) \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
             WHERE tc.table_schema = $1 AND tc.table_name = $2 AND tc.constraint_type = 'PRIMARY KEY' \
             GROUP BY tc.constraint_name",
        )
        .bind(schema)
        .bind(table)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        if let Some((pk_cols,)) = pk_row {
            // Need comma after last column
            if !ddl.ends_with(",\n") {
                // Replace last \n with ,\n
                if ddl.ends_with('\n') {
                    ddl.pop();
                    ddl.push_str(",\n");
                }
            }
            ddl.push_str(&format!("    PRIMARY KEY ({pk_cols})\n"));
        }

        ddl.push_str(");");
        Ok(ddl)
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
        let rows = sqlx::query(
            "SELECT c.column_name, c.data_type, c.is_nullable, \
             CASE WHEN tc.constraint_type = 'PRIMARY KEY' THEN true ELSE false END as is_pk \
             FROM information_schema.columns c \
             LEFT JOIN information_schema.key_column_usage kcu \
               ON c.table_schema = kcu.table_schema \
               AND c.table_name = kcu.table_name \
               AND c.column_name = kcu.column_name \
             LEFT JOIN information_schema.table_constraints tc \
               ON kcu.constraint_name = tc.constraint_name \
               AND kcu.table_schema = tc.table_schema \
               AND tc.constraint_type = 'PRIMARY KEY' \
             WHERE c.table_schema = $1 AND c.table_name = $2 \
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
        let trimmed = query.trim_start().to_uppercase();
        if !trimmed.starts_with("SELECT") && !trimmed.starts_with("WITH") {
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
                    .map(|i| pg_value_to_string(row, i))
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

        // DDL/DML: execute and return a single "success" batch
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

        // Begin transaction so PostgreSQL uses a server-side cursor (streams rows)
        // Without this, PG fetches ALL rows before returning any.
        let mut db_tx = self
            .pool
            .begin()
            .await
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
        let mut stream = sqlx::query(query).fetch(&mut *db_tx);
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
            let row_data: Vec<String> = (0..cols)
                .map(|i| pg_value_to_string(&row, i))
                .collect();
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

        // Drop stream before rolling back transaction
        drop(stream);

        // Send remaining rows (or empty final batch)
        let _ = tx
            .send(Ok(QueryBatch {
                columns: columns.unwrap_or_default(),
                rows: batch,
                done: true,
            }))
            .await;

        // Rollback read-only transaction
        let _ = db_tx.rollback().await;

        Ok(())
    }

    async fn get_foreign_keys(&self, schema: &str, table: &str) -> DbResult<Vec<ForeignKeyInfo>> {
        let rows = sqlx::query(
            "SELECT kcu.constraint_name, kcu.column_name, \
                    ccu.table_schema AS ref_schema, \
                    ccu.table_name AS ref_table, \
                    ccu.column_name AS ref_column \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name \
              AND tc.table_schema = kcu.table_schema \
             JOIN information_schema.constraint_column_usage ccu \
               ON tc.constraint_name = ccu.constraint_name \
              AND tc.table_schema = ccu.table_schema \
             WHERE tc.table_schema = $1 \
               AND tc.table_name = $2 \
               AND tc.constraint_type = 'FOREIGN KEY' \
             ORDER BY kcu.constraint_name, kcu.ordinal_position",
        )
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
