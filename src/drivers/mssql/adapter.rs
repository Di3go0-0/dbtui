//! SQL Server driver, built on tiberius (TDS 7.3+).
//!
//! Targets the currently supported server versions — SQL Server 2016 through
//! 2022 and Azure SQL Database — so metadata comes from the `sys.*` catalog
//! views rather than `INFORMATION_SCHEMA`, which is a lossy ANSI shim on SQL
//! Server (no filtered indexes, no computed-column flags, no sequences).
//!
//! In SQL Server a connection targets one *database*, and schemas live inside
//! it. That maps onto the `Schema` model the same way PostgreSQL does: the
//! connection picks the database, `get_schemas()` lists the schemas within it.

use async_trait::async_trait;
use futures_util::TryStreamExt;
use std::sync::Arc;
use tiberius::{AuthMethod, Config, Row, ToSql};
use tokio::sync::mpsc;

use crate::core::DatabaseAdapter;
use crate::core::adapter::QueryBatch;
use crate::core::error::{DbError, DbResult};
use crate::core::models::*;
use crate::drivers::mssql::pool::MssqlPool;
use crate::drivers::mssql::value::mssql_row_to_strings;

/// Schemas SQL Server creates in every database; hiding them keeps the tree
/// showing only what a user actually authored.
const SYSTEM_SCHEMAS: &str = "'sys','INFORMATION_SCHEMA','guest','db_owner',\
     'db_accessadmin','db_securityadmin','db_ddladmin','db_backupoperator',\
     'db_datareader','db_datawriter','db_denydatareader','db_denydatawriter'";

pub struct MssqlAdapter {
    pool: Arc<MssqlPool>,
}

impl MssqlAdapter {
    pub async fn connect(
        host: &str,
        port: u16,
        database: Option<&str>,
        username: &str,
        password: &str,
    ) -> DbResult<Self> {
        let mut config = Config::new();
        config.host(host);
        config.port(port);
        config.authentication(AuthMethod::sql_server(username, password));
        if let Some(db) = database.filter(|d| !d.is_empty()) {
            config.database(db);
        }
        config.application_name("dbtui");
        // SQL Server ships with a self-signed certificate and every mainstream
        // client (SSMS, DBeaver, sqlcmd -C) trusts it by default, so requiring
        // a verifiable chain here would reject nearly every real server. The
        // transport is still encrypted; only the certificate is unverified.
        config.trust_cert();

        let pool = MssqlPool::connect(config).await?;
        Ok(Self { pool })
    }

    /// Run a parameterized catalog query and collect its first result set.
    async fn fetch(&self, sql: &str, params: &[&dyn ToSql]) -> DbResult<Vec<Row>> {
        let mut pooled = self.pool.get().await?;
        let result = async {
            let client = pooled.client()?;
            let stream = client
                .query(sql, params)
                .await
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            stream
                .into_first_result()
                .await
                .map_err(|e| DbError::QueryFailed(e.to_string()))
        }
        .await;

        match result {
            Ok(rows) => Ok(rows),
            Err(e) => {
                pooled.discard();
                Err(e)
            }
        }
    }

    /// Fetch a single-column list of object names for one schema.
    ///
    /// `template` is SQL where `{p}` marks each catalog-qualified reference;
    /// it expands to `[database].` when `schema` carries one.
    async fn fetch_names(&self, template: &str, schema: &str) -> DbResult<Vec<String>> {
        let (catalog, schema) = split_qualified(schema);
        let sql = template.replace("{p}", &catalog_prefix(catalog));
        let rows = self.fetch(&sql, &[&schema]).await?;
        Ok(rows.iter().map(|r| text(r, 0)).collect())
    }

    /// List the non-system schemas of one database. `prefix` is `[db].` or empty.
    async fn schemas_with_prefix(&self, prefix: &str) -> DbResult<Vec<Schema>> {
        let sql = format!(
            "SELECT s.name FROM {prefix}sys.schemas s \
             WHERE s.name NOT IN ({SYSTEM_SCHEMAS}) ORDER BY s.name"
        );
        let rows = self.fetch(&sql, &[]).await?;
        Ok(rows.iter().map(|r| Schema { name: text(r, 0) }).collect())
    }

