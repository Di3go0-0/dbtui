//! Execution of user statements on Oracle.
//!
//! Everything here is synchronous: the `oracle` crate blocks, so the adapter
//! calls into this module from `spawn_blocking` with the connection locked.

use oracle::sql_type::ToSql;
use oracle::{Connection, Statement};

use crate::core::error::{DbError, DbResult};
use crate::drivers::oracle::compile::compile_failure;
use crate::drivers::oracle::error::{fetch_error, statement_error};
use crate::drivers::oracle::value::oracle_col_to_string;
use crate::drivers::sink::{RowSink, RowWriter, success_message};

const NO_PARAMS: &[&dyn ToSql] = &[];

/// Run one user statement and deliver its outcome to `sink`.
///
/// `schema`, when set, becomes the session's `CURRENT_SCHEMA` for the
/// duration of the statement, so unqualified names resolve there.
pub(super) fn run(
    conn: &Connection,
    sql: &str,
    schema: Option<&str>,
    sink: RowSink<'_>,
) -> DbResult<()> {
    // Declared first so it is dropped last: the session is back on its own
    // schema on every way out of this function — success, error, a receiver
    // that hung up, or a panic unwinding through it.
    let _schema = match schema.filter(|s| !s.is_empty()) {
        Some(schema) => SchemaGuard::enter(conn, schema)?,
        None => None,
    };

    let mut writer = RowWriter::new(sink);
    let mut stmt = conn
        .statement(sql)
        .build()
        .map_err(|e| statement_error(&e, sql))?;

    // The statement type comes from the Oracle client's own parse, so there
    // is no keyword guessing: `query` is only valid for a SELECT (including
    // `WITH` and parenthesised forms) and `execute` for everything else.
    if stmt.is_query() {
        if !fetch_rows(&mut stmt, sql, &mut writer)? {
            return Ok(());
        }
    } else {
        let affected = execute_statement(conn, &mut stmt, sql)?;
        writer.set_message(success_message(Some(affected)));
    }

    writer.finish_blocking();
    Ok(())
}

/// Run a query and feed its rows to `writer`. Returns `false` when the
/// receiver hung up before the last row.
fn fetch_rows(stmt: &mut Statement, sql: &str, writer: &mut RowWriter<'_>) -> DbResult<bool> {
    let rows = stmt
        .query(NO_PARAMS)
        .map_err(|e| statement_error(&e, sql))?;

    let columns: Vec<String> = rows
        .column_info()
        .iter()
        .map(|c| c.name().to_string())
        .collect();
    let column_count = columns.len();
    writer.set_columns(columns);

    for row in rows {
        let row = row.map_err(|e| fetch_error(&e, sql))?;
        let cells = (0..column_count)
            .map(|i| oracle_col_to_string(&row, i))
            .collect();
        if !writer.push_blocking(cells) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Run DML, DDL or PL/SQL, commit, and return the affected row count.
///
/// A `CREATE` of a stored unit that compiled with errors is reported as a
/// failure even though Oracle answered with success.
fn execute_statement(conn: &Connection, stmt: &mut Statement, sql: &str) -> DbResult<u64> {
    stmt.execute(NO_PARAMS)
        .map_err(|e| statement_error(&e, sql))?;
    let affected = stmt.row_count().unwrap_or(0);

    // Read before the commit: the warning belongs to the last execution.
    let compile_error = compile_failure(conn, sql);

    conn.commit().map_err(|e| statement_error(&e, sql))?;

    match compile_error {
        Some(err) => Err(err),
        None => Ok(affected),
    }
}

/// Points the session at another schema and puts it back when dropped.
///
/// The connection is shared by every tab, so a `CURRENT_SCHEMA` left behind
/// would silently redirect the next statement that runs on it.
struct SchemaGuard<'a> {
    conn: &'a Connection,
    previous: String,
}

impl<'a> SchemaGuard<'a> {
    /// Switch to `schema`. Returns `None` when the session is already there,
    /// in which case there is nothing to restore.
    fn enter(conn: &'a Connection, schema: &str) -> DbResult<Option<Self>> {
        let previous = current_schema(conn)?;
        if previous == schema {
            return Ok(None);
        }
        switch_schema(conn, schema)?;
        Ok(Some(Self { conn, previous }))
    }
}

impl Drop for SchemaGuard<'_> {
    fn drop(&mut self) {
        // Nothing useful can be done if this fails: the name was read from
        // the session itself, so a failure means the connection is gone.
        let _ = set_current_schema(self.conn, &self.previous);
    }
}

fn current_schema(conn: &Connection) -> DbResult<String> {
    conn.query_row_as::<String>(
        "SELECT SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA') FROM DUAL",
        NO_PARAMS,
    )
    .map_err(|e| DbError::QueryFailed(format!("Could not read the current schema: {e}")))
}

/// Switch to `schema` as the sidebar lists it. A name that only differs in
/// case from the dictionary's is retried folded to upper case, the way an
/// unquoted identifier would resolve.
fn switch_schema(conn: &Connection, schema: &str) -> DbResult<()> {
    let exact = set_current_schema(conn, schema);
    let upper = schema.to_uppercase();
    match exact {
        Err(_) if upper != schema => set_current_schema(conn, &upper),
        other => other,
    }
}

fn set_current_schema(conn: &Connection, schema: &str) -> DbResult<()> {
    let sql = set_schema_sql(schema)?;
    conn.execute(&sql, NO_PARAMS)
        .map(|_| ())
        .map_err(|e| DbError::QueryFailed(format!("Could not switch to schema {schema}: {e}")))
}

/// `ALTER SESSION SET CURRENT_SCHEMA` takes an identifier, not a bind
/// variable. Oracle has no escape for a double quote inside a quoted
/// identifier — such a name cannot exist — so one is rejected outright.
fn set_schema_sql(schema: &str) -> DbResult<String> {
    if schema.is_empty() || schema.contains(['"', '\0']) {
        return Err(DbError::QueryFailed(format!(
            "Invalid schema name: {schema:?}"
        )));
    }
    Ok(format!("ALTER SESSION SET CURRENT_SCHEMA = \"{schema}\""))
}

#[cfg(test)]
mod tests {
    use super::set_schema_sql;

    #[test]
    fn schema_is_quoted_as_listed() {
        assert_eq!(
            set_schema_sql("HR").ok().as_deref(),
            Some("ALTER SESSION SET CURRENT_SCHEMA = \"HR\"")
        );
        assert_eq!(
            set_schema_sql("MixedCase").ok().as_deref(),
            Some("ALTER SESSION SET CURRENT_SCHEMA = \"MixedCase\"")
        );
    }

    #[test]
    fn names_that_could_break_out_of_the_quotes_are_rejected() {
        assert!(set_schema_sql("HR\" ; DROP USER x --").is_err());
        assert!(set_schema_sql("a\0b").is_err());
        assert!(set_schema_sql("").is_err());
    }
}
