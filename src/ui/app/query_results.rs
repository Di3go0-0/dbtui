//! Applying a query run's outcome to its tab: streamed batches into result
//! tabs, and failures into an error tab plus a mark on the offending line.

use super::message_helpers::wrap_error_text;
use super::*;
use crate::core::error::ErrorPosition;
use crate::ui::diagnostics::{Diagnostic, error_span};
use crate::ui::tabs::{ResultTab, SubFocus};

/// A `QueryBatch` message, unpacked.
pub(super) struct QueryBatchArrival {
    pub tab_id: TabId,
    pub run_id: u64,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub done: bool,
    pub new_tab: bool,
    pub elapsed: Option<std::time::Duration>,
}

/// A `QueryFailed` message, unpacked.
pub(super) struct QueryFailure {
    pub tab_id: TabId,
    pub run_id: u64,
    pub error: String,
    pub position: Option<ErrorPosition>,
    pub query: String,
    pub new_tab: bool,
    pub start_line: usize,
}

impl App {
    /// Handle the QueryBatch message: append rows to a script result tab or
    /// table/view tab. Batches from a run the tab has moved on from are
    /// dropped.
    pub(super) fn handle_query_batch(&mut self, batch: QueryBatchArrival) {
        let tab_id = batch.tab_id;
        let done = batch.done;
        let elapsed = batch.elapsed;
        let batch_len = batch.rows.len();

        let Some(tab) = self.state.find_tab_mut(tab_id) else {
            return;
        };
        if tab.query_run_id != batch.run_id {
            return;
        }
        let first_batch = tab.first_batch_pending;
        let total_rows = if matches!(tab.kind, TabKind::Script { .. }) {
            apply_script_batch(tab, batch)
        } else {
            apply_table_batch(tab, batch)
        };

        // The statement ran, so whatever the server said about the previous
        // attempt no longer applies.
        if first_batch && self.clear_server_diagnostics(tab_id) {
            self.refresh_diagnostics_if_active(tab_id);
        }

        if done {
            self.finish_loading();
            self.state.status_message = match elapsed {
                Some(d) if d.as_millis() < 1000 => {
                    format!("{total_rows} rows returned ({} ms)", d.as_millis())
                }
                Some(d) => format!("{total_rows} rows returned ({:.2} s)", d.as_secs_f64()),
                None => format!("{total_rows} rows returned"),
            };
        } else {
            self.state.status_message = format!("Loading... {total_rows} rows (+{batch_len})");
        }
    }

    /// Handle the QueryFailed message: show the error in a result tab and
    /// mark the failing line in the editor.
    pub(super) fn handle_query_failed(&mut self, failure: QueryFailure) {
        let tab_id = failure.tab_id;
        // Line of the buffer the server pointed at; the statement's first
        // line when it gave no position.
        let failed_row =
            failure.start_line + failure.position.map_or(0, |p| p.line.saturating_sub(1));

        let Some(tab) = self.state.find_tab_mut(tab_id) else {
            return;
        };
        if tab.query_run_id != failure.run_id {
            return;
        }
        end_run(tab);
        let is_script = matches!(tab.kind, TabKind::Script { .. });
        if is_script {
            show_error_result(tab, &failure, failed_row);
            mark_failed_line(tab, &failure, failed_row);
        }

        self.finish_loading();
        if is_script {
            self.state.status_message = format!(
                "Query failed at line {} — K on the marked line shows why",
                failed_row + 1
            );
            self.refresh_diagnostics_if_active(tab_id);
        }
    }

    /// Drop a tab's server diagnostics. Returns whether there were any.
    pub(super) fn clear_server_diagnostics(&mut self, tab_id: TabId) -> bool {
        self.state
            .find_tab_mut(tab_id)
            .is_some_and(|tab| !std::mem::take(&mut tab.server_diagnostics).is_empty())
    }

    /// Recompute the diagnostics on screen when `tab_id` is the tab showing.
    pub(super) fn refresh_diagnostics_if_active(&mut self, tab_id: TabId) {
        if self.state.active_tab().map(|t| t.id) == Some(tab_id) {
            self.refresh_active_diagnostics();
        }
    }
}

