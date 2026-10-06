//! Where the rows of a user statement go.
//!
//! Every adapter runs a statement the same way whether the caller wants the
//! rows streamed (`execute_streaming*`) or collected (`execute`). `RowWriter`
//! hides that difference, so each driver has a single execution path and the
//! two trait entry points cannot drift apart.

use tokio::sync::mpsc;

use crate::core::adapter::QueryBatch;
use crate::core::error::DbResult;
use crate::core::models::QueryResult;

/// Rows per streamed batch.
pub(crate) const BATCH_SIZE: usize = 500;

/// Destination of a statement's rows.
pub(crate) enum RowSink<'a> {
    /// Forward rows in batches as they arrive.
    Stream(&'a mpsc::Sender<DbResult<QueryBatch>>),
    /// Accumulate every row in memory.
    Collect(&'a mut QueryResult),
}

/// Buffers rows and hands them to a `RowSink`.
pub(crate) struct RowWriter<'a> {
    sink: RowSink<'a>,
    columns: Vec<String>,
    pending: Vec<Vec<String>>,
}

impl<'a> RowWriter<'a> {
    pub(crate) fn new(sink: RowSink<'a>) -> Self {
        Self {
            sink,
            columns: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// Set the header row. Called before the first `push`.
    pub(crate) fn set_columns(&mut self, columns: Vec<String>) {
        self.columns = columns;
    }

    pub(crate) fn has_columns(&self) -> bool {
        !self.columns.is_empty()
    }

    /// Queue one row, flushing a full batch to a streaming sink.
    ///
    /// Returns `false` once the receiver has hung up — the tab was closed or
    /// the query cancelled — so the caller can stop reading from the server.
    pub(crate) async fn push(&mut self, row: Vec<String>) -> bool {
        self.pending.push(row);
        match self.take_full_batch() {
            Some((tx, batch)) => tx.send(Ok(batch)).await.is_ok(),
            None => true,
        }
    }

    /// `push` for drivers that run on a blocking thread (Oracle).
    pub(crate) fn push_blocking(&mut self, row: Vec<String>) -> bool {
        self.pending.push(row);
        match self.take_full_batch() {
            Some((tx, batch)) => tx.blocking_send(Ok(batch)).is_ok(),
            None => true,
        }
    }

    /// Deliver the remaining rows and mark the result complete.
    pub(crate) async fn finish(self) {
        if let Some((tx, batch)) = self.into_final_batch() {
            let _ = tx.send(Ok(batch)).await;
        }
    }

    /// `finish` for drivers that run on a blocking thread (Oracle).
    pub(crate) fn finish_blocking(self) {
        if let Some((tx, batch)) = self.into_final_batch() {
            let _ = tx.blocking_send(Ok(batch));
        }
    }

    /// Replace whatever was queued with the single-cell result shown for
    /// statements that return no rows.
    pub(crate) fn set_message(&mut self, message: String) {
        self.columns = vec!["Result".to_string()];
        self.pending = vec![vec![message]];
    }

    /// A full batch and the channel it goes to, when streaming.
    fn take_full_batch(&mut self) -> Option<(&'a mpsc::Sender<DbResult<QueryBatch>>, QueryBatch)> {
        let tx = match &self.sink {
            RowSink::Stream(tx) => *tx,
            RowSink::Collect(_) => return None,
        };
        if self.pending.len() < BATCH_SIZE {
            return None;
        }
        let rows = std::mem::replace(&mut self.pending, Vec::with_capacity(BATCH_SIZE));
        Some((
            tx,
            QueryBatch {
                columns: self.columns.clone(),
                rows,
                done: false,
            },
        ))
    }

    /// The closing batch for a streaming sink. A collecting sink is filled in
    /// place and yields nothing to send.
    fn into_final_batch(self) -> Option<(&'a mpsc::Sender<DbResult<QueryBatch>>, QueryBatch)> {
        match self.sink {
            RowSink::Stream(tx) => Some((
                tx,
                QueryBatch {
                    columns: self.columns,
                    rows: self.pending,
                    done: true,
                },
            )),
            RowSink::Collect(result) => {
                result.columns = self.columns;
                result.rows = self.pending;
                None
            }
        }
    }
}

/// Text of the single-cell result shown for a statement that returns no rows.
pub(crate) fn success_message(rows_affected: Option<u64>) -> String {
    match rows_affected {
        Some(n) => format!("Statement executed successfully ({n} row(s) affected)"),
        None => "Statement executed successfully".to_string(),
    }
}

/// An empty result for `execute()` to fill through `RowSink::Collect`.
pub(crate) fn empty_result() -> QueryResult {
    QueryResult {
        columns: Vec::new(),
        rows: Vec::new(),
        elapsed: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(n: usize) -> Vec<String> {
        vec![n.to_string()]
    }

    #[tokio::test]
    async fn collect_keeps_every_row_and_the_header() {
        let mut result = empty_result();
        let mut writer = RowWriter::new(RowSink::Collect(&mut result));
        writer.set_columns(vec!["n".to_string()]);
        for n in 0..(BATCH_SIZE + 3) {
            assert!(writer.push(row(n)).await);
        }
        writer.finish().await;
        assert_eq!(result.columns, vec!["n".to_string()]);
        assert_eq!(result.rows.len(), BATCH_SIZE + 3);
    }

    #[tokio::test]
    async fn stream_flushes_full_batches_then_a_final_one() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut writer = RowWriter::new(RowSink::Stream(&tx));
        writer.set_columns(vec!["n".to_string()]);
        for n in 0..(BATCH_SIZE + 3) {
            assert!(writer.push(row(n)).await);
        }
        writer.finish().await;

        let first = rx.recv().await.and_then(Result::ok);
        let last = rx.recv().await.and_then(Result::ok);
        assert_eq!(first.as_ref().map(|b| b.rows.len()), Some(BATCH_SIZE));
        assert_eq!(first.map(|b| b.done), Some(false));
        assert_eq!(last.as_ref().map(|b| b.rows.len()), Some(3));
        assert_eq!(last.as_ref().map(|b| b.done), Some(true));
        assert_eq!(last.map(|b| b.columns), Some(vec!["n".to_string()]));
    }

    #[tokio::test]
    async fn empty_result_still_sends_its_header() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut writer = RowWriter::new(RowSink::Stream(&tx));
        writer.set_columns(vec!["id".to_string(), "name".to_string()]);
        writer.finish().await;

        let only = rx.recv().await.and_then(Result::ok);
        assert_eq!(only.as_ref().map(|b| b.columns.len()), Some(2));
        assert_eq!(only.as_ref().map(|b| b.rows.len()), Some(0));
        assert_eq!(only.map(|b| b.done), Some(true));
    }

    #[tokio::test]
    async fn push_reports_a_receiver_that_hung_up() {
        let (tx, rx) = mpsc::channel(8);
        drop(rx);
        let mut writer = RowWriter::new(RowSink::Stream(&tx));
        let mut alive = true;
        for n in 0..BATCH_SIZE {
            alive = writer.push(row(n)).await;
        }
        assert!(!alive);
    }

    #[tokio::test]
    async fn message_replaces_queued_rows() {
        let mut result = empty_result();
        let mut writer = RowWriter::new(RowSink::Collect(&mut result));
        writer.set_message(success_message(Some(2)));
        writer.finish().await;
        assert_eq!(result.columns, vec!["Result".to_string()]);
        assert_eq!(
            result.rows,
            vec![vec![
                "Statement executed successfully (2 row(s) affected)".to_string()
            ]]
        );
    }

    #[test]
    fn message_without_a_count() {
        assert_eq!(success_message(None), "Statement executed successfully");
    }
}
