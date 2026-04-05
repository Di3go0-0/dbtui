use async_trait::async_trait;
use futures_util::TryStreamExt;
use sqlx::postgres::PgPool;
use sqlx::{Column as SqlxColumn, Row};
use tokio::sync::mpsc;

use crate::core::DatabaseAdapter;
use crate::core::adapter::QueryBatch;
use crate::core::error::{DbError, DbResult};
use crate::core::models::*;

pub struct PostgresAdapter {
    pool: PgPool,
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
            sqlx::query(query)
                .execute(&self.pool)
                .await
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            return Ok(QueryResult {
                columns: vec!["Result".to_string()],
                rows: vec![vec!["Statement executed successfully".to_string()]],
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
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        row.try_get::<String, _>(i)
                            .or_else(|_| row.try_get::<i64, _>(i).map(|v| v.to_string()))
                            .or_else(|_| row.try_get::<f64, _>(i).map(|v| v.to_string()))
                            .or_else(|_| row.try_get::<bool, _>(i).map(|v| v.to_string()))
                            .unwrap_or_else(|_| "NULL".to_string())
                    })
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
                .map(|i| {
                    row.try_get::<String, _>(i)
                        .or_else(|_| row.try_get::<i64, _>(i).map(|v| v.to_string()))
                        .or_else(|_| row.try_get::<f64, _>(i).map(|v| v.to_string()))
                        .or_else(|_| row.try_get::<bool, _>(i).map(|v| v.to_string()))
                        .unwrap_or_else(|_| "NULL".to_string())
                })
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
}
