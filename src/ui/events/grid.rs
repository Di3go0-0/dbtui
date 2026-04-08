use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keybindings::Context;
use crate::ui::state::{AppState, Mode, Overlay};
use crate::ui::tabs::{CellEdit, RowChange, TabKind, WorkspaceTab};

use super::Action;

pub(super) fn handle_tab_data_grid(state: &mut AppState, key: KeyEvent) -> Action {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return Action::None;
    }

    // For scripts, sync grid state from the active result tab
    let is_script = matches!(state.tabs[tab_idx].kind, TabKind::Script { .. });
    if is_script {
        let tab = &mut state.tabs[tab_idx];
        let idx = tab.active_result_idx;
        if idx < tab.result_tabs.len() {
            let rt = &tab.result_tabs[idx];
            tab.query_result = Some(rt.result.clone());
            tab.grid_scroll_row = rt.scroll_row;
            tab.grid_selected_row = rt.selected_row;
            tab.grid_selected_col = rt.selected_col;
            tab.grid_selection_anchor = rt.selection_anchor;
        }

        // Result tab cycling is handled in events::mod.rs via [ and ] so it
        // shares the same keybinding as sub-view switching and stays
        // consistent with the global tab navigation standard.
    }

    // Resolve which grid action (if any) the key matches BEFORE taking the
    // `tab` mut borrow, so the bindings.matches() lookups don't collide with
    // the mutable state we need to modify below.
    let b = &state.bindings;
    let action_name: Option<&'static str> = if b.matches(Context::Grid, "scroll_down", &key) {
        Some("scroll_down")
    } else if b.matches(Context::Grid, "scroll_up", &key) {
        Some("scroll_up")
    } else if b.matches(Context::Grid, "scroll_left", &key) {
        Some("scroll_left")
    } else if b.matches(Context::Grid, "scroll_right", &key) {
        Some("scroll_right")
    } else if b.matches(Context::Grid, "next_cell", &key) {
        Some("next_cell")
    } else if b.matches(Context::Grid, "prev_cell", &key) {
        Some("prev_cell")
    } else if b.matches(Context::Grid, "scroll_top", &key) {
        Some("scroll_top")
    } else if b.matches(Context::Grid, "scroll_bottom", &key) {
        Some("scroll_bottom")
    } else if b.matches(Context::Grid, "half_page_down", &key) {
        Some("half_page_down")
    } else if b.matches(Context::Grid, "half_page_up", &key) {
        Some("half_page_up")
    } else if b.matches(Context::Grid, "toggle_visual", &key) {
        Some("toggle_visual")
    } else if b.matches(Context::Grid, "yank", &key) {
        Some("yank")
    } else if b.matches(Context::Grid, "refresh_data", &key) {
        Some("refresh_data")
    } else if b.matches(Context::Grid, "toggle_auto_refresh", &key) {
        Some("toggle_auto_refresh")
    } else if b.matches(Context::Grid, "edit_cell", &key) {
        Some("edit_cell")
    } else if b.matches(Context::Grid, "new_row", &key) {
        Some("new_row")
    } else if b.matches(Context::Grid, "delete_pending", &key) {
        Some("delete_pending")
    } else if b.matches(Context::Grid, "undo_changes", &key) {
        Some("undo_changes")
    } else if b.matches(Context::Grid, "save_changes", &key) {
        Some("save_changes")
    } else if b.matches(Context::Grid, "exit_grid", &key) {
        Some("exit_grid")
    } else {
        None
    };

    let tab = &mut state.tabs[tab_idx];

    // If in insert mode editing a cell, handle inline editing keys
    if tab.grid_editing.is_some() && state.mode == Mode::Insert {
        return handle_grid_cell_edit(state, key);
    }

    // Clear error panes on any grid interaction
    if tab.grid_error_editor.is_some() && action_name == Some("exit_grid") {
        tab.grid_error_editor = None;
        tab.grid_query_editor = None;
        return Action::Render;
    }

    let is_table_tab = matches!(tab.kind, TabKind::Table { .. });
    let row_count = tab.query_result.as_ref().map(|r| r.rows.len()).unwrap_or(0);
    let col_count = tab
        .query_result
        .as_ref()
        .map(|r| r.columns.len())
        .unwrap_or(0);
    let vh = tab.grid_visible_height.max(1);
    let visual = tab.grid_visual_mode;

    let action = match action_name {
        // --- Enter cell edit mode (only for table tabs) ---
        Some("edit_cell") if is_table_tab && !visual => {
            if row_count > 0 && col_count > 0 {
                let row = tab.grid_selected_row;
                let col = tab.grid_selected_col;
                let val = tab
                    .query_result
                    .as_ref()
                    .and_then(|r| r.rows.get(row))
                    .and_then(|r| r.get(col))
                    .cloned()
                    .unwrap_or_default();
                // Clear NULL values so user starts with empty field
                let val = if val == "NULL" { String::new() } else { val };
                let cursor = val.len();
                tab.grid_editing = Some((row, col));
                tab.grid_edit_buffer = val;
                tab.grid_edit_cursor = cursor;
                state.mode = Mode::Insert;
            }
            return Action::Render;
        }
        // --- New row ---
        Some("new_row") if is_table_tab && !visual => {
            if let Some(ref mut qr) = tab.query_result {
                let new_row: Vec<String> = qr.columns.iter().map(|_| "NULL".to_string()).collect();
                let insert_pos = (tab.grid_selected_row + 1).min(qr.rows.len());
                qr.rows.insert(insert_pos, new_row.clone());
                // Shift existing change keys >= insert_pos
                let mut shifted: std::collections::HashMap<usize, _> =
                    std::collections::HashMap::new();
                for (k, v) in tab.grid_changes.drain() {
                    if k >= insert_pos {
                        shifted.insert(k + 1, v);
                    } else {
                        shifted.insert(k, v);
                    }
                }
                tab.grid_changes = shifted;
                tab.grid_changes
                    .insert(insert_pos, RowChange::New { values: new_row });
                tab.grid_selected_row = insert_pos;
                tab.grid_selected_col = 0;
                if tab.grid_selected_row >= tab.grid_scroll_row + vh {
                    tab.grid_scroll_row = tab.grid_selected_row.saturating_sub(vh - 1);
                }
            }
            return Action::Render;
        }
        // --- Delete row ---
        Some("delete_pending") if is_table_tab && !visual => {
            if state.pending_d {
                // dd: mark row as deleted
                state.pending_d = false;
                if row_count > 0 {
                    let row = tab.grid_selected_row;
                    // If it's a new row, just remove it entirely
                    if matches!(tab.grid_changes.get(&row), Some(RowChange::New { .. })) {
                        tab.grid_changes.remove(&row);
                        if let Some(ref mut qr) = tab.query_result
                            && row < qr.rows.len()
                        {
                            qr.rows.remove(row);
                        }
                        // Shift keys > row
                        let mut shifted: std::collections::HashMap<usize, _> =
                            std::collections::HashMap::new();
                        for (k, v) in tab.grid_changes.drain() {
                            if k > row {
                                shifted.insert(k - 1, v);
                            } else {
                                shifted.insert(k, v);
                            }
                        }
                        tab.grid_changes = shifted;
                        let new_count =
                            tab.query_result.as_ref().map(|r| r.rows.len()).unwrap_or(0);
                        if tab.grid_selected_row >= new_count && new_count > 0 {
                            tab.grid_selected_row = new_count - 1;
                        }
                    } else {
                        tab.grid_changes.insert(row, RowChange::Deleted);
                    }
                }
                return Action::Render;
            } else {
                state.pending_d = true;
                return Action::Render;
            }
        }
        // --- Undo all changes ---
        Some("undo_changes") if is_table_tab && !visual => {
            if !tab.grid_changes.is_empty() {
                tab.grid_changes.clear();
                state.status_message = "Changes discarded".to_string();
                return Action::ReloadTableData;
            }
            return Action::Render;
        }
        // --- Refresh table data (re-fetch from DB) ---
        Some("refresh_data") if !visual => {
            if !tab.grid_changes.is_empty() {
                state.status_message =
                    "Pending changes — save with Ctrl+s or discard with u first".to_string();
                return Action::Render;
            }
            // Script tab: re-execute the source query of the active result
            // tab. This is the manual side of feature F (auto-refresh) — the
            // user is on the result pane and wants to see fresh data without
            // jumping back to the editor.
            if is_script {
                let idx = tab.active_result_idx;
                if let Some(rt) = tab.result_tabs.get(idx)
                    && !rt.source_query.is_empty()
                {
                    let query = rt.source_query.clone();
                    let start_line = rt.source_start_line;
                    let tab_id = tab.id;
                    state.status_message = "Refreshing query...".to_string();
                    return Action::ExecuteQuery {
                        tab_id,
                        query,
                        start_line,
                    };
                }
                state.status_message = "No source query to refresh".to_string();
                return Action::Render;
            }
            state.status_message = "Refreshing...".to_string();
            return Action::ReloadTableData;
        }
        // --- Toggle auto-refresh on the active script result tab ---
        // Cycles through preset intervals: off → 2s → 5s → 10s → 30s → off.
        // Only meaningful for script tabs that have a `source_query`.
        Some("toggle_auto_refresh") if !visual && is_script => {
            use crate::ui::tabs::AutoRefresh;
            let idx = tab.active_result_idx;
            if let Some(rt) = tab.result_tabs.get_mut(idx) {
                if rt.source_query.is_empty() {
                    state.status_message = "No source query to auto-refresh".to_string();
                    return Action::Render;
                }
                let next = match rt.auto_refresh.as_ref().map(|a| a.interval.as_secs()) {
                    None => Some(2),
                    Some(2) => Some(5),
                    Some(5) => Some(10),
                    Some(10) => Some(30),
                    Some(30) | Some(_) => None,
                };
                rt.auto_refresh = next.map(|secs| AutoRefresh {
                    interval: std::time::Duration::from_secs(secs),
                    next_at: std::time::Instant::now() + std::time::Duration::from_secs(secs),
                    in_flight: false,
                });
                state.status_message = match next {
                    Some(s) => format!("Auto-refresh: every {s}s"),
                    None => "Auto-refresh: off".to_string(),
                };
            }
            return Action::Render;
        }
        // --- Save changes ---
        Some("save_changes") if is_table_tab => {
            if !tab.grid_changes.is_empty() {
                let modified = tab
                    .grid_changes
                    .values()
                    .filter(|c| matches!(c, RowChange::Modified { .. }))
                    .count();
                let new = tab
                    .grid_changes
                    .values()
                    .filter(|c| matches!(c, RowChange::New { .. }))
                    .count();
                let deleted = tab
                    .grid_changes
                    .values()
                    .filter(|c| matches!(c, RowChange::Deleted))
                    .count();
                state.status_message =
                    format!("Save: {modified} modified, {new} new, {deleted} deleted — y/n?");
                state.overlay = Some(Overlay::SaveGridChanges);
            } else {
                state.status_message = "No pending changes".to_string();
            }
            return Action::Render;
        }
        // --- Toggle visual mode ---
        Some("toggle_visual") => {
            if visual {
                tab.grid_visual_mode = false;
                tab.grid_selection_anchor = None;
                tab.grid_anchor_on_header = false;
            } else {
                tab.grid_visual_mode = true;
                tab.grid_selection_anchor = Some((tab.grid_selected_row, tab.grid_selected_col));
                // Remember whether visual mode started on the header row so
                // the subsequent yank can include the column names.
                tab.grid_anchor_on_header = tab.grid_on_header;
            }
            Action::Render
        }
        // --- Movement ---
        Some("scroll_down") => {
            if tab.grid_on_header {
                tab.grid_on_header = false;
            } else if tab.grid_selected_row + 1 < row_count {
                tab.grid_selected_row += 1;
                if tab.grid_selected_row >= tab.grid_scroll_row + vh {
                    tab.grid_scroll_row = tab.grid_selected_row - vh + 1;
                }
            }
            Action::Render
        }
        Some("scroll_up") => {
            if tab.grid_on_header {
                // Already on header
            } else if tab.grid_selected_row > 0 {
                tab.grid_selected_row -= 1;
                if tab.grid_selected_row < tab.grid_scroll_row {
                    tab.grid_scroll_row = tab.grid_selected_row;
                }
            } else {
                tab.grid_on_header = true;
            }
            Action::Render
        }
        Some("scroll_left") => {
            if tab.grid_selected_col > 0 {
                tab.grid_selected_col -= 1;
            }
            Action::Render
        }
        Some("scroll_right") => {
            if col_count > 0 && tab.grid_selected_col + 1 < col_count {
                tab.grid_selected_col += 1;
            }
            Action::Render
        }
        Some("next_cell") => {
            if col_count > 0 {
                if tab.grid_selected_col + 1 < col_count {
                    tab.grid_selected_col += 1;
                } else if tab.grid_selected_row + 1 < row_count {
                    tab.grid_selected_col = 0;
                    tab.grid_selected_row += 1;
                    if tab.grid_selected_row >= tab.grid_scroll_row + vh {
                        tab.grid_scroll_row = tab.grid_selected_row - vh + 1;
                    }
                }
            }
            Action::Render
        }
        Some("prev_cell") => {
            if tab.grid_selected_col > 0 {
                tab.grid_selected_col -= 1;
            } else if tab.grid_selected_row > 0 {
                tab.grid_selected_row -= 1;
                tab.grid_selected_col = col_count.saturating_sub(1);
                if tab.grid_selected_row < tab.grid_scroll_row {
                    tab.grid_scroll_row = tab.grid_selected_row;
                }
            }
            Action::Render
        }
        Some("half_page_down") => {
            let half = vh / 2;
            tab.grid_selected_row = (tab.grid_selected_row + half).min(row_count.saturating_sub(1));
            tab.grid_scroll_row = tab.grid_selected_row.saturating_sub(vh / 2);
            Action::Render
        }
        Some("half_page_up") => {
            let half = vh / 2;
            tab.grid_selected_row = tab.grid_selected_row.saturating_sub(half);
            tab.grid_scroll_row = tab.grid_selected_row.saturating_sub(vh / 2);
            Action::Render
        }
        Some("scroll_top") => {
            tab.grid_selected_row = 0;
            tab.grid_selected_col = 0;
            tab.grid_scroll_row = 0;
            tab.grid_on_header = true;
            Action::Render
        }
        Some("scroll_bottom") => {
            tab.grid_on_header = false;
            if row_count > 0 {
                tab.grid_selected_row = row_count - 1;
                tab.grid_scroll_row = row_count.saturating_sub(vh);
            }
            Action::Render
        }
        Some("yank") => {
            grid_yank(tab);
            tab.grid_visual_mode = false;
            tab.grid_selection_anchor = None;
            tab.grid_anchor_on_header = false;
            Action::Render
        }
        Some("exit_grid") => {
            if visual {
                tab.grid_visual_mode = false;
                tab.grid_selection_anchor = None;
            } else {
                tab.grid_focused = false;
                tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
            }
            Action::Render
        }
        _ => Action::None,
    };

    if action_name == Some("yank") {
        state.status_message = "Copied to clipboard".to_string();
    }

    // Sync grid state back to the active result tab for scripts
    if is_script {
        let tab = &mut state.tabs[tab_idx];
        sync_grid_to_result_tab(tab);
    }

    action
}