    /// Run a two-parameter catalog query for one schema-qualified object.
    async fn fetch_object(&self, template: &str, schema: &str, object: &str) -> DbResult<Vec<Row>> {
        let (catalog, schema) = split_qualified(schema);
        let sql = template.replace("{p}", &catalog_prefix(catalog));
        self.fetch(&sql, &[&schema, &object]).await
    }
}

/// Read column `idx` as an owned String, treating NULL as empty.
fn text(row: &Row, idx: usize) -> String {
    row.get::<&str, _>(idx).unwrap_or_default().to_string()
}

/// Split a `database.schema` argument into its parts.
///
/// The sidebar qualifies schema names with their database once a connection
/// reports catalogs, so a single connection can read every database on the
/// server. A bare name (no dot) targets whatever database the connection
/// itself opened.
fn split_qualified(schema: &str) -> (Option<&str>, &str) {
    match schema.split_once('.') {
        Some((catalog, schema)) if !catalog.is_empty() && !schema.is_empty() => {
            (Some(catalog), schema)
        }
        _ => (None, schema),
    }
}

/// Render `[database].` as a prefix for a `sys.*` reference, or empty for the
/// connection's own database.
///
/// The name is bracket-quoted with embedded `]` doubled. Catalog names come
/// from `sys.databases`, but they reach here through the tree, so they are
/// quoted rather than trusted.
fn catalog_prefix(catalog: Option<&str>) -> String {
    match catalog {
        Some(db) => format!("[{}].", db.replace(']', "]]")),
        None => String::new(),
    }
}

#[async_trait]
impl DatabaseAdapter for MssqlAdapter {
    fn name(&self) -> &str {
        "SQL Server"
    }

    fn db_type(&self) -> DatabaseType {
        DatabaseType::SqlServer
    }

    async fn get_catalogs(&self) -> DbResult<Vec<Catalog>> {
        // state = 0 is ONLINE; HAS_DBACCESS drops the databases this login
        // cannot open, which would otherwise fail the moment they are expanded.
        let rows = self
            .fetch(
                "SELECT d.name FROM sys.databases d \
                 WHERE d.state = 0 AND HAS_DBACCESS(d.name) = 1 \
                 ORDER BY d.name",
                &[],
            )
            .await?;
        Ok(rows.iter().map(|r| Catalog { name: text(r, 0) }).collect())
    }

    async fn get_schemas(&self) -> DbResult<Vec<Schema>> {
        self.schemas_with_prefix("").await
    }

    async fn get_schemas_in(&self, catalog: &str) -> DbResult<Vec<Schema>> {
        self.schemas_with_prefix(&catalog_prefix(Some(catalog)))
            .await
    }

    async fn get_tables(&self, schema: &str) -> DbResult<Vec<Table>> {
        let names = self
            .fetch_names(
                "SELECT t.name FROM {p}sys.tables t \
                 JOIN {p}sys.schemas s ON s.schema_id = t.schema_id \
                 WHERE s.name = @P1 AND t.is_ms_shipped = 0 \
                 ORDER BY t.name",
                schema,
            )
            .await?;
        Ok(names
            .into_iter()
            .map(|name| Table {
                name,
                schema: schema.to_string(),
                privilege: ObjectPrivilege::Full,
            })
            .collect())
    }

    async fn get_views(&self, schema: &str) -> DbResult<Vec<View>> {
        let names = self
            .fetch_names(
                "SELECT v.name FROM {p}sys.views v \
                 JOIN {p}sys.schemas s ON s.schema_id = v.schema_id \
                 WHERE s.name = @P1 AND v.is_ms_shipped = 0 \
                 ORDER BY v.name",
                schema,
            )
            .await?;
        Ok(names
            .into_iter()
            .map(|name| View {
                name,
                schema: schema.to_string(),
                valid: true,
                privilege: ObjectPrivilege::Full,
            })
            .collect())
    }

