use std::collections::HashMap;

use async_trait::async_trait;
use sqlx::Row;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use tokio::sync::{Mutex, mpsc};

use crate::core::DatabaseAdapter;
use crate::core::adapter::QueryBatch;
use crate::core::error::{DbError, DbResult, friendly_connect_error};
use crate::core::models::*;
use crate::drivers::postgres::ddl::{
    COLUMNS_SQL, DDL_COLUMNS_SQL, DdlColumn, FOREIGN_KEYS_SQL, PRIMARY_KEY_SQL, build_table_ddl,
};
use crate::drivers::postgres::exec;
use crate::drivers::sink::{RowSink, empty_result};

/// A PostgreSQL server reached through one saved connection.
///
/// A PostgreSQL session sees one database, so the explorer lists the
/// server's databases as a catalog level and names every schema
/// `database.schema`. Each database other than the one the connection
/// opened gets its own pool the first time something in it is read.
pub struct PostgresAdapter {
    /// The connection's own database.
    pool: PgPool,
    /// How `pool` was opened; other databases are opened the same way.
    options: PgConnectOptions,
    /// Name of the database `pool` is connected to.
    database: String,
    /// Pools of the other databases, opened on demand.
    pools: Mutex<HashMap<String, PgPool>>,
}

/// Connections kept per extra database. Small: these serve the explorer
/// and the odd script, not the main workload.
const EXTRA_POOL_SIZE: u32 = 4;

impl PostgresAdapter {
    /// Connect from a `postgres://` URL (the `DBTUI_POSTGRES_URL` shortcut).
    pub async fn connect(connection_string: &str) -> DbResult<Self> {
        let options: PgConnectOptions = connection_string
            .parse()
            .map_err(|e: sqlx::Error| connect_error(e))?;
        Self::open(options).await
    }

    /// Connect from a saved connection.
    ///
    /// The fields go to the driver one by one instead of being spliced into a
    /// URL, so credentials and database names containing `/ @ : # ? %` reach
    /// the server exactly as typed.
    pub async fn connect_with_config(config: &ConnectionConfig) -> DbResult<Self> {
        Self::open(connect_options(config)).await
    }

    async fn open(options: PgConnectOptions) -> DbResult<Self> {
        let pool = PgPool::connect_with(options.clone())
            .await
            .map_err(connect_error)?;
        // The server reports the database it actually opened, which covers
        // a URL that left it to the defaults.
        let database: (String,) = sqlx::query_as("SELECT current_database()::text")
            .fetch_one(&pool)
            .await
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
        Ok(Self {
            pool,
            options,
            database: database.0,
            pools: Mutex::new(HashMap::new()),
        })
    }

    /// The pool serving `schema` and the schema's bare name.
    ///
    /// `database.schema` goes to that database's pool, opened on first use;
    /// a bare name, or the connection's own database, uses the main pool.
    async fn pool_for(&self, schema: &str) -> DbResult<(PgPool, String)> {
        let (catalog, bare) = split_qualified(schema);
        let Some(catalog) = catalog.filter(|c| *c != self.database) else {
            return Ok((self.pool.clone(), bare.to_string()));
        };
        let mut pools = self.pools.lock().await;
        if let Some(pool) = pools.get(catalog) {
            return Ok((pool.clone(), bare.to_string()));
        }
        let pool = PgPoolOptions::new()
            .max_connections(EXTRA_POOL_SIZE)
            .connect_with(self.options.clone().database(catalog))
            .await
            .map_err(connect_error)?;
        pools.insert(catalog.to_string(), pool.clone());
        Ok((pool, bare.to_string()))
    }

    /// Name of `schema` in the form the explorer uses: qualified with its
    /// database whenever the schema asked about was.
    fn qualify_like(schema: &str, name: &str) -> String {
        match split_qualified(schema) {
            (Some(catalog), _) => format!("{catalog}.{name}"),
            (None, _) => name.to_string(),
        }
    }
}

/// Every schema of the database behind `pool`, system ones aside.
///
/// pg_namespace lists them all; the information_schema view hides the
/// ones the user has no privilege on, which made other people's schemas
/// vanish from the tree.
async fn schemas_of(pool: &PgPool) -> DbResult<Vec<Schema>> {
    let rows = sqlx::query(
        "SELECT nspname::text AS schema_name FROM pg_catalog.pg_namespace \
         WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema' \
         ORDER BY nspname",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::QueryFailed(e.to_string()))?;

    Ok(rows
        .iter()
        .map(|r| Schema {
            name: r.get("schema_name"),
        })
        .collect())
}

/// Split a `database.schema` argument into its parts.
///
/// The split is on the *last* dot: a database name may contain dots, and
/// the explorer appends the schema after it. A bare name has no database.
fn split_qualified(schema: &str) -> (Option<&str>, &str) {
    match schema.rsplit_once('.') {
        Some((catalog, bare)) if !catalog.is_empty() && !bare.is_empty() => (Some(catalog), bare),
        _ => (None, schema),
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

impl PostgresAdapter {
    /// Foreign keys of `table`, or of the whole schema when `table` is None.
    async fn foreign_keys(
        &self,
        schema: &str,
        table: Option<&str>,
    ) -> DbResult<Vec<ForeignKeyInfo>> {
        let (pool, bare) = self.pool_for(schema).await?;
        let rows = sqlx::query(FOREIGN_KEYS_SQL)
            .bind(&bare)
            .bind(table)
            .fetch_all(&pool)
            .await
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| ForeignKeyInfo {
                constraint_name: r.get("constraint_name"),
                table_name: r.get("table_name"),
                column_name: r.get("column_name"),
                referenced_schema: Self::qualify_like(schema, r.get("ref_schema")),
                referenced_table: r.get("ref_table"),
                referenced_column: r.get("ref_column"),
            })
            .collect())
    }
}