/// Handle key input when editing a cell inline (Insert mode in grid)
pub(super) fn handle_grid_cell_edit(state: &mut AppState, key: KeyEvent) -> Action {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return Action::None;
    }
    let tab = &mut state.tabs[tab_idx];

    match key.code {
        KeyCode::Esc | KeyCode::Enter => {
            // Commit cell edit
            if let Some((row, col)) = tab.grid_editing.take() {
                let new_val = if tab.grid_edit_buffer.is_empty() {
                    "NULL".to_string()
                } else {
                    tab.grid_edit_buffer.clone()
                };
                let original = tab
                    .query_result
                    .as_ref()
                    .and_then(|r| r.rows.get(row))
                    .and_then(|r| r.get(col))
                    .cloned()
                    .unwrap_or_default();

                // Update the value in query_result
                if let Some(ref mut qr) = tab.query_result
                    && let Some(r) = qr.rows.get_mut(row)
                    && let Some(cell) = r.get_mut(col)
                {
                    *cell = new_val.clone();
                }

                // Track change (only if different and not already a New row)
                if new_val != original {
                    match tab.grid_changes.get_mut(&row) {
                        Some(RowChange::Modified { edits }) => {
                            // Update existing edit or add new one
                            if let Some(e) = edits.iter_mut().find(|e| e.col == col) {
                                e.value = new_val;
                            } else {
                                edits.push(CellEdit {
                                    col,
                                    original,
                                    value: new_val,
                                });
                            }
                        }
                        Some(RowChange::New { values }) => {
                            if let Some(v) = values.get_mut(col) {
                                *v = new_val;
                            }
                        }
                        Some(RowChange::Deleted) => {}
                        None => {
                            tab.grid_changes.insert(
                                row,
                                RowChange::Modified {
                                    edits: vec![CellEdit {
                                        col,
                                        original,
                                        value: new_val,
                                    }],
                                },
                            );
                        }
                    }
                }
            }
            tab.grid_edit_buffer.clear();
            tab.grid_edit_cursor = 0;
            state.mode = Mode::Normal;
            Action::Render
        }
        KeyCode::Tab => {
            // Commit current cell, move to next and enter edit mode
            // First trigger Esc logic to commit
            let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
            let _ = handle_grid_cell_edit(state, esc_event);

            let tab = &mut state.tabs[tab_idx];
            let col_count = tab
                .query_result
                .as_ref()
                .map(|r| r.columns.len())
                .unwrap_or(0);
            let row_count = tab.query_result.as_ref().map(|r| r.rows.len()).unwrap_or(0);

            // Move to next cell
            if col_count > 0 && tab.grid_selected_col + 1 < col_count {
                tab.grid_selected_col += 1;
            } else if tab.grid_selected_row + 1 < row_count {
                tab.grid_selected_col = 0;
                tab.grid_selected_row += 1;
            }

            // Enter edit mode on new cell
            let row = tab.grid_selected_row;
            let col = tab.grid_selected_col;
            let val = tab
                .query_result
                .as_ref()
                .and_then(|r| r.rows.get(row))
                .and_then(|r| r.get(col))
                .cloned()
                .unwrap_or_default();
            let val = if val == "NULL" { String::new() } else { val };
            let cursor = val.len();
            tab.grid_editing = Some((row, col));
            tab.grid_edit_buffer = val;
            tab.grid_edit_cursor = cursor;
            state.mode = Mode::Insert;
            Action::Render
        }
        KeyCode::Backspace => {
            let tab = &mut state.tabs[tab_idx];
            if tab.grid_edit_cursor > 0 {
                tab.grid_edit_cursor -= 1;
                tab.grid_edit_buffer.remove(tab.grid_edit_cursor);
            }
            Action::Render
        }
        KeyCode::Delete => {
            let tab = &mut state.tabs[tab_idx];
            if tab.grid_edit_cursor < tab.grid_edit_buffer.len() {
                tab.grid_edit_buffer.remove(tab.grid_edit_cursor);
            }
            Action::Render
        }
        KeyCode::Left => {
            let tab = &mut state.tabs[tab_idx];
            if tab.grid_edit_cursor > 0 {
                tab.grid_edit_cursor -= 1;
            }
            Action::Render
        }
        KeyCode::Right => {
            let tab = &mut state.tabs[tab_idx];
            if tab.grid_edit_cursor < tab.grid_edit_buffer.len() {
                tab.grid_edit_cursor += 1;
            }
            Action::Render
        }
        KeyCode::Home => {
            let tab = &mut state.tabs[tab_idx];
            tab.grid_edit_cursor = 0;
            Action::Render
        }
        KeyCode::End => {
            let tab = &mut state.tabs[tab_idx];
            tab.grid_edit_cursor = tab.grid_edit_buffer.len();
            Action::Render
        }
        KeyCode::Char(c) => {
            let tab = &mut state.tabs[tab_idx];
            tab.grid_edit_buffer.insert(tab.grid_edit_cursor, c);
            tab.grid_edit_cursor += 1;
            Action::Render
        }
        _ => Action::None,
    }
}