/// Append a batch to the result tab its run writes into, creating or
/// replacing that tab on the run's first batch. Returns the row count now in
/// the result.
fn apply_script_batch(tab: &mut WorkspaceTab, batch: QueryBatchArrival) -> usize {
    let target = if std::mem::take(&mut tab.first_batch_pending) {
        // Consume the SQL stashed at dispatch time so the result tab knows
        // how to re-execute itself (manual refresh / auto-refresh).
        let (source, start_line) = tab.pending_query.take().unwrap_or_default();
        let in_place = tab
            .run_result_idx
            .filter(|idx| !batch.new_tab && *idx < tab.result_tabs.len());
        match in_place {
            Some(idx) => {
                // Replace in place so <leader>Enter overwrites the previous
                // result. The run counter and auto-refresh carry over only
                // when the same query is being re-executed; edited SQL is a
                // brand-new result.
                let mut result = ResultTab::new_data(
                    format!("Result {}", idx + 1),
                    batch.columns,
                    batch.rows,
                    source,
                    start_line,
                );
                let previous = &tab.result_tabs[idx];
                if previous.source_query.trim() == result.source_query.trim() {
                    result.run_count = previous.run_count + 1;
                    result.auto_refresh = previous.auto_refresh.clone();
                }
                tab.result_tabs[idx] = result;
                idx
            }
            None => {
                let label = format!("Result {}", tab.result_tabs.len() + 1);
                tab.result_tabs.push(ResultTab::new_data(
                    label,
                    batch.columns,
                    batch.rows,
                    source,
                    start_line,
                ));
                tab.active_result_idx = tab.result_tabs.len() - 1;
                tab.grid_focused = false;
                tab.sub_focus = SubFocus::Editor;
                tab.active_result_idx
            }
        }
    } else {
        // Continuing the stream: rows go to the run's own result tab, not to
        // whichever one happens to be selected now.
        let Some(idx) = tab
            .run_result_idx
            .filter(|idx| *idx < tab.result_tabs.len())
        else {
            return 0;
        };
        tab.result_tabs[idx].result.rows.extend(batch.rows);
        idx
    };
    tab.run_result_idx = Some(target);

    tab.streaming = !batch.done;
    if batch.done {
        end_run(tab);
    }
    let result = &mut tab.result_tabs[target];
    if let Some(elapsed) = batch.elapsed {
        result.result.elapsed = Some(elapsed);
    }
    result.result.rows.len()
}

/// Append a batch to a table/view tab's grid. Returns the row count.
fn apply_table_batch(tab: &mut WorkspaceTab, batch: QueryBatchArrival) -> usize {
    tab.first_batch_pending = false;
    match tab.query_result.as_mut() {
        Some(result) => result.rows.extend(batch.rows),
        None => {
            tab.query_result = Some(QueryResult {
                columns: batch.columns,
                rows: batch.rows,
                elapsed: batch.elapsed,
            });
            tab.grid_selected_row = 0;
            tab.grid_scroll_row = 0;
        }
    }
    tab.streaming = !batch.done;
    if batch.done {
        tab.streaming_abort = None;
        tab.streaming_since = None;
    }
    tab.query_result.as_ref().map_or(0, |r| r.rows.len())
}

/// Clear every "a query is in flight" marker and release the auto-refresh
/// slot of the result the run was feeding, measuring the next interval from
/// now so slow refreshes do not drift into each other.
fn end_run(tab: &mut WorkspaceTab) {
    tab.streaming = false;
    tab.streaming_since = None;
    tab.streaming_abort = None;
    tab.first_batch_pending = false;
    tab.pending_query = None;
    if let Some(refresh) = tab
        .run_result_idx
        .and_then(|idx| tab.result_tabs.get_mut(idx))
        .and_then(|result| result.auto_refresh.as_mut())
    {
        refresh.in_flight = false;
        refresh.next_at = std::time::Instant::now() + refresh.interval;
    }
}

/// Put the error and the SQL that failed into a result tab — the one the run
/// was replacing, or a new one.
fn show_error_result(tab: &mut WorkspaceTab, failure: &QueryFailure, failed_row: usize) {
    let text = format!(
        "-- Query Error (line {}) --\n\n{}",
        failed_row + 1,
        wrap_error_text(&failure.error, 40)
    );
    let in_place = tab
        .run_result_idx
        .filter(|idx| !failure.new_tab && *idx < tab.result_tabs.len());
    let idx = in_place.unwrap_or(tab.result_tabs.len());
    let result = ResultTab::new_error(
        format!("Error {}", idx + 1),
        &text,
        &failure.query,
        failure.start_line,
    );
    match in_place {
        Some(idx) => tab.result_tabs[idx] = result,
        None => {
            tab.result_tabs.push(result);
            tab.active_result_idx = idx;
        }
    }
    // Stay in the editor — the mark is there; the error tab is one key away.
    tab.grid_focused = false;
    tab.sub_focus = SubFocus::Editor;
}

/// Record the failure as a server diagnostic on the line it happened.
fn mark_failed_line(tab: &mut WorkspaceTab, failure: &QueryFailure, failed_row: usize) {
    let Some(editor) = tab.editor.as_ref() else {
        return;
    };
    let row = failed_row.min(editor.lines.len().saturating_sub(1));
    let Some(line) = editor.lines.get(row) else {
        return;
    };
    let span = error_span(line, failure.position.and_then(|p| p.col));
    let message = failure
        .error
        .lines()
        .next()
        .unwrap_or_default()
        .trim_start_matches("Query failed: ")
        // sqlx's wrapper around the server's own message.
        .trim_start_matches("error returned from database: ")
        .to_string();
    tab.server_diagnostics = vec![Diagnostic::server_error(row, span, message)];
    tab.server_diagnostics_view = None;
}
