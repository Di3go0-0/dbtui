//! Source code and DDL retrieval for Oracle objects.

use oracle::Connection;

use crate::core::error::{DbError, DbResult};

/// Characters fetched per `DBMS_LOB.SUBSTR` call.
///
/// `SUBSTR` counts characters while the VARCHAR2 it returns is limited to
/// 4000 *bytes*. A character takes up to four bytes in AL32UTF8, so 1000 is
/// the largest chunk that can never overflow (ORA-06502) or be cut short.
const DDL_CHUNK_CHARS: usize = 1000;

/// Four bytes per character at most, 4000 bytes per VARCHAR2.
const _: () = assert!(DDL_CHUNK_CHARS * 4 <= 4000);

/// Translate raw Oracle errors from DBMS_METADATA.GET_DDL into user-friendly
/// messages. ORA-31603 in particular is misleading: it says "object not found"
/// but in practice it almost always means the current user lacks privileges
/// to view the metadata of that object.
fn humanize_ddl_error(err: &str, obj_type: &str, name: &str, schema: &str) -> String {
    if err.contains("ORA-31603") {
        format!(
            "Insufficient privileges to read DDL for {obj_type} \"{schema}\".\"{name}\". \
             Your current Oracle user doesn't have the rights to call \
             DBMS_METADATA.GET_DDL on objects in schema \"{schema}\". \
             Ask the DBA for SELECT_CATALOG_ROLE or the SELECT_ANY_DICTIONARY \
             privilege if you need to inspect this object's DDL."
        )
    } else if err.contains("ORA-31604") {
        format!(
            "Invalid argument when fetching DDL for {obj_type} \"{schema}\".\"{name}\". \
             The object type may not be supported by DBMS_METADATA.GET_DDL."
        )
    } else if err.contains("ORA-00942") {
        format!("Table or view \"{schema}\".\"{name}\" does not exist or you have no access to it.")
    } else {
        format!("DDL fetch failed for {obj_type} \"{schema}\".\"{name}\": {err}")
    }
}

/// The chunked `DBMS_METADATA.GET_DDL` query.
fn ddl_chunk_sql() -> String {
    format!(
        "SELECT DBMS_LOB.SUBSTR(DBMS_METADATA.GET_DDL(:1, :2, :3), {n}, 1 + (LEVEL-1)*{n}) chunk \
         FROM DUAL \
         CONNECT BY LEVEL <= CEIL(DBMS_LOB.GETLENGTH(DBMS_METADATA.GET_DDL(:1, :2, :3)) / {n})",
        n = DDL_CHUNK_CHARS
    )
}

/// Fetch DDL via DBMS_METADATA, reading the CLOB in chunks server-side
/// to avoid ODPI-C CLOB handling bugs that cause DPI-1080/ORA-03135.
///
/// `name` and `schema` are passed exactly as the dictionary lists them, so
/// objects created with quoted mixed-case names are found.
pub(super) fn fetch_ddl(
    conn: &Connection,
    obj_type: &str,
    name: &str,
    schema: &str,
) -> DbResult<String> {
    let sql = ddl_chunk_sql();

    let rows = conn
        .query(&sql, &[&obj_type, &name, &schema])
        .map_err(|e| {
            DbError::QueryFailed(humanize_ddl_error(&e.to_string(), obj_type, name, schema))
        })?;

    let mut result = String::new();
    for row_result in rows {
        let row = row_result.map_err(|e| {
            DbError::QueryFailed(humanize_ddl_error(&e.to_string(), obj_type, name, schema))
        })?;
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
pub(super) fn add_create_prefix(source: &str) -> String {
    format!("CREATE OR REPLACE {source}")
}

/// Fetch source code row-by-row from ALL_SOURCE and concatenate in Rust.
/// Avoids CLOB buffer issues in the oracle crate that can truncate large packages.
pub(super) fn fetch_source(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_size_is_used_for_length_offset_and_count() {
        let sql = ddl_chunk_sql();
        assert_eq!(sql.matches("1000").count(), 3);
        assert!(!sql.contains("4000"));
        assert!(sql.contains(
            "DBMS_LOB.SUBSTR(DBMS_METADATA.GET_DDL(:1, :2, :3), 1000, 1 + (LEVEL-1)*1000)"
        ));
    }

    #[test]
    fn create_prefix_is_prepended() {
        assert_eq!(
            add_create_prefix("PROCEDURE p IS BEGIN NULL; END;"),
            "CREATE OR REPLACE PROCEDURE p IS BEGIN NULL; END;"
        );
    }

    #[test]
    fn privilege_error_is_explained() {
        let msg = humanize_ddl_error(
            "ORA-31603: object \"T\" of type TABLE not found",
            "TABLE",
            "T",
            "HR",
        );
        assert!(msg.starts_with("Insufficient privileges"));
        assert!(msg.contains("\"HR\".\"T\""));
    }
}
