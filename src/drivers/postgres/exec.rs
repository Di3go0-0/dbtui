//! Execution of user statements on PostgreSQL.
//!
//! A statement is prepared first, and the server's description of it decides
//! what it is: a statement that describes result columns returns rows
//! (`SELECT`, `EXPLAIN`, `SHOW`, `INSERT ... RETURNING`, ...), anything else
//! reports a row count. Nothing is guessed from keywords, and an empty result
//! still has its column headers.

use futures_util::TryStreamExt;
use sqlx::postgres::{PgConnection, PgPool};
use sqlx::{Column as _, Either, Executor, Row, Statement};

use crate::core::error::DbResult;
use crate::drivers::postgres::error::{is_transaction_block_refusal, plain_error, statement_error};
use crate::drivers::postgres::statement::runs_outside_transaction;
use crate::drivers::postgres::value::pg_value_to_string;
use crate::drivers::sink::{RowSink, RowWriter, success_message};

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
/// `schema`, when set, becomes the statement's `search_path`. The statement
/// runs in its own transaction, committed on success, unless PostgreSQL
/// refuses to run it inside one.
pub(super) async fn run(
    pool: &PgPool,
    sql: &str,
    schema: Option<&str>,
    sink: RowSink<'_>,
) -> DbResult<()> {
    let schema = schema.filter(|s| !s.is_empty());
    let mut writer = RowWriter::new(sink);

    let completion = if runs_outside_transaction(sql) {
        run_autocommit(pool, sql, schema, &mut writer).await?
    } else {
        match run_in_transaction(pool, sql, schema, &mut writer).await? {
            Some(completion) => completion,
            // The server refused the transaction block before doing any work.
            None => run_autocommit(pool, sql, schema, &mut writer).await?,
        }
    };

    match completion {
        Completion::ReceiverGone => return Ok(()),
        Completion::Affected(n) => writer.set_message(success_message(Some(n))),
        Completion::Rows => {}
    }
    writer.finish().await;
    Ok(())
}

/// Run the statement in a transaction of its own.
///
/// The transaction is committed when the statement succeeds: row-returning
/// statements can write too (`WITH d AS (DELETE ... RETURNING *) SELECT ...`,
/// `SELECT fn_that_writes()`, `INSERT ... RETURNING`). It is rolled back on
/// error, and dropped — which also rolls back — when the receiver goes away
/// mid-stream.
///
/// Returns `None` when the statement cannot run inside a transaction block.
async fn run_in_transaction(
    pool: &PgPool,
    sql: &str,
    schema: Option<&str>,
    writer: &mut RowWriter<'_>,
) -> DbResult<Option<Completion>> {
    let mut tx = pool.begin().await.map_err(|e| plain_error(&e))?;

    if let Some(schema) = schema {
        // SET LOCAL lasts until the transaction ends, so no session state
        // leaks back into the pool.
        let set_path = format!("SET LOCAL search_path TO {}", quote_schema(schema));
        (&mut *tx)
            .execute(set_path.as_str())
            .await
            .map_err(|e| plain_error(&e))?;
    }

    match run_statement(&mut tx, sql, writer).await {
        Ok(Completion::ReceiverGone) => Ok(Some(Completion::ReceiverGone)),
        Ok(completion) => {
            tx.commit().await.map_err(|e| statement_error(&e, sql))?;
            Ok(Some(completion))
        }
        Err(e) => {
            let _ = tx.rollback().await;
            if is_transaction_block_refusal(&e) {
                Ok(None)
            } else {
                Err(statement_error(&e, sql))
            }
        }
    }
}

/// Run the statement on a pooled connection in autocommit mode, for the
/// statements PostgreSQL refuses inside a transaction block (`VACUUM`,
/// `CREATE DATABASE`, `CREATE INDEX CONCURRENTLY`, ...).
async fn run_autocommit(
    pool: &PgPool,
    sql: &str,
    schema: Option<&str>,
    writer: &mut RowWriter<'_>,
) -> DbResult<Completion> {
    let mut conn = pool.acquire().await.map_err(|e| plain_error(&e))?;

    if let Some(schema) = schema {
        // Without a transaction there is no SET LOCAL; the session-level
        // setting is undone below.
        let set_path = format!("SET search_path TO {}", quote_schema(schema));
        (&mut *conn)
            .execute(set_path.as_str())
            .await
            .map_err(|e| plain_error(&e))?;
    }

    let outcome = run_statement(&mut conn, sql, writer).await;

    if schema.is_some() && (&mut *conn).execute("RESET search_path").await.is_err() {
        // A connection still carrying this tab's search_path must not be
        // handed to the next caller.
        let _ = conn.close().await;
    }

    outcome.map_err(|e| statement_error(&e, sql))
}

/// Prepare and execute `sql` on `conn`, feeding its rows to `writer`.
async fn run_statement(
    conn: &mut PgConnection,
    sql: &str,
    writer: &mut RowWriter<'_>,
) -> Result<Completion, sqlx::Error> {
    let statement = (&mut *conn).prepare(sql).await?;
    let columns: Vec<String> = statement
        .columns()
        .iter()
        .map(|c| c.name().to_string())
        .collect();
    let returns_rows = !columns.is_empty();
    writer.set_columns(columns);

    let mut stream = (&mut *conn).fetch_many(statement.query());
    let mut affected = 0u64;
    while let Some(item) = stream.try_next().await? {
        match item {
            Either::Left(done) => affected += done.rows_affected(),
            Either::Right(row) => {
                let cells = (0..row.len())
                    .map(|i| pg_value_to_string(&row, i))
                    .collect();
                if !writer.push(cells).await {
                    return Ok(Completion::ReceiverGone);
                }
            }
        }
    }

    Ok(if returns_rows {
        Completion::Rows
    } else {
        Completion::Affected(affected)
    })
}

/// `search_path` takes an identifier, not a bind parameter, so the schema is
/// double-quoted with embedded quotes doubled.
fn quote_schema(schema: &str) -> String {
    format!("\"{}\"", schema.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::quote_schema;

    #[test]
    fn schema_is_always_quoted() {
        assert_eq!(quote_schema("public"), "\"public\"");
        assert_eq!(quote_schema("Sales"), "\"Sales\"");
    }

    #[test]
    fn embedded_quotes_are_doubled() {
        assert_eq!(quote_schema("we\"ird"), "\"we\"\"ird\"");
        assert_eq!(
            quote_schema("x\"; DROP TABLE t; --"),
            "\"x\"\"; DROP TABLE t; --\""
        );
    }
}