    async fn get_procedures(&self, schema: &str) -> DbResult<Vec<Procedure>> {
        let names = self
            .fetch_names(
                "SELECT p.name FROM {p}sys.procedures p \
                 JOIN {p}sys.schemas s ON s.schema_id = p.schema_id \
                 WHERE s.name = @P1 AND p.is_ms_shipped = 0 \
                 ORDER BY p.name",
                schema,
            )
            .await?;
        Ok(names
            .into_iter()
            .map(|name| Procedure {
                name,
                schema: schema.to_string(),
                valid: true,
                privilege: ObjectPrivilege::Full,
            })
            .collect())
    }

    async fn get_functions(&self, schema: &str) -> DbResult<Vec<Function>> {
        // FN = scalar, IF = inline table-valued, TF = multi-statement
        // table-valued, AF = CLR aggregate, FS/FT = CLR scalar/table.
        let names = self
            .fetch_names(
                "SELECT o.name FROM {p}sys.objects o \
                 JOIN {p}sys.schemas s ON s.schema_id = o.schema_id \
                 WHERE s.name = @P1 AND o.type IN ('FN','IF','TF','AF','FS','FT') \
                   AND o.is_ms_shipped = 0 \
                 ORDER BY o.name",
                schema,
            )
            .await?;
        Ok(names
            .into_iter()
            .map(|name| Function {
                name,
                schema: schema.to_string(),
                valid: true,
                privilege: ObjectPrivilege::Full,
            })
            .collect())
    }

    async fn get_columns(&self, schema: &str, table: &str) -> DbResult<Vec<Column>> {
        let rows = self.fetch_object(COLUMNS_SQL, schema, table).await?;
        Ok(rows
            .iter()
            .map(|r| Column {
                name: text(r, 0),
                data_type: format_type(
                    &text(r, 1),
                    r.get::<i16, _>(2).unwrap_or(0),
                    r.get::<u8, _>(3).unwrap_or(0),
                    r.get::<u8, _>(4).unwrap_or(0),
                ),
                nullable: r.get::<bool, _>(5).unwrap_or(true),
                is_primary_key: r.get::<i32, _>(6).unwrap_or(0) == 1,
            })
            .collect())
    }

    async fn get_indexes(&self, schema: &str) -> DbResult<Vec<Index>> {
        // Heaps have index_id 0 and no name; hypothetical indexes are stats.
        let names = self
            .fetch_names(
                "SELECT i.name FROM {p}sys.indexes i \
                 JOIN {p}sys.objects o ON o.object_id = i.object_id \
                 JOIN {p}sys.schemas s ON s.schema_id = o.schema_id \
                 WHERE s.name = @P1 AND i.name IS NOT NULL \
                   AND i.is_hypothetical = 0 AND o.is_ms_shipped = 0 \
                 ORDER BY i.name",
                schema,
            )
            .await?;
        Ok(names
            .into_iter()
            .map(|name| Index {
                name,
                schema: schema.to_string(),
            })
            .collect())
    }

    async fn get_triggers(&self, schema: &str) -> DbResult<Vec<Trigger>> {
        let names = self
            .fetch_names(
                "SELECT tr.name FROM {p}sys.triggers tr \
                 JOIN {p}sys.objects o ON o.object_id = tr.parent_id \
                 JOIN {p}sys.schemas s ON s.schema_id = o.schema_id \
                 WHERE s.name = @P1 AND tr.is_ms_shipped = 0 \
                 ORDER BY tr.name",
                schema,
            )
            .await?;
        Ok(names
            .into_iter()
            .map(|name| Trigger {
                name,
                schema: schema.to_string(),
            })
            .collect())
    }

    async fn get_sequences(&self, schema: &str) -> DbResult<Vec<Sequence>> {
        let names = self
            .fetch_names(
                "SELECT q.name FROM {p}sys.sequences q \
                 JOIN {p}sys.schemas s ON s.schema_id = q.schema_id \
                 WHERE s.name = @P1 ORDER BY q.name",
                schema,
            )
            .await?;
        Ok(names
            .into_iter()
            .map(|name| Sequence {
                name,
                schema: schema.to_string(),
            })
            .collect())
    }

