//! Execution of user statements on SQL Server.

use std::sync::Arc;

use futures_util::TryStreamExt;
use tiberius::QueryItem;

use crate::core::error::DbResult;
use crate::core::models::DatabaseType;
use crate::drivers::mssql::error::statement_error;
use crate::drivers::mssql::pool::{MssqlClient, MssqlPool};
use crate::drivers::mssql::value::mssql_row_to_strings;
use crate::drivers::sink::{RowSink, RowWriter, success_message};
use crate::drivers::statement::is_row_producing;

/// How a statement ended.
enum Completion {
    /// It returned a result set (possibly empty); the rows are in the writer.
    Rows,
    /// It returned no result set. The count is unknown for a raw batch.
    Affected(Option<u64>),
    /// The receiver hung up before the last row was read.
    ReceiverGone,
}

/// Run one user batch and deliver its outcome to `sink`.
pub(super) async fn run(pool: &Arc<MssqlPool>, sql: &str, sink: RowSink<'_>) -> DbResult<()> {
    let mut writer = RowWriter::new(sink);
    let mut pooled = pool.get().await?;

    let outcome = async {
        let client = pooled.client()?;
        if is_row_producing(DatabaseType::SqlServer, sql) {
            run_batch(client, sql, &mut writer).await
        } else {
            run_counted(client, sql).await
        }
    }
    .await;

    match outcome {
        Ok(Completion::Rows) => {}
        Ok(Completion::Affected(count)) => writer.set_message(success_message(count)),
        Ok(Completion::ReceiverGone) => {
            // The tab was closed or the query cancelled. Unread tokens make
            // this connection unsafe to reuse.
            pooled.discard();
            return Ok(());
        }
        Err(e) => {
            pooled.discard();
            return Err(e);
        }
    }
    writer.finish().await;
    Ok(())
}

/// Run a statement that returns no rows through `sp_executesql`, which
/// reports how many rows it touched.
async fn run_counted(client: &mut MssqlClient, sql: &str) -> DbResult<Completion> {
    let outcome = client
        .execute(sql, &[])
        .await
        .map_err(|e| statement_error(&e))?;
    Ok(Completion::Affected(Some(outcome.total())))
}

/// Run a raw batch and feed the rows of its first result set to `writer`.
///
/// A raw batch, not `sp_executesql`, so user SQL behaves the way it does in
/// SSMS — `DECLARE`, temp tables and multiple statements all share one batch
/// scope.
///
/// Later result sets are read to the end but not shown: their rows do not fit
/// the first set's header. Reading them lets the whole batch run, surfaces an
/// error raised by a later statement, and leaves the connection reusable.
async fn run_batch(
    client: &mut MssqlClient,
    sql: &str,
    writer: &mut RowWriter<'_>,
) -> DbResult<Completion> {
    let mut stream = client
        .simple_query(sql)
        .await
        .map_err(|e| statement_error(&e))?;

    let mut has_result_set = false;
    while let Some(item) = stream.try_next().await.map_err(|e| statement_error(&e))? {
        match item {
            // Taken from the metadata rather than the first row, so an empty
            // result set still renders its headers.
            QueryItem::Metadata(meta) if meta.result_index() == 0 => {
                has_result_set = true;
                writer.set_columns(
                    meta.columns()
                        .iter()
                        .map(|c| c.name().to_string())
                        .collect(),
                );
            }
            QueryItem::Row(row) if row.result_index() == 0 => {
                if !writer.push(mssql_row_to_strings(&row)).await {
                    return Ok(Completion::ReceiverGone);
                }
            }
            QueryItem::Metadata(_) | QueryItem::Row(_) => {}
        }
    }

    Ok(if has_result_set {
        Completion::Rows
    } else {
        // `EXEC` of a procedure that selects nothing, or a procedural batch
        // made only of DML.
        Completion::Affected(None)
    })
}
