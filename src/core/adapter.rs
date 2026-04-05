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
}
