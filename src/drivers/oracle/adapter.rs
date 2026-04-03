use async_trait::async_trait;
use oracle::Connection;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task;

use crate::core::DatabaseAdapter;
use crate::core::error::{DbError, DbResult};
use crate::core::models::*;

/// Fetch source code for a given object type from ALL_SOURCE.
/// Uses a PL/SQL block to concatenate all lines server-side into one CLOB,
/// then returns it as a single String — avoids ODPI-C row-by-row buffer issues.
/// Fetch source code row-by-row from ALL_SOURCE and concatenate in Rust.
/// Avoids CLOB buffer issues in the oracle crate that can truncate large packages.
fn fetch_source(
    conn: &Connection,
    schema: &str,
    name: &str,
    obj_type: &str,
) -> DbResult<Option<String>> {
    let sql = "SELECT text FROM all_source \
               WHERE owner = :1 AND name = :2 AND type = :3 \
               ORDER BY line";

    let rows = conn
        .query(sql, &[&schema, &name, &obj_type])
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

    let mut result = String::new();
    let mut found = false;

    for row_result in rows {
        let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
        let text: Option<String> = row
            .get(0)
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
        if let Some(line) = text {
            found = true;
            // ALL_SOURCE.TEXT typically includes trailing newline;
            // expand tabs to 4 spaces and strip \0 and \r.
            for c in line.chars() {
                if c == '\t' {
                    result.push_str("    ");
                } else if c != '\0' && c != '\r' {
                    result.push(c);
                }
            }
        }
    }

    if found {
        // Trim trailing whitespace/newlines
        let trimmed = result.trim_end().to_string();
        Ok(Some(trimmed))
    } else {
        Ok(None)
    }
}

/// Oracle adapter wrapping the synchronous `oracle` crate.
/// All DB calls run inside `spawn_blocking` to avoid blocking the Tokio runtime.
pub struct OracleAdapter {
    conn: Arc<Mutex<Connection>>,
}

impl OracleAdapter {
    pub async fn connect(username: &str, password: &str, connect_string: &str) -> DbResult<Self> {
        let user = username.to_string();
        let pass = password.to_string();
        let cs = connect_string.to_string();

        let conn = task::spawn_blocking(move || {
            Connection::connect(&user, &pass, &cs)
                .map_err(|e| DbError::ConnectionFailed(e.to_string()))
        })
        .await
        .map_err(|e| DbError::ConnectionFailed(format!("Task join failed: {e}")))??;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

macro_rules! blocking_query {
    ($conn:expr, $sql:expr, $bind:expr, $map:expr) => {{
        let conn = Arc::clone(&$conn);
        let bind_val = $bind.to_string();
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = conn
                .query($sql, &[&bind_val])
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let mut results = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                results.push($map(&row));
            }
            Ok(results)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }};
}

#[async_trait]
impl DatabaseAdapter for OracleAdapter {
    fn name(&self) -> &str {
        "Oracle"
    }

    fn db_type(&self) -> DatabaseType {
        DatabaseType::Oracle
    }

    async fn get_schemas(&self) -> DbResult<Vec<Schema>> {
        let conn = Arc::clone(&self.conn);
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = conn
                .query(
                    "SELECT username FROM all_users ORDER BY username",
                    &[] as &[&dyn oracle::sql_type::ToSql],
                )
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let mut schemas = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let name: String = row
                    .get(0)
                    .map_err(|e| DbError::QueryFailed(e.to_string()))?;
                schemas.push(Schema { name });
            }
            Ok(schemas)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_tables(&self, schema: &str) -> DbResult<Vec<Table>> {
        let schema_owned = schema.to_string();
        blocking_query!(
            self.conn,
            "SELECT table_name FROM all_tables WHERE owner = :1 ORDER BY table_name",
            schema_owned,
            |row: &oracle::Row| {
                let name: String = row.get(0).unwrap_or_default();
                Table {
                    name,
                    schema: schema_owned.clone(),
                }
            }
        )
    }