    async fn get_types(&self, schema: &str) -> DbResult<Vec<DbType>> {
        let names = self
            .fetch_names(
                "SELECT t.name FROM {p}sys.types t \
                 JOIN {p}sys.schemas s ON s.schema_id = t.schema_id \
                 WHERE s.name = @P1 AND t.is_user_defined = 1 \
                 ORDER BY t.name",
                schema,
            )
            .await?;
        Ok(names
            .into_iter()
            .map(|name| DbType {
                name,
                schema: schema.to_string(),
            })
            .collect())
    }

    async fn get_foreign_keys(&self, schema: &str, table: &str) -> DbResult<Vec<ForeignKeyInfo>> {
        let rows = self.fetch_object(FOREIGN_KEYS_SQL, schema, table).await?;
        Ok(rows
            .iter()
            .map(|r| ForeignKeyInfo {
                constraint_name: text(r, 0),
                column_name: text(r, 1),
                referenced_schema: text(r, 2),
                referenced_table: text(r, 3),
                referenced_column: text(r, 4),
            })
            .collect())
    }

    async fn get_source_code(&self, schema: &str, name: &str, _obj_type: &str) -> DbResult<String> {
        let rows = self
            .fetch_object(
                "SELECT m.definition FROM {p}sys.sql_modules m \
                 JOIN {p}sys.objects o ON o.object_id = m.object_id \
                 JOIN {p}sys.schemas s ON s.schema_id = o.schema_id \
                 WHERE s.name = @P1 AND o.name = @P2",
                schema,
                name,
            )
            .await?;
        // Encrypted modules (WITH ENCRYPTION) have a NULL definition.
        Ok(rows.first().map(|r| text(r, 0)).unwrap_or_default())
    }

    async fn get_table_ddl(&self, schema: &str, table: &str) -> DbResult<String> {
        let cols = self.get_columns(schema, table).await?;
        if cols.is_empty() {
            return Ok(format!("-- No columns found for {schema}.{table}"));
        }

        let mut ddl = format!("CREATE TABLE [{schema}].[{table}] (\n");
        for (i, c) in cols.iter().enumerate() {
            let null_str = if c.nullable { "NULL" } else { "NOT NULL" };
            let comma = if i + 1 < cols.len() { "," } else { "" };
            ddl.push_str(&format!(
                "    [{}] {} {}{}\n",
                c.name, c.data_type, null_str, comma
            ));
        }

        let pk: Vec<&str> = cols
            .iter()
            .filter(|c| c.is_primary_key)
            .map(|c| c.name.as_str())
            .collect();
        if !pk.is_empty() {
            if ddl.ends_with('\n') && !ddl.ends_with(",\n") {
                ddl.pop();
                ddl.push_str(",\n");
            }
            let cols = pk
                .iter()
                .map(|c| format!("[{c}]"))
                .collect::<Vec<_>>()
                .join(", ");
            ddl.push_str(&format!("    PRIMARY KEY ({cols})\n"));
        }

        ddl.push_str(");");
        Ok(ddl)
    }

    async fn execute(&self, query: &str) -> DbResult<QueryResult> {
        let mut pooled = self.pool.get().await?;
        let result = async {
            let client = pooled.client()?;

            if !crate::core::adapter::is_row_producing_query(query) {
                let outcome = client
                    .execute(query, &[])
                    .await
                    .map_err(|e| DbError::QueryFailed(e.to_string()))?;
                return Ok(QueryResult {
                    columns: vec!["Result".to_string()],
                    rows: vec![vec![format!(
                        "Statement executed successfully ({} row(s) affected)",
                        outcome.total()
                    )]],
                    elapsed: None,
                });
            }

            // A raw batch, not sp_executesql, so user SQL behaves the way it
            // does in SSMS — DECLARE, temp tables and multiple statements all
            // share one batch scope.
            let mut stream = client
                .simple_query(query)
                .await
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let columns = column_names(&mut stream).await?;
            let rows = stream
                .into_first_result()
                .await
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;

            Ok(QueryResult {
                columns,
                rows: rows.iter().map(mssql_row_to_strings).collect(),
                elapsed: None,
            })
        }
        .await;

        match result {
            Ok(v) => Ok(v),
            Err(e) => {
                pooled.discard();
                Err(e)
            }
        }
    }