#[async_trait]
impl DatabaseAdapter for PostgresAdapter {
    fn name(&self) -> &str {
        "PostgreSQL"
    }

    async fn get_table_ddl(&self, schema: &str, table: &str) -> DbResult<String> {
        let (pool, schema) = self.pool_for(schema).await?;
        let schema = schema.as_str();
        let rows: Vec<(String, String, bool, Option<String>)> = sqlx::query_as(DDL_COLUMNS_SQL)
            .bind(schema)
            .bind(table)
            .fetch_all(&pool)
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
            .fetch_all(&pool)
            .await
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
        let primary_key: Vec<String> = primary_key.into_iter().map(|(name,)| name).collect();

        Ok(build_table_ddl(schema, table, &columns, &primary_key))
    }

    fn db_type(&self) -> DatabaseType {
        DatabaseType::PostgreSQL
    }

    async fn get_catalogs(&self) -> DbResult<Vec<Catalog>> {
        let rows = sqlx::query(
            "SELECT datname::text AS name FROM pg_catalog.pg_database \
             WHERE NOT datistemplate AND datallowconn \
               AND has_database_privilege(datname, 'CONNECT') \
             ORDER BY datname",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;
        Ok(rows
            .iter()
            .map(|r| Catalog {
                name: r.get("name"),
            })
            .collect())
    }

    async fn get_schemas(&self) -> DbResult<Vec<Schema>> {
        schemas_of(&self.pool).await
    }

    async fn get_schemas_in(&self, catalog: &str) -> DbResult<Vec<Schema>> {
        let (pool, _) = self.pool_for(&format!("{catalog}.public")).await?;
        schemas_of(&pool).await
    }

    async fn get_tables(&self, schema: &str) -> DbResult<Vec<Table>> {
        let (pool, bare) = self.pool_for(schema).await?;
        let rows = sqlx::query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = $1 AND table_type = 'BASE TABLE' \
             ORDER BY table_name",
        )
        .bind(bare.as_str())
        .fetch_all(&pool)
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
        let (pool, bare) = self.pool_for(schema).await?;
        let rows = sqlx::query(
            "SELECT table_name FROM information_schema.views \
             WHERE table_schema = $1 \
             ORDER BY table_name",
        )
        .bind(bare.as_str())
        .fetch_all(&pool)
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
        let (pool, bare) = self.pool_for(schema).await?;
        let rows = sqlx::query(
            "SELECT routine_name FROM information_schema.routines \
             WHERE routine_schema = $1 AND routine_type = 'PROCEDURE' \
             ORDER BY routine_name",
        )
        .bind(bare.as_str())
        .fetch_all(&pool)
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
        let (pool, bare) = self.pool_for(schema).await?;
        let rows = sqlx::query(
            "SELECT routine_name FROM information_schema.routines \
             WHERE routine_schema = $1 AND routine_type = 'FUNCTION' \
             ORDER BY routine_name",
        )
        .bind(bare.as_str())
        .fetch_all(&pool)
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
        let (pool, bare) = self.pool_for(schema).await?;
        let rows = sqlx::query(
            "SELECT matviewname FROM pg_matviews \
             WHERE schemaname = $1 ORDER BY matviewname",
        )
        .bind(bare.as_str())
        .fetch_all(&pool)
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
        let (pool, bare) = self.pool_for(schema).await?;
        let rows = sqlx::query(
            "SELECT indexname FROM pg_indexes \
             WHERE schemaname = $1 ORDER BY indexname",
        )
        .bind(bare.as_str())
        .fetch_all(&pool)
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
        let (pool, bare) = self.pool_for(schema).await?;
        let rows = sqlx::query(
            "SELECT sequence_name FROM information_schema.sequences \
             WHERE sequence_schema = $1 ORDER BY sequence_name",
        )
        .bind(bare.as_str())
        .fetch_all(&pool)
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
        let (pool, bare) = self.pool_for(schema).await?;
        let rows = sqlx::query(
            "SELECT DISTINCT trigger_name \
             FROM information_schema.triggers \
             WHERE trigger_schema = $1 ORDER BY trigger_name",
        )
        .bind(bare.as_str())
        .fetch_all(&pool)
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
        let (pool, bare) = self.pool_for(schema).await?;
        let rows = sqlx::query(COLUMNS_SQL)
            .bind(bare.as_str())
            .bind(table)
            .fetch_all(&pool)
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
        let Some(schema) = schema.filter(|s| !s.is_empty()) else {
            return exec::run(&self.pool, query, None, RowSink::Stream(&tx)).await;
        };
        let (pool, bare) = self.pool_for(schema).await?;
        exec::run(&pool, query, Some(&bare), RowSink::Stream(&tx)).await
    }

    async fn get_foreign_keys(&self, schema: &str, table: &str) -> DbResult<Vec<ForeignKeyInfo>> {
        self.foreign_keys(schema, Some(table)).await
    }

    async fn get_schema_foreign_keys(&self, schema: &str) -> DbResult<Vec<ForeignKeyInfo>> {
        self.foreign_keys(schema, None).await
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