/// Handle keys in the table error/SQL read-only editor panes
pub(super) fn handle_table_error_editor(
    state: &mut AppState,
    key: KeyEvent,
    is_query: bool,
) -> Action {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return Action::None;
    }

    // Escape: exit visual/search mode first, only return to editor from Normal mode
    if key.code == KeyCode::Esc {
        let tab = &mut state.tabs[tab_idx];
        let editor = if is_query {
            tab.grid_query_editor.as_ref()
        } else {
            tab.grid_error_editor.as_ref()
        };
        let in_normal =
            editor.is_some_and(|e| matches!(e.mode, vimltui::VimMode::Normal) && !e.search.active);
        if in_normal {
            tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
            return Action::Render;
        }
        // Otherwise let vimltui handle Esc (exit visual/search)
    }

    // Ctrl+h/l or Ctrl+Left/Right: switch between error and SQL panes
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('l') | KeyCode::Right => {
                if !is_query {
                    let tab = &mut state.tabs[tab_idx];
                    if tab.grid_query_editor.is_some() {
                        tab.sub_focus = crate::ui::tabs::SubFocus::QueryView;
                    }
                }
                return Action::Render;
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if is_query {
                    let tab = &mut state.tabs[tab_idx];
                    tab.sub_focus = crate::ui::tabs::SubFocus::Results;
                }
                return Action::Render;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let tab = &mut state.tabs[tab_idx];
                tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
                return Action::Render;
            }
            _ => {}
        }
    }

    // Forward vim keys to the appropriate editor
    let tab = &mut state.tabs[tab_idx];
    let editor = if is_query {
        tab.grid_query_editor.as_mut()
    } else {
        tab.grid_error_editor.as_mut()
    };
    if let Some(ed) = editor {
        let _ = ed.handle_key(key);
    }
    Action::Render
}