    async fn execute_streaming(
        &self,
        query: &str,
        tx: mpsc::Sender<DbResult<QueryBatch>>,
    ) -> DbResult<()> {
        const BATCH_SIZE: usize = 500;

        if !crate::core::adapter::is_row_producing_query(query) {
            let result = self.execute(query).await?;
            let _ = tx
                .send(Ok(QueryBatch {
                    columns: result.columns,
                    rows: result.rows,
                    done: true,
                }))
                .await;
            return Ok(());
        }

        let mut pooled = self.pool.get().await?;
        let result = async {
            let client = pooled.client()?;
            let mut stream = client
                .simple_query(query)
                .await
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let columns = column_names(&mut stream).await?;

            let mut rows = stream.into_row_stream();
            let mut batch = Vec::with_capacity(BATCH_SIZE);

            loop {
                let row = match rows.try_next().await {
                    Ok(Some(row)) => row,
                    Ok(None) => break,
                    Err(e) => return Err(DbError::QueryFailed(e.to_string())),
                };
                batch.push(mssql_row_to_strings(&row));

                if batch.len() >= BATCH_SIZE {
                    let rows = std::mem::replace(&mut batch, Vec::with_capacity(BATCH_SIZE));
                    if tx
                        .send(Ok(QueryBatch {
                            columns: columns.clone(),
                            rows,
                            done: false,
                        }))
                        .await
                        .is_err()
                    {
                        // Receiver dropped — the tab was closed or the query
                        // cancelled. Stop early; unread tokens make this
                        // connection unsafe to reuse.
                        return Ok(false);
                    }
                }
            }

            let _ = tx
                .send(Ok(QueryBatch {
                    columns,
                    rows: batch,
                    done: true,
                }))
                .await;
            Ok(true)
        }
        .await;

        match result {
            Ok(true) => Ok(()),
            Ok(false) => {
                pooled.discard();
                Ok(())
            }
            Err(e) => {
                pooled.discard();
                Err(e)
            }
        }
    }
}

/// Read the result-set column names from a stream before its rows are consumed.
///
/// Taking them here rather than from the first row means an empty result set
/// still renders its headers.
async fn column_names(stream: &mut tiberius::QueryStream<'_>) -> DbResult<Vec<String>> {
    let cols = stream
        .columns()
        .await
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;
    Ok(cols
        .map(|cols| cols.iter().map(|c| c.name().to_string()).collect())
        .unwrap_or_default())
}

/// Render a SQL Server type with its length/precision, the way SSMS shows it.
///
/// `max_length` is in bytes, so the Unicode types report double their character
/// length, and -1 means the `(max)` variant.
fn format_type(name: &str, max_length: i16, precision: u8, scale: u8) -> String {
    match name {
        "decimal" | "numeric" => format!("{name}({precision},{scale})"),
        "datetime2" | "datetimeoffset" | "time" => format!("{name}({scale})"),
        "varchar" | "char" | "varbinary" | "binary" => {
            if max_length == -1 {
                format!("{name}(max)")
            } else {
                format!("{name}({max_length})")
            }
        }
        "nvarchar" | "nchar" => {
            if max_length == -1 {
                format!("{name}(max)")
            } else {
                format!("{name}({})", max_length / 2)
            }
        }
        _ => name.to_string(),
    }
}

const COLUMNS_SQL: &str = "SELECT c.name, ty.name AS type_name, c.max_length, \
            c.precision, c.scale, c.is_nullable, \
            CASE WHEN pk.column_id IS NOT NULL THEN 1 ELSE 0 END AS is_pk \
     FROM {p}sys.columns c \
     JOIN {p}sys.objects o ON o.object_id = c.object_id \
     JOIN {p}sys.schemas s ON s.schema_id = o.schema_id \
     JOIN {p}sys.types ty ON ty.user_type_id = c.user_type_id \
     OUTER APPLY ( \
         SELECT ic.column_id FROM {p}sys.index_columns ic \
         JOIN {p}sys.indexes i ON i.object_id = ic.object_id AND i.index_id = ic.index_id \
         WHERE i.is_primary_key = 1 \
           AND ic.object_id = c.object_id AND ic.column_id = c.column_id \
     ) pk \
     WHERE s.name = @P1 AND o.name = @P2 \
     ORDER BY c.column_id";

