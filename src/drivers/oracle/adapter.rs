use async_trait::async_trait;
use oracle::Connection;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio::task;

use crate::core::DatabaseAdapter;
use crate::core::adapter::QueryBatch;
use crate::core::error::{DbError, DbResult};
use crate::core::models::*;

/// Fetch DDL via DBMS_METADATA, reading the CLOB in 4000-char chunks server-side
/// to avoid ODPI-C CLOB handling bugs that cause DPI-1080/ORA-03135.
fn fetch_ddl(conn: &Connection, obj_type: &str, name: &str, schema: &str) -> DbResult<String> {
    // Read CLOB in chunks of 4000 chars using DBMS_LOB.SUBSTR
    let sql = "SELECT DBMS_LOB.SUBSTR(DBMS_METADATA.GET_DDL(:1, :2, :3), 4000, 1 + (LEVEL-1)*4000) chunk \
               FROM DUAL \
               CONNECT BY LEVEL <= CEIL(DBMS_LOB.GETLENGTH(DBMS_METADATA.GET_DDL(:1, :2, :3)) / 4000)";

    let rows = conn
        .query(sql, &[&obj_type, &name, &schema])
        .map_err(|e| DbError::QueryFailed(e.to_string()))?;

    let mut result = String::new();
    for row_result in rows {
        let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
        let chunk: Option<String> = row.get(0).unwrap_or(None);
        if let Some(c) = chunk {
            result.push_str(&c);
        }
    }
    Ok(result.trim().to_string())
}

/// Prepend "CREATE OR REPLACE" to source code from ALL_SOURCE.
/// ALL_SOURCE returns e.g. "PACKAGE test AS..." — this simply prepends
/// "CREATE OR REPLACE" before the existing first line which already
/// contains the object type and name.
fn add_create_prefix(source: &str) -> String {
    format!("CREATE OR REPLACE {source}")
}

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
/// Uses two connections: `conn` for user queries (execute, streaming) and
/// `meta_conn` for metadata (tree, DDL, source code) to avoid ORA-03135.
/// Stores credentials to auto-reconnect on stale connections.
pub struct OracleAdapter {
    conn: Arc<Mutex<Connection>>,
    meta_conn: Arc<Mutex<Connection>>,
    username: String,
}

impl OracleAdapter {
    pub async fn connect(username: &str, password: &str, connect_string: &str) -> DbResult<Self> {
        let user = username.to_string();
        let pass = password.to_string();
        let cs = connect_string.to_string();

        let (conn, meta_conn) = task::spawn_blocking(move || {
            let c1 = Connection::connect(&user, &pass, &cs)
                .map_err(|e| DbError::ConnectionFailed(e.to_string()))?;
            let c2 = Connection::connect(&user, &pass, &cs)
                .map_err(|e| DbError::ConnectionFailed(e.to_string()))?;
            Ok::<_, DbError>((c1, c2))
        })
        .await
        .map_err(|e| DbError::ConnectionFailed(format!("Task join failed: {e}")))??;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            meta_conn: Arc::new(Mutex::new(meta_conn)),
            username: username.to_uppercase(),
        })
    }

    fn is_own_schema(&self, schema: &str) -> bool {
        self.username.eq_ignore_ascii_case(schema)
    }
}

#[async_trait]
impl DatabaseAdapter for OracleAdapter {
    fn name(&self) -> &str {
        "Oracle"
    }

    fn db_type(&self) -> DatabaseType {
        DatabaseType::Oracle
    }