/// Handle the save grid changes confirmation overlay (y/n)
pub(super) fn handle_save_grid_confirm(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            state.overlay = None;
            Action::SaveGridChanges
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            state.overlay = None;
            state.status_message = "Save cancelled".to_string();
            Action::Render
        }
        _ => Action::None,
    }
}

pub(super) fn sync_grid_to_result_tab(tab: &mut WorkspaceTab) {
    let idx = tab.active_result_idx;
    if idx < tab.result_tabs.len() {
        tab.result_tabs[idx].scroll_row = tab.grid_scroll_row;
        tab.result_tabs[idx].selected_row = tab.grid_selected_row;
        tab.result_tabs[idx].selected_col = tab.grid_selected_col;
        tab.result_tabs[idx].selection_anchor = tab.grid_selection_anchor;
    }
}

/// Copy grid data to system clipboard.
/// - No selection: copies entire current row (values joined by space).
/// - With selection: copies the selected rectangle of cells.
///   Same-row values joined by space, different rows by newline.
pub(super) fn grid_yank(tab: &WorkspaceTab) {
    let text = build_yank_text(tab);
    if !text.is_empty() {
        copy_to_clipboard(&text);
    }
}

/// Build the text that would be yanked from the current grid state. Pure
/// function so it can be unit-tested without touching the clipboard.
pub(super) fn build_yank_text(tab: &WorkspaceTab) -> String {
    let result = match &tab.query_result {
        Some(r) => r,
        None => return String::new(),
    };

    // `include_header` is true when the cursor is currently on the header
    // row OR the visual-mode anchor was on the header row — in either case
    // the column names should be the first line of the yanked text (for
    // the selected column range).
    let include_header = tab.grid_on_header || tab.grid_anchor_on_header;

    // Cursor on header with no visual selection → just copy the column
    // names (scoped to all columns — nothing else is selected in this mode).
    if tab.grid_on_header && tab.grid_selection_anchor.is_none() {
        let vals: Vec<&str> = result.columns.iter().map(|c| c.as_str()).collect();
        return vals.join(" ");
    }

    let last_col = result.columns.len().saturating_sub(1);

    let (sr, sc, er, ec) = match tab.grid_selection_anchor {
        Some((ar, ac)) => {
            let r1 = ar.min(tab.grid_selected_row);
            let r2 = ar.max(tab.grid_selected_row);
            let c1 = ac.min(tab.grid_selected_col);
            let c2 = ac.max(tab.grid_selected_col);
            (r1, c1, r2, c2)
        }
        None => (tab.grid_selected_row, 0, tab.grid_selected_row, last_col),
    };

    let mut text = String::new();
    if include_header {
        let vals: Vec<&str> = (sc..=ec)
            .filter_map(|c| result.columns.get(c).map(|v| v.as_str()))
            .collect();
        text.push_str(&vals.join(" "));
    }

    if !result.rows.is_empty() {
        for row_idx in sr..=er {
            if let Some(row_data) = result.rows.get(row_idx) {
                if !text.is_empty() {
                    text.push('\n');
                }
                let vals: Vec<&str> = (sc..=ec)
                    .filter_map(|c| row_data.get(c).map(|v| v.as_str()))
                    .collect();
                text.push_str(&vals.join(" "));
            }
        }
    }

    text
}

