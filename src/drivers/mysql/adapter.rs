use async_trait::async_trait;
use sqlx::mysql::MySqlPool;
use sqlx::{Column as SqlxColumn, Row};

use crate::core::DatabaseAdapter;
use crate::core::error::{DbError, DbResult};
use crate::core::models::*;

pub struct MysqlAdapter {
    pool: MySqlPool,
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
        let rows = sqlx::query(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        if rows.is_empty() {
            return Ok(QueryResult {
                columns: vec![],
                rows: vec![],
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
                            .unwrap_or_else(|_| "NULL".to_string())
                    })
                    .collect()
            })
            .collect();

        Ok(QueryResult {
            columns,
            rows: data,
        })
    }
}