const FOREIGN_KEYS_SQL: &str = "SELECT fk.name, pc.name, rs.name, rt.name, rc.name \
     FROM {p}sys.foreign_keys fk \
     JOIN {p}sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id \
     JOIN {p}sys.tables pt ON pt.object_id = fk.parent_object_id \
     JOIN {p}sys.schemas ps ON ps.schema_id = pt.schema_id \
     JOIN {p}sys.columns pc ON pc.object_id = fkc.parent_object_id \
                        AND pc.column_id = fkc.parent_column_id \
     JOIN {p}sys.tables rt ON rt.object_id = fk.referenced_object_id \
     JOIN {p}sys.schemas rs ON rs.schema_id = rt.schema_id \
     JOIN {p}sys.columns rc ON rc.object_id = fkc.referenced_object_id \
                        AND rc.column_id = fkc.referenced_column_id \
     WHERE ps.name = @P1 AND pt.name = @P2 \
     ORDER BY fk.name, fkc.constraint_column_id";

#[cfg(test)]
mod tests {
    use super::{catalog_prefix, format_type, split_qualified};

    #[test]
    fn qualified_schema_splits_on_the_database() {
        assert_eq!(split_qualified("BANCO.dbo"), (Some("BANCO"), "dbo"));
    }

    #[test]
    fn bare_schema_targets_the_connections_own_database() {
        assert_eq!(split_qualified("dbo"), (None, "dbo"));
    }

    #[test]
    fn malformed_qualifications_fall_back_to_bare() {
        assert_eq!(split_qualified(".dbo"), (None, ".dbo"));
        assert_eq!(split_qualified("BANCO."), (None, "BANCO."));
    }

    #[test]
    fn prefix_is_bracket_quoted() {
        assert_eq!(catalog_prefix(Some("BANCO")), "[BANCO].");
        assert_eq!(catalog_prefix(None), "");
    }

    #[test]
    fn prefix_doubles_closing_brackets() {
        // Without doubling, a `]` in the name would end the quoted identifier
        // early and the rest would be parsed as SQL.
        assert_eq!(catalog_prefix(Some("we]rd")), "[we]]rd].");
    }

    #[test]
    fn template_expands_for_both_shapes() {
        let template = "SELECT s.name FROM {p}sys.schemas s";
        assert_eq!(
            template.replace("{p}", &catalog_prefix(Some("BANCO"))),
            "SELECT s.name FROM [BANCO].sys.schemas s"
        );
        assert_eq!(
            template.replace("{p}", &catalog_prefix(None)),
            "SELECT s.name FROM sys.schemas s"
        );
    }

    #[test]
    fn unicode_length_is_halved() {
        assert_eq!(format_type("nvarchar", 100, 0, 0), "nvarchar(50)");
    }

    #[test]
    fn byte_length_is_verbatim() {
        assert_eq!(format_type("varchar", 50, 0, 0), "varchar(50)");
    }

    #[test]
    fn max_variants() {
        assert_eq!(format_type("nvarchar", -1, 0, 0), "nvarchar(max)");
        assert_eq!(format_type("varbinary", -1, 0, 0), "varbinary(max)");
    }

    #[test]
    fn decimal_uses_precision_and_scale() {
        assert_eq!(format_type("decimal", 9, 18, 2), "decimal(18,2)");
    }

    #[test]
    fn plain_types_have_no_suffix() {
        assert_eq!(format_type("int", 4, 10, 0), "int");
        assert_eq!(
            format_type("uniqueidentifier", 16, 0, 0),
            "uniqueidentifier"
        );
    }
}