    async fn get_views(&self, schema: &str) -> DbResult<Vec<View>> {
        let conn = Arc::clone(&self.conn);
        let schema_owned = schema.to_string();
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = conn
                .query(
                    "SELECT v.view_name, NVL(o.status, 'VALID') as status \
                     FROM all_views v \
                     LEFT JOIN all_objects o ON v.owner = o.owner AND v.view_name = o.object_name AND o.object_type = 'VIEW' \
                     WHERE v.owner = :1 \
                     ORDER BY v.view_name",
                    &[&schema_owned],
                )
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let mut results = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let name: String = row.get(0).unwrap_or_default();
                let status: String = row.get(1).unwrap_or_default();
                results.push(View {
                    name,
                    schema: schema_owned.clone(),
                    valid: status == "VALID",
                });
            }
            Ok(results)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_packages(&self, schema: &str) -> DbResult<Vec<Package>> {
        let conn = Arc::clone(&self.conn);
        let schema_owned = schema.to_string();
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = conn
                .query(
                    "SELECT object_name, \
                     MAX(CASE WHEN object_type = 'PACKAGE BODY' THEN 1 ELSE 0 END) as has_body, \
                     MIN(CASE WHEN status = 'INVALID' THEN 0 ELSE 1 END) as is_valid \
                     FROM all_objects \
                     WHERE owner = :1 AND object_type IN ('PACKAGE', 'PACKAGE BODY') \
                     GROUP BY object_name \
                     ORDER BY object_name",
                    &[&schema_owned],
                )
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let mut packages = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let name: String = row.get(0).unwrap_or_default();
                let has_body_num: i32 = row.get(1).unwrap_or(0);
                let is_valid_num: i32 = row.get(2).unwrap_or(1);
                packages.push(Package {
                    name,
                    schema: schema_owned.clone(),
                    has_body: has_body_num == 1,
                    valid: is_valid_num == 1,
                });
            }
            Ok(packages)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_procedures(&self, schema: &str) -> DbResult<Vec<Procedure>> {
        let conn = Arc::clone(&self.conn);
        let schema_owned = schema.to_string();
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = conn
                .query(
                    "SELECT object_name, status FROM all_objects \
                     WHERE owner = :1 AND object_type = 'PROCEDURE' \
                     ORDER BY object_name",
                    &[&schema_owned],
                )
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let mut results = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let name: String = row.get(0).unwrap_or_default();
                let status: String = row.get(1).unwrap_or_default();
                results.push(Procedure {
                    name,
                    schema: schema_owned.clone(),
                    valid: status == "VALID",
                });
            }
            Ok(results)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_functions(&self, schema: &str) -> DbResult<Vec<Function>> {
        let conn = Arc::clone(&self.conn);
        let schema_owned = schema.to_string();
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = conn
                .query(
                    "SELECT object_name, status FROM all_objects \
                     WHERE owner = :1 AND object_type = 'FUNCTION' \
                     ORDER BY object_name",
                    &[&schema_owned],
                )
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let mut results = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let name: String = row.get(0).unwrap_or_default();
                let status: String = row.get(1).unwrap_or_default();
                results.push(Function {
                    name,
                    schema: schema_owned.clone(),
                    valid: status == "VALID",
                });
            }
            Ok(results)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_columns(&self, schema: &str, table: &str) -> DbResult<Vec<Column>> {
        let conn = Arc::clone(&self.conn);
        let schema_owned = schema.to_string();
        let table_owned = table.to_string();
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = conn
                .query(
                    "SELECT c.column_name, c.data_type || \
                     CASE WHEN c.data_precision IS NOT NULL \
                       THEN '(' || c.data_precision || \
                         CASE WHEN c.data_scale > 0 THEN ',' || c.data_scale ELSE '' END || ')' \
                       WHEN c.char_length > 0 THEN '(' || c.char_length || ')' \
                       ELSE '' END as full_type, \
                     c.nullable, \
                     CASE WHEN cc.column_name IS NOT NULL THEN 1 ELSE 0 END as is_pk \
                     FROM all_tab_columns c \
                     LEFT JOIN all_cons_columns cc \
                       ON c.owner = cc.owner AND c.table_name = cc.table_name \
                       AND c.column_name = cc.column_name \
                       AND cc.constraint_name IN ( \
                         SELECT constraint_name FROM all_constraints \
                         WHERE owner = c.owner AND table_name = c.table_name \
                         AND constraint_type = 'P') \
                     WHERE c.owner = :1 AND c.table_name = :2 \
                     ORDER BY c.column_id",
                    &[&schema_owned, &table_owned],
                )
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let mut columns = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let name: String = row.get(0).unwrap_or_default();
                let data_type: String = row.get(1).unwrap_or_default();
                let nullable_str: String = row.get(2).unwrap_or_default();
                let is_pk_num: i32 = row.get(3).unwrap_or(0);
                columns.push(Column {
                    name,
                    data_type,
                    nullable: nullable_str == "Y",
                    is_primary_key: is_pk_num == 1,
                });
            }
            Ok(columns)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_package_content(
        &self,
        schema: &str,
        name: &str,
    ) -> DbResult<Option<PackageContent>> {
        let conn = Arc::clone(&self.conn);
        let schema_owned = schema.to_string();
        let name_owned = name.to_string();
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();

            let declaration = match fetch_source(&conn, &schema_owned, &name_owned, "PACKAGE")? {
                Some(d) => d,
                None => return Ok(None),
            };
            let body = fetch_source(&conn, &schema_owned, &name_owned, "PACKAGE BODY")?;

            Ok(Some(PackageContent { declaration, body }))
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn execute(&self, query: &str) -> DbResult<QueryResult> {
        let conn = Arc::clone(&self.conn);
        let query_owned = query.to_string();
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn
                .statement(&query_owned)
                .build()
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let rows = stmt
                .query(&[] as &[&dyn oracle::sql_type::ToSql])
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;

            let column_info = rows.column_info();
            let columns: Vec<String> = column_info.iter().map(|c| c.name().to_string()).collect();

            let mut data = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let mut row_data = Vec::new();
                for i in 0..columns.len() {
                    let val: String = row
                        .get::<usize, Option<String>>(i)
                        .unwrap_or(None)
                        .unwrap_or_else(|| "NULL".to_string());
                    row_data.push(val);
                }
                data.push(row_data);
            }

            Ok(QueryResult {
                columns,
                rows: data,
            })
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }
}
