//! Execution of user statements on MySQL.
//!
//! Statements are sent as plain text (`COM_QUERY`). sqlx only uses the
//! prepared-statement protocol for queries built with `sqlx::query`; a bare
//! `&str` goes out unprepared. That matters twice over:
//!
//! * the server refuses to prepare `CREATE PROCEDURE/FUNCTION/TRIGGER/EVENT`,
//!   `USE`, `LOCK TABLES` and friends, but runs them as text;
//! * a text response says for itself whether it carries a result set or a
//!   row count, so `SHOW`, `DESCRIBE`, `EXPLAIN` and `CALL` need no keyword
//!   guessing to get their rows shown.

use futures_util::TryStreamExt;
use sqlx::mysql::{MySqlConnection, MySqlPool};
use sqlx::{Column as _, Either, Executor, Row, Statement};

use crate::core::error::DbResult;
use crate::core::models::DatabaseType;
use crate::drivers::mysql::error::{plain_error, statement_error};
use crate::drivers::mysql::value::mysql_value_to_string;
use crate::drivers::sink::{RowSink, RowWriter, success_message};
use crate::drivers::statement::is_row_producing;

/// How a statement ended.
enum Completion {
    /// It returned a result set (possibly empty); the rows are in the writer.
    Rows,
    /// It returned no result set, only a row count.
    Affected(u64),
    /// The receiver hung up before the last row was read.
    ReceiverGone,
}

/// Run one user statement and deliver its outcome to `sink`.
///
/// The statement runs inside an explicit transaction so DML is committed
/// regardless of the server's autocommit setting. A receiver that hangs up
/// mid-stream drops the transaction, which rolls it back.
pub(super) async fn run(pool: &MySqlPool, sql: &str, sink: RowSink<'_>) -> DbResult<()> {
    let mut writer = RowWriter::new(sink);
    let mut tx = pool.begin().await.map_err(|e| plain_error(&e))?;

    match run_statement(&mut tx, sql, &mut writer).await {
        Ok(Completion::ReceiverGone) => return Ok(()),
        Ok(completion) => {
            tx.commit().await.map_err(|e| statement_error(&e))?;
            if let Completion::Affected(n) = completion {
                writer.set_message(success_message(Some(n)));
            }
        }
        Err(e) => {
            let _ = tx.rollback().await;
            return Err(statement_error(&e));
        }
    }

    writer.finish().await;
    Ok(())
}

/// Send `sql` on `conn` and feed the rows of its first result set to `writer`.
async fn run_statement(
    conn: &mut MySqlConnection,
    sql: &str,
    writer: &mut RowWriter<'_>,
) -> Result<Completion, sqlx::Error> {
    let mut affected = 0u64;
    {
        let mut stream = (&mut *conn).fetch_many(sql);
        // A stored procedure can return several result sets. Only the first
        // is shown; the rest are still read so the connection stays in sync.
        let mut first_set_closed = false;
        while let Some(item) = stream.try_next().await? {
            match item {
                Either::Left(done) => {
                    affected += done.rows_affected();
                    first_set_closed = writer.has_columns();
                }
                Either::Right(_) if first_set_closed => {}
                Either::Right(row) => {
                    if !writer.has_columns() {
                        writer.set_columns(column_names(row.columns()));
                    }
                    let cells = (0..row.len())
                        .map(|i| mysql_value_to_string(&row, i))
                        .collect();
                    if !writer.push(cells).await {
                        return Ok(Completion::ReceiverGone);
                    }
                }
            }
        }
    }

    if writer.has_columns() {
        return Ok(Completion::Rows);
    }
    if let Some(columns) = empty_result_columns(conn, sql).await {
        writer.set_columns(columns);
        return Ok(Completion::Rows);
    }
    Ok(Completion::Affected(affected))
}

/// Column headers for a result set that came back without rows.
///
/// The text protocol only exposes column metadata through a row, so an empty
/// result looks the same as a statement that returns none. When the statement
/// reads like a query, preparing it (without executing) recovers the headers.
/// Statements the server will not prepare simply get no headers.
async fn empty_result_columns(conn: &mut MySqlConnection, sql: &str) -> Option<Vec<String>> {
    if !is_row_producing(DatabaseType::MySQL, sql) {
        return None;
    }
    let statement = (&mut *conn).prepare(sql).await.ok()?;
    let columns = column_names(statement.columns());
    (!columns.is_empty()).then_some(columns)
}

fn column_names(columns: &[sqlx::mysql::MySqlColumn]) -> Vec<String> {
    columns.iter().map(|c| c.name().to_string()).collect()
}
