//! Running a script's SQL: dispatching the statement, streaming its batches
//! back as `AppMessage`s, and reporting failures with the server's position.

use super::*;

/// Strip trailing line comments (`-- …`) from the query text so that
/// the semicolon removal logic can see the real last statement character.
/// Without this, a query like:
/// ```sql
/// SELECT 1 FROM DUAL;
/// -- comment
/// ```
/// would keep its `;` (because `trim_end()` sees the comment, not the
/// semicolon) and Oracle would reject it with ORA-00911.
fn strip_trailing_comments(sql: &str) -> String {
    let mut lines: Vec<&str> = sql.lines().collect();
    // Pop trailing lines that are only whitespace or `-- …` comments.
    while let Some(last) = lines.last() {
        let t = last.trim();
        if t.is_empty() || t.starts_with("--") {
            lines.pop();
        } else {
            break;
        }
    }
    lines.join("\n")
}

/// Return true if `sql` is a PL/SQL anonymous block (starts with
/// DECLARE, BEGIN, or a labelled `<<label>> ... BEGIN`). These blocks
/// must keep their trailing `END;` — stripping the semicolon would
/// leave an incomplete statement that Oracle rejects with PLS-00103.
fn is_plsql_block(sql: &str) -> bool {
    // Walk past any leading whitespace and SQL comments.
    let bytes = sql.as_bytes();
    let mut i = 0;
    loop {
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            // An unterminated comment runs to the end of the text.
            i = (i + 2).min(bytes.len());
            continue;
        }
        break;
    }
    let rest = sql.get(i..).unwrap_or("");
    let upper: String = rest
        .chars()
        .take(8)
        .flat_map(|c| c.to_uppercase())
        .collect();
    upper.starts_with("DECLARE") || upper.starts_with("BEGIN")
}

/// Text carried by a panic payload, for reporting a driver task that died.
fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_string())
}

impl App {
    /// Start executing `query` for a script tab and stream the result back.
    ///
    /// The run gets a fresh id that every message it produces carries, and
    /// any run still streaming on the tab is aborted first — without that, a
    /// re-execute interleaved the old stream's rows into the new result.
    pub(super) fn spawn_execute_query_at(
        &mut self,
        tab_id: TabId,
        query: &str,
        new_tab: bool,
        start_line: usize,
    ) {
        let Some(adapter) = self.require_tab_adapter(tab_id) else {
            self.stop_auto_refresh(tab_id);
            return;
        };

        self.next_run_id += 1;
        let run_id = self.next_run_id;
        if let Some(tab) = self.state.find_tab_mut(tab_id) {
            if let Some(previous) = tab.streaming_abort.take() {
                previous.abort();
            }
            tab.query_run_id = run_id;
            tab.run_result_idx = (!new_tab).then_some(tab.active_result_idx);
            tab.streaming = true;
            tab.streaming_since = Some(std::time::Instant::now());
            tab.first_batch_pending = true;
            tab.pending_query = Some((query.to_string(), start_line));
        }

        // A per-script schema overrides the connection's current schema for
        // this run only.
        let schema = self
            .state
            .find_tab(tab_id)
            .and_then(|tab| tab.kind.schema_override().map(|s| s.to_string()));

        let tx = self.msg_tx.clone();
        // Strip trailing semicolon for regular SQL statements — the drivers
        // reject it. PL/SQL anonymous blocks (DECLARE/BEGIN...END;) are the
        // opposite: they REQUIRE the trailing `;` on the final END, so those
        // are left alone. Only the tail is touched, so positions the server
        // reports still line up with the editor text.
        let query = {
            let trimmed = strip_trailing_comments(query.trim_end());
            if is_plsql_block(&trimmed) {
                trimmed
            } else {
                trimmed.trim_end_matches(';').trim_end().to_string()
            }
        };

        let handle = tokio::spawn(async move {
            let start = std::time::Instant::now();
            let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(4);

            let query_clone = query.clone();
            let stream_handle = tokio::spawn(async move {
                adapter
                    .execute_streaming_in_schema(&query_clone, schema.as_deref(), batch_tx)
                    .await
            });

            let failed = |error: crate::core::error::DbError| AppMessage::QueryFailed {
                tab_id,
                run_id,
                position: error.position(),
                error: error.to_string(),
                query: query.clone(),
                new_tab,
                start_line,
            };

            while let Some(batch_result) = batch_rx.recv().await {
                match batch_result {
                    Ok(batch) => {
                        let done = batch.done;
                        let message = AppMessage::QueryBatch {
                            tab_id,
                            run_id,
                            columns: batch.columns,
                            rows: batch.rows,
                            done,
                            new_tab,
                            elapsed: done.then(|| start.elapsed()),
                        };
                        if tx.send(message).await.is_err() {
                            // UI channel closed — abort the DB query
                            stream_handle.abort();
                            return;
                        }
                    }
                    Err(e) => {
                        // The driver task may still be blocked sending into
                        // the batch channel; returning drops the receiver,
                        // which is what lets it finish.
                        let _ = tx.send(failed(e)).await;
                        return;
                    }
                }
            }

            // The batch channel closed without an error batch: the driver
            // task has ended, one way or another.
            match stream_handle.await {
                Ok(Ok(())) => {
                    // A DDL statement went through — refresh the tree.
                    let upper = query.trim_start().to_uppercase();
                    if ["CREATE", "DROP", "ALTER", "RENAME"]
                        .iter()
                        .any(|kw| upper.starts_with(kw))
                    {
                        let _ = tx.send(AppMessage::DdlExecuted { query }).await;
                    }
                }
                Ok(Err(e)) => {
                    let _ = tx.send(failed(e)).await;
                }
                Err(join) if join.is_panic() => {
                    let detail = panic_message(join.into_panic());
                    let error = crate::core::error::DbError::QueryFailed(format!(
                        "the driver hit an internal error while reading the result: {detail}"
                    ));
                    let _ = tx.send(failed(error)).await;
                }
                // Aborted: cancelled by the user or superseded by a newer run.
                Err(_) => {}
            }
        });

        // Store the abort handle so the streaming task can be cancelled
        if let Some(tab) = self.state.find_tab_mut(tab_id) {
            tab.streaming_abort = Some(handle.abort_handle());
        }
    }