/// Copy text to system clipboard. Tries (in order):
/// 1. OSC 52 escape sequence (works in most modern terminals, even over SSH)
/// 2. Native clipboard tools (wl-copy, xclip, xsel)
pub(super) fn copy_to_clipboard(text: &str) {
    use std::io::Write;

    // OSC 52: terminal-native clipboard access — works in kitty, alacritty,
    // foot, iTerm2, WezTerm, Windows Terminal, tmux (with set-clipboard on), etc.
    let b64 = simple_base64_encode(text.as_bytes());
    let osc = format!("\x1b]52;c;{b64}\x07");
    let _ = std::io::stdout().write_all(osc.as_bytes());
    let _ = std::io::stdout().flush();

    // Also try native clipboard tools for broader compatibility
    let cmds: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    for (cmd, args) in cmds {
        if let Ok(mut child) = std::process::Command::new(cmd)
            .args(*args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
            return;
        }
    }
}

/// Minimal Base64 encoder (RFC 4648). Avoids adding a crate dependency.
fn simple_base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((n >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(n & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::QueryResult;
    use crate::ui::tabs::{TabId, WorkspaceTab};

    fn test_tab() -> WorkspaceTab {
        let mut tab =
            WorkspaceTab::new_script(TabId(1), "test".to_string(), Some("conn".to_string()), None);
        tab.query_result = Some(QueryResult {
            columns: vec!["id".to_string(), "name".to_string(), "age".to_string()],
            rows: vec![
                vec!["1".to_string(), "Alice".to_string(), "30".to_string()],
                vec!["2".to_string(), "Bob".to_string(), "25".to_string()],
                vec!["3".to_string(), "Carol".to_string(), "40".to_string()],
            ],
            elapsed: None,
        });
        tab
    }

    #[test]
    fn yank_header_only_when_cursor_on_header_no_selection() {
        let mut tab = test_tab();
        tab.grid_on_header = true;
        assert_eq!(build_yank_text(&tab), "id name age");
    }

    #[test]
    fn yank_visual_from_header_top_right_going_down() {
        // Repro for user bug: start on header, rightmost column, press v,
        // then press j twice to move down. Yank should include header AND
        // rows 0..=current, scoped to the selected column range.
        let mut tab = test_tab();
        // User navigates to header (row 0, col last) via k then l's.
        tab.grid_on_header = true;
        tab.grid_selected_row = 0;
        tab.grid_selected_col = 2; // last col
        // Press `v` — toggle_visual captures anchor and grid_anchor_on_header.
        tab.grid_visual_mode = true;
        tab.grid_selection_anchor = Some((tab.grid_selected_row, tab.grid_selected_col));
        tab.grid_anchor_on_header = tab.grid_on_header; // = true
        // First `j` leaves the header flag without moving the row.
        tab.grid_on_header = false;
        // Second `j` moves to row 1.
        tab.grid_selected_row = 1;

        let text = build_yank_text(&tab);
        // Expected: header (just the last col since sc=ec=2), then row 0,
        // then row 1 — all scoped to col 2.
        assert_eq!(text, "age\n30\n25");
    }

    #[test]
    fn yank_visual_from_header_top_right_going_down_and_left() {
        // Same as above but the user also moves left to widen the column
        // range. Anchor col = 2 (last), current col = 0 → range 0..=2.
        let mut tab = test_tab();
        tab.grid_on_header = true;
        tab.grid_selected_row = 0;
        tab.grid_selected_col = 2;
        tab.grid_visual_mode = true;
        tab.grid_selection_anchor = Some((0, 2));
        tab.grid_anchor_on_header = true;
        tab.grid_on_header = false;
        tab.grid_selected_row = 2;
        tab.grid_selected_col = 0;

        let text = build_yank_text(&tab);
        assert_eq!(text, "id name age\n1 Alice 30\n2 Bob 25\n3 Carol 40");
    }

    #[test]
    fn yank_visual_from_bottom_going_up_into_header() {
        // The other direction — user starts on a data row, presses v, then
        // walks up past row 0 onto the header. Already worked before my
        // fix; keep the regression test to prove it still does.
        let mut tab = test_tab();
        tab.grid_on_header = false;
        tab.grid_selected_row = 2;
        tab.grid_selected_col = 0;
        tab.grid_visual_mode = true;
        tab.grid_selection_anchor = Some((2, 0));
        tab.grid_anchor_on_header = false;
        // k, k, k (past row 0) → grid_on_header = true.
        tab.grid_selected_row = 0;
        tab.grid_on_header = true;

        let text = build_yank_text(&tab);
        assert_eq!(text, "id\n1\n2\n3");
    }
}