    async fn get_table_ddl(&self, schema: &str, table: &str) -> DbResult<String> {
        let conn = Arc::clone(&self.meta_conn);
        let schema = schema.to_uppercase();
        let table = table.to_uppercase();
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            fetch_ddl(&conn, "TABLE", &table, &schema)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_schemas(&self) -> DbResult<Vec<Schema>> {
        let conn = Arc::clone(&self.meta_conn);
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
        let conn = Arc::clone(&self.meta_conn);
        let schema_owned = schema.to_string();
        let is_own = self.is_own_schema(schema);
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = conn
                .query(
                    "SELECT t.table_name, \
                     MAX(CASE WHEN p.privilege = 'SELECT' THEN 1 ELSE 0 END) as can_select, \
                     MAX(CASE WHEN p.privilege IN ('INSERT','UPDATE','DELETE') THEN 1 ELSE 0 END) as can_modify \
                     FROM all_tables t \
                     LEFT JOIN all_tab_privs p \
                       ON p.table_schema = t.owner AND p.table_name = t.table_name \
                       AND (p.grantee = SYS_CONTEXT('USERENV','SESSION_USER') OR p.grantee = 'PUBLIC') \
                     WHERE t.owner = :1 \
                     GROUP BY t.table_name \
                     ORDER BY t.table_name",
                    &[&schema_owned],
                )
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let mut results = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let name: String = row.get(0).unwrap_or_default();
                let privilege = if is_own {
                    ObjectPrivilege::Full
                } else {
                    let can_select: i32 = row.get(1).unwrap_or(0);
                    let can_modify: i32 = row.get(2).unwrap_or(0);
                    if can_select == 1 && can_modify == 1 {
                        ObjectPrivilege::Full
                    } else if can_select == 1 {
                        ObjectPrivilege::ReadOnly
                    } else {
                        ObjectPrivilege::Unknown
                    }
                };
                results.push(Table {
                    name,
                    schema: schema_owned.clone(),
                    privilege,
                });
            }
            Ok(results)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_views(&self, schema: &str) -> DbResult<Vec<View>> {
        let conn = Arc::clone(&self.meta_conn);
        let schema_owned = schema.to_string();
        let is_own = self.is_own_schema(schema);
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
                    privilege: if is_own { ObjectPrivilege::Full } else { ObjectPrivilege::ReadOnly },
                });
            }
            Ok(results)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_packages(&self, schema: &str) -> DbResult<Vec<Package>> {
        let conn = Arc::clone(&self.meta_conn);
        let schema_owned = schema.to_string();
        let is_own = self.is_own_schema(schema);
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
                    privilege: if is_own {
                        ObjectPrivilege::Full
                    } else {
                        ObjectPrivilege::Execute
                    },
                });
            }
            Ok(packages)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_procedures(&self, schema: &str) -> DbResult<Vec<Procedure>> {
        let conn = Arc::clone(&self.meta_conn);
        let schema_owned = schema.to_string();
        let is_own = self.is_own_schema(schema);
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
                    privilege: if is_own {
                        ObjectPrivilege::Full
                    } else {
                        ObjectPrivilege::Execute
                    },
                });
            }
            Ok(results)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_functions(&self, schema: &str) -> DbResult<Vec<Function>> {
        let conn = Arc::clone(&self.meta_conn);
        let schema_owned = schema.to_string();
        let is_own = self.is_own_schema(schema);
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
                    privilege: if is_own {
                        ObjectPrivilege::Full
                    } else {
                        ObjectPrivilege::Execute
                    },
                });
            }
            Ok(results)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_materialized_views(&self, schema: &str) -> DbResult<Vec<MaterializedView>> {
        let conn = Arc::clone(&self.meta_conn);
        let schema = schema.to_string();
        let is_own = self.is_own_schema(&schema);
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = if is_own {
                conn.query(
                    "SELECT o.object_name, o.status \
                     FROM user_objects o \
                     WHERE o.object_type = 'MATERIALIZED VIEW' \
                     ORDER BY o.object_name",
                    &[] as &[&dyn oracle::sql_type::ToSql],
                )
            } else {
                conn.query(
                    "SELECT o.object_name, o.status \
                     FROM all_objects o \
                     WHERE o.owner = :1 AND o.object_type = 'MATERIALIZED VIEW' \
                     ORDER BY o.object_name",
                    &[&schema],
                )
            }
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let mut result = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let name: String = row
                    .get(0)
                    .map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let status: String = row.get(1).unwrap_or_default();
                result.push(MaterializedView {
                    name,
                    schema: schema.clone(),
                    valid: status == "VALID",
                    privilege: if is_own {
                        ObjectPrivilege::Full
                    } else {
                        ObjectPrivilege::ReadOnly
                    },
                });
            }
            Ok(result)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_indexes(&self, schema: &str) -> DbResult<Vec<Index>> {
        let conn = Arc::clone(&self.meta_conn);
        let schema = schema.to_string();
        let is_own = self.is_own_schema(&schema);
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = if is_own {
                conn.query(
                    "SELECT index_name FROM user_indexes \
                     WHERE index_type != 'LOB' ORDER BY index_name",
                    &[] as &[&dyn oracle::sql_type::ToSql],
                )
            } else {
                conn.query(
                    "SELECT index_name FROM all_indexes \
                     WHERE owner = :1 AND index_type != 'LOB' ORDER BY index_name",
                    &[&schema],
                )
            }
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let mut result = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let name: String = row
                    .get(0)
                    .map_err(|e| DbError::QueryFailed(e.to_string()))?;
                result.push(Index {
                    name,
                    schema: schema.clone(),
                });
            }
            Ok(result)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_sequences(&self, schema: &str) -> DbResult<Vec<Sequence>> {
        let conn = Arc::clone(&self.meta_conn);
        let schema = schema.to_string();
        let is_own = self.is_own_schema(&schema);
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = if is_own {
                conn.query(
                    "SELECT sequence_name FROM user_sequences ORDER BY sequence_name",
                    &[] as &[&dyn oracle::sql_type::ToSql],
                )
            } else {
                conn.query(
                    "SELECT sequence_name FROM all_sequences \
                     WHERE sequence_owner = :1 ORDER BY sequence_name",
                    &[&schema],
                )
            }
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let mut result = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let name: String = row
                    .get(0)
                    .map_err(|e| DbError::QueryFailed(e.to_string()))?;
                result.push(Sequence {
                    name,
                    schema: schema.clone(),
                });
            }
            Ok(result)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_types(&self, schema: &str) -> DbResult<Vec<DbType>> {
        let conn = Arc::clone(&self.meta_conn);
        let schema = schema.to_string();
        let is_own = self.is_own_schema(&schema);
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = if is_own {
                conn.query(
                    "SELECT type_name FROM user_types ORDER BY type_name",
                    &[] as &[&dyn oracle::sql_type::ToSql],
                )
            } else {
                conn.query(
                    "SELECT type_name FROM all_types \
                     WHERE owner = :1 ORDER BY type_name",
                    &[&schema],
                )
            }
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let mut result = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let name: String = row
                    .get(0)
                    .map_err(|e| DbError::QueryFailed(e.to_string()))?;
                result.push(DbType {
                    name,
                    schema: schema.clone(),
                });
            }
            Ok(result)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_triggers(&self, schema: &str) -> DbResult<Vec<Trigger>> {
        let conn = Arc::clone(&self.meta_conn);
        let schema = schema.to_string();
        let is_own = self.is_own_schema(&schema);
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = if is_own {
                conn.query(
                    "SELECT trigger_name FROM user_triggers ORDER BY trigger_name",
                    &[] as &[&dyn oracle::sql_type::ToSql],
                )
            } else {
                conn.query(
                    "SELECT trigger_name FROM all_triggers \
                     WHERE owner = :1 ORDER BY trigger_name",
                    &[&schema],
                )
            }
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let mut result = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let name: String = row
                    .get(0)
                    .map_err(|e| DbError::QueryFailed(e.to_string()))?;
                result.push(Trigger {
                    name,
                    schema: schema.clone(),
                });
            }
            Ok(result)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_columns(&self, schema: &str, table: &str) -> DbResult<Vec<Column>> {
        let conn = Arc::clone(&self.meta_conn);
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
        let conn = Arc::clone(&self.meta_conn);
        let schema_owned = schema.to_string();
        let name_owned = name.to_string();
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();

            let declaration = match fetch_source(&conn, &schema_owned, &name_owned, "PACKAGE")? {
                Some(d) => add_create_prefix(&d),
                None => return Ok(None),
            };
            let body = fetch_source(&conn, &schema_owned, &name_owned, "PACKAGE BODY")?
                .map(|b| add_create_prefix(&b));

            Ok(Some(PackageContent { declaration, body }))
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_type_attributes(&self, schema: &str, name: &str) -> DbResult<QueryResult> {
        let conn = Arc::clone(&self.meta_conn);
        let schema = schema.to_uppercase();
        let name = name.to_uppercase();
        let is_own = self.is_own_schema(&schema);
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = if is_own {
                conn.query(
                    "SELECT attr_no, attr_name, attr_type_name, attr_type_mod, length \
                     FROM user_type_attrs \
                     WHERE type_name = :1 ORDER BY attr_no",
                    &[&name],
                )
            } else {
                conn.query(
                    "SELECT attr_no, attr_name, attr_type_name, attr_type_mod, length \
                     FROM all_type_attrs \
                     WHERE owner = :1 AND type_name = :2 ORDER BY attr_no",
                    &[&schema, &name],
                )
            }
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let columns = vec![
                "#".to_string(),
                "Name".to_string(),
                "Type".to_string(),
                "Type Mod".to_string(),
                "Length".to_string(),
            ];
            let mut data_rows = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let no: i32 = row.get(0).unwrap_or(0);
                let attr_name: String = row.get(1).unwrap_or_default();
                let type_name: String = row.get(2).unwrap_or_default();
                let type_mod: Option<String> = row.get(3).unwrap_or(None);
                let length: Option<i32> = row.get(4).unwrap_or(None);
                data_rows.push(vec![
                    no.to_string(),
                    attr_name,
                    type_name,
                    type_mod.unwrap_or_default(),
                    length.map(|l| l.to_string()).unwrap_or_default(),
                ]);
            }
            Ok(QueryResult {
                columns,
                rows: data_rows,
                elapsed: None,
            })
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_type_methods(&self, schema: &str, name: &str) -> DbResult<QueryResult> {
        let conn = Arc::clone(&self.meta_conn);
        let schema = schema.to_uppercase();
        let name = name.to_uppercase();
        let is_own = self.is_own_schema(&schema);
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = if is_own {
                conn.query(
                    "SELECT method_name, method_type, \
                     NVL(result_type_name, '') as result_type, \
                     NVL(result_type_mod, '') as result_mod, \
                     final, instantiable \
                     FROM user_type_methods \
                     WHERE type_name = :1 ORDER BY method_no",
                    &[&name],
                )
            } else {
                conn.query(
                    "SELECT method_name, method_type, \
                     NVL(result_type_name, '') as result_type, \
                     NVL(result_type_mod, '') as result_mod, \
                     final, instantiable \
                     FROM all_type_methods \
                     WHERE owner = :1 AND type_name = :2 ORDER BY method_no",
                    &[&schema, &name],
                )
            }
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let columns = vec![
                "Name".to_string(),
                "Method Type".to_string(),
                "Result".to_string(),
                "Result Mod".to_string(),
                "Final".to_string(),
                "Instantiable".to_string(),
            ];
            let mut data_rows = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let method_name: String = row.get(0).unwrap_or_default();
                let method_type: String = row.get(1).unwrap_or_default();
                let result_type: String = row.get(2).unwrap_or_default();
                let result_mod: String = row.get(3).unwrap_or_default();
                let final_flag: String = row.get(4).unwrap_or_default();
                let instantiable: String = row.get(5).unwrap_or_default();
                data_rows.push(vec![
                    method_name,
                    method_type,
                    result_type,
                    result_mod,
                    final_flag,
                    instantiable,
                ]);
            }
            Ok(QueryResult {
                columns,
                rows: data_rows,
                elapsed: None,
            })
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_trigger_info(&self, schema: &str, name: &str) -> DbResult<QueryResult> {
        let conn = Arc::clone(&self.meta_conn);
        let schema = schema.to_uppercase();
        let name = name.to_uppercase();
        let is_own = self.is_own_schema(&schema);
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let rows = if is_own {
                conn.query(
                    "SELECT column_name, column_usage \
                     FROM user_trigger_cols \
                     WHERE trigger_name = :1 ORDER BY column_name",
                    &[&name],
                )
            } else {
                conn.query(
                    "SELECT column_name, column_usage \
                     FROM all_trigger_cols \
                     WHERE trigger_owner = :1 AND trigger_name = :2 ORDER BY column_name",
                    &[&schema, &name],
                )
            }
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let columns = vec!["Name".to_string(), "Usage".to_string()];
            let mut data_rows = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let col_name: String = row.get(0).unwrap_or_default();
                let usage: String = row.get(1).unwrap_or_default();
                data_rows.push(vec![col_name, usage]);
            }
            Ok(QueryResult {
                columns,
                rows: data_rows,
                elapsed: None,
            })
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_source_code(&self, schema: &str, name: &str, obj_type: &str) -> DbResult<String> {
        let conn = Arc::clone(&self.meta_conn);
        let schema = schema.to_uppercase();
        let name = name.to_uppercase();
        let obj_type = obj_type.to_uppercase();
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            match obj_type.as_str() {
                "FUNCTION" | "PROCEDURE" => Ok(fetch_source(&conn, &schema, &name, &obj_type)?
                    .map(|s| add_create_prefix(&s))
                    .unwrap_or_default()),
                "INDEX" | "SEQUENCE" | "TRIGGER" | "TYPE" | "TYPE_BODY" => {
                    fetch_ddl(&conn, &obj_type, &name, &schema)
                }
                _ => Ok(String::new()),
            }
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn execute(&self, query: &str) -> DbResult<QueryResult> {
        let conn = Arc::clone(&self.conn);
        let query_owned = query.to_string();
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();

            // DDL/DML: use execute() instead of query()
            let trimmed = query_owned.trim_start().to_uppercase();
            if !trimmed.starts_with("SELECT") && !trimmed.starts_with("WITH") {
                conn.execute(&query_owned, &[] as &[&dyn oracle::sql_type::ToSql])
                    .map_err(|e| DbError::QueryFailed(e.to_string()))?;
                conn.commit()
                    .map_err(|e| DbError::QueryFailed(e.to_string()))?;
                return Ok(QueryResult {
                    columns: vec!["Result".to_string()],
                    rows: vec![vec!["Statement executed successfully".to_string()]],
                    elapsed: None,
                });
            }

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
                elapsed: None,
            })
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn execute_streaming(
        &self,
        query: &str,
        tx: mpsc::Sender<DbResult<QueryBatch>>,
    ) -> DbResult<()> {
        const BATCH_SIZE: usize = 500;

        let conn = Arc::clone(&self.conn);
        let query_owned = query.to_string();
        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();

            // DDL/DML: execute and return a single "success" batch
            let trimmed = query_owned.trim_start().to_uppercase();
            if !trimmed.starts_with("SELECT") && !trimmed.starts_with("WITH") {
                conn.execute(&query_owned, &[] as &[&dyn oracle::sql_type::ToSql])
                    .map_err(|e| DbError::QueryFailed(e.to_string()))?;
                conn.commit()
                    .map_err(|e| DbError::QueryFailed(e.to_string()))?;
                let _ = tx.blocking_send(Ok(QueryBatch {
                    columns: vec!["Result".to_string()],
                    rows: vec![vec!["Statement executed successfully".to_string()]],
                    done: true,
                }));
                return Ok(());
            }

            let mut stmt = conn
                .statement(&query_owned)
                .build()
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;
            let rows = stmt
                .query(&[] as &[&dyn oracle::sql_type::ToSql])
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;

            let column_info = rows.column_info();
            let columns: Vec<String> = column_info.iter().map(|c| c.name().to_string()).collect();

            let mut batch = Vec::with_capacity(BATCH_SIZE);
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
                batch.push(row_data);

                if batch.len() >= BATCH_SIZE {
                    let rows = std::mem::replace(&mut batch, Vec::with_capacity(BATCH_SIZE));
                    if tx
                        .blocking_send(Ok(QueryBatch {
                            columns: columns.clone(),
                            rows,
                            done: false,
                        }))
                        .is_err()
                    {
                        return Ok(());
                    }
                }
            }

            let _ = tx.blocking_send(Ok(QueryBatch {
                columns,
                rows: batch,
                done: true,
            }));

            Ok(())
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn get_foreign_keys(&self, schema: &str, table: &str) -> DbResult<Vec<ForeignKeyInfo>> {
        let conn = Arc::clone(&self.meta_conn);
        let schema_owned = schema.to_string();
        let table_owned = table.to_string();

        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let sql = "SELECT acc.column_name, \
                              rac.owner AS ref_schema, \
                              rac.table_name AS ref_table, \
                              racc.column_name AS ref_column, \
                              ac.constraint_name \
                       FROM all_constraints ac \
                       JOIN all_cons_columns acc \
                         ON ac.constraint_name = acc.constraint_name \
                        AND ac.owner = acc.owner \
                       JOIN all_constraints rac \
                         ON ac.r_constraint_name = rac.constraint_name \
                        AND ac.r_owner = rac.owner \
                       JOIN all_cons_columns racc \
                         ON rac.constraint_name = racc.constraint_name \
                        AND rac.owner = racc.owner \
                        AND acc.position = racc.position \
                       WHERE ac.owner = :1 \
                         AND ac.table_name = :2 \
                         AND ac.constraint_type = 'R' \
                       ORDER BY ac.constraint_name, acc.position";

            let rows = conn
                .query(sql, &[&schema_owned, &table_owned])
                .map_err(|e| DbError::QueryFailed(e.to_string()))?;

            let mut fks = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| DbError::QueryFailed(e.to_string()))?;
                fks.push(ForeignKeyInfo {
                    column_name: row.get::<usize, String>(0).unwrap_or_default(),
                    referenced_schema: row.get::<usize, String>(1).unwrap_or_default(),
                    referenced_table: row.get::<usize, String>(2).unwrap_or_default(),
                    referenced_column: row.get::<usize, String>(3).unwrap_or_default(),
                    constraint_name: row.get::<usize, String>(4).unwrap_or_default(),
                });
            }
            Ok(fks)
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }

    async fn compile_check(&self, sql: &str) -> DbResult<Vec<CompileDiagnostic>> {
        let conn = Arc::clone(&self.meta_conn);
        let sql_owned = sql.to_string();

        task::spawn_blocking(move || {
            let conn = conn.blocking_lock();

            // Try to compile the SQL; capture the error if it fails
            let result = conn.execute(
                &sql_owned,
                &[] as &[&dyn oracle::sql_type::ToSql],
            );

            match result {
                Ok(_) => {
                    // DDL auto-commits; rollback not possible.
                    // Check USER_ERRORS for any compilation warnings.
                    let err_rows = conn
                        .query(
                            "SELECT line, position, text, attribute \
                             FROM user_errors \
                             WHERE name = (SELECT object_name FROM user_objects \
                                           WHERE object_id = (SELECT MAX(object_id) FROM user_objects)) \
                             ORDER BY sequence",
                            &[] as &[&dyn oracle::sql_type::ToSql],
                        );

                    match err_rows {
                        Ok(rows) => {
                            let mut diags = Vec::new();
                            for row_result in rows.flatten() {
                                diags.push(CompileDiagnostic {
                                    line: row_result.get::<usize, i32>(0).unwrap_or(0) as usize,
                                    col: row_result.get::<usize, i32>(1).unwrap_or(0) as usize,
                                    message: row_result
                                        .get::<usize, String>(2)
                                        .unwrap_or_default(),
                                    severity: row_result
                                        .get::<usize, String>(3)
                                        .unwrap_or_default(),
                                });
                            }
                            Ok(diags)
                        }
                        Err(_) => Ok(vec![]),
                    }
                }
                Err(e) => {
                    // Parse Oracle error: ORA-XXXXX at line N, column M
                    let msg = e.to_string();
                    Ok(vec![CompileDiagnostic {
                        line: 1,
                        col: 1,
                        message: msg,
                        severity: "ERROR".to_string(),
                    }])
                }
            }
        })
        .await
        .map_err(|e| DbError::QueryFailed(format!("Task join failed: {e}")))?
    }
}