    /// Turn auto-refresh off for a tab whose query can no longer be started,
    /// so it does not retry on every interval.
    fn stop_auto_refresh(&mut self, tab_id: TabId) {
        if let Some(tab) = self.state.find_tab_mut(tab_id) {
            for result in &mut tab.result_tabs {
                result.auto_refresh = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_plsql_block, strip_trailing_comments};

    #[test]
    fn detects_plsql_blocks_behind_comments() {
        assert!(is_plsql_block("BEGIN NULL; END;"));
        assert!(is_plsql_block(
            "-- note\n/* x */ declare v number; begin null; end;"
        ));
        assert!(!is_plsql_block("SELECT 1 FROM dual"));
    }

    #[test]
    fn unterminated_block_comment_is_not_a_plsql_block() {
        // Used to slice past the end of the string and panic.
        assert!(!is_plsql_block("/* todo"));
        assert!(!is_plsql_block("/*"));
        assert!(!is_plsql_block("/* café"));
    }

    #[test]
    fn strip_trailing_line_comment() {
        let sql = "SELECT 1 FROM DUAL;\n-- trailing comment";
        assert_eq!(strip_trailing_comments(sql), "SELECT 1 FROM DUAL;");
    }

    #[test]
    fn strip_multiple_trailing_comments() {
        let sql = "SELECT 1 FROM DUAL;\n-- comment 1\n-- comment 2\n";
        assert_eq!(strip_trailing_comments(sql), "SELECT 1 FROM DUAL;");
    }

    #[test]
    fn no_trailing_comment_unchanged() {
        let sql = "SELECT 1 FROM DUAL";
        assert_eq!(strip_trailing_comments(sql), "SELECT 1 FROM DUAL");
    }

    #[test]
    fn leading_comment_preserved() {
        let sql = "-- leading\nSELECT 1 FROM DUAL";
        assert_eq!(
            strip_trailing_comments(sql),
            "-- leading\nSELECT 1 FROM DUAL"
        );
    }

    #[test]
    fn inline_comment_preserved() {
        let sql = "SELECT 1 -- inline\nFROM DUAL";
        assert_eq!(
            strip_trailing_comments(sql),
            "SELECT 1 -- inline\nFROM DUAL"
        );
    }

    #[test]
    fn trailing_blank_lines_stripped() {
        let sql = "SELECT 1 FROM DUAL;\n\n  \n";
        assert_eq!(strip_trailing_comments(sql), "SELECT 1 FROM DUAL;");
    }
}
