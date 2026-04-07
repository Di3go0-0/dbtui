use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keybindings::Context;
use crate::ui::state::{AppState, ExportDialogState, Focus, ImportDialogState, Overlay};
use crate::ui::tabs::TabKind;

use super::Action;
use super::overlays::maybe_prompt_bind_vars;

// --- Leader key for non-editor views ---

/// Reset every leader-related pending flag so subsequent keys start fresh.
pub(super) fn clear_leader_state(state: &mut AppState) {
    state.leader.leader_pending = false;
    state.leader.b_pending = false;
    state.leader.w_pending = false;
    state.leader.s_pending = false;
    state.leader.f_pending = false;
    state.leader.q_pending = false;
    state.leader.pending = false;
    state.leader.pressed_at = None;
}

/// Global leader key handler — works from any panel.
/// Returns Some(Action) if the key was consumed, None otherwise.
pub(super) fn handle_global_leader(state: &mut AppState, key: KeyEvent) -> Option<Action> {
    // --- Sub-menu: <leader><leader> -> s ---
    if state.leader.leader_pending {
        // Compile to DB (only for source tabs)
        let action = state
            .active_tab()
            .filter(|tab| {
                matches!(
                    tab.kind,
                    TabKind::Package { .. } | TabKind::Function { .. } | TabKind::Procedure { .. }
                )
            })
            .map(|tab| Action::CompileToDb { tab_id: tab.id })
            .unwrap_or(Action::Render);
        let matched = matches!(key.code, KeyCode::Char('s'));
        clear_leader_state(state);
        return Some(if matched { action } else { Action::Render });
    }

    // --- Sub-menu: <leader>s -> SQL template snippets ---
    if state.leader.s_pending {
        let b = &state.bindings;
        let db = state.conn.db_type;
        let template = if b.matches(Context::LeaderSnippet, "snippet_select", &key) {
            Some("SELECT\n    *\nFROM $")
        } else if b.matches(Context::LeaderSnippet, "snippet_update", &key) {
            Some("UPDATE $\nSET \nWHERE ")
        } else if b.matches(Context::LeaderSnippet, "snippet_delete", &key) {
            Some("DELETE FROM $\nWHERE ")
        } else if b.matches(Context::LeaderSnippet, "snippet_call_proc", &key) {
            Some(match db {
                Some(crate::core::models::DatabaseType::Oracle) => "BEGIN\n    $;\nEND;",
                Some(crate::core::models::DatabaseType::MySQL) => "CALL $",
                _ => "CALL $",
            })
        } else if b.matches(Context::LeaderSnippet, "snippet_select_func", &key) {
            Some(match db {
                Some(crate::core::models::DatabaseType::Oracle) => {
                    "SELECT $() AS result\nFROM DUAL"
                }
                _ => "SELECT $() AS result\nFROM ",
            })
        } else if b.matches(Context::LeaderSnippet, "snippet_create_table", &key) {
            Some(match db {
                Some(crate::core::models::DatabaseType::Oracle) => {
                    "CREATE TABLE $ (\n    id NUMBER GENERATED ALWAYS AS IDENTITY,\n    \n    CONSTRAINT _pk PRIMARY KEY (id)\n)"
                }
                Some(crate::core::models::DatabaseType::MySQL) => {
                    "CREATE TABLE $ (\n    id INT AUTO_INCREMENT PRIMARY KEY,\n    \n)"
                }
                _ => "CREATE TABLE $ (\n    id SERIAL PRIMARY KEY,\n    \n)",
            })
        } else {
            None
        };
        clear_leader_state(state);

        if let Some(tpl) = template
            && let Some(tab) = state.active_tab_mut()
            && matches!(tab.kind, TabKind::Script { .. })
            && let Some(editor) = tab.active_editor_mut()
        {
            insert_template(editor, tpl);
        }
        return Some(Action::Render);
    }

    // --- Sub-menu: <leader>b -> d (close tab) ---
    if state.leader.b_pending {
        let matched = state
            .bindings
            .matches(Context::LeaderBuffer, "close_tab", &key);
        clear_leader_state(state);
        return Some(if matched {
            Action::CloseTab
        } else {
            Action::Render
        });
    }

    // --- Sub-menu: <leader>w -> d (close group) ---
    if state.leader.w_pending {
        let matched = state
            .bindings
            .matches(Context::LeaderWindow, "close_group", &key);
        clear_leader_state(state);
        return Some(if matched {
            Action::CloseGroup
        } else {
            Action::Render
        });
    }

    // --- Sub-menu: <leader>f -> e (export) / i (import) ---
    if state.leader.f_pending {
        let export = state
            .bindings
            .matches(Context::LeaderFile, "export_connections", &key);
        let import = state
            .bindings
            .matches(Context::LeaderFile, "import_connections", &key);
        clear_leader_state(state);
        return Some(if export {
            state.dialogs.export_dialog = Some(ExportDialogState::new());
            state.overlay = Some(Overlay::ExportDialog);
            Action::Render
        } else if import {
            state.dialogs.import_dialog = Some(ImportDialogState::new());
            state.overlay = Some(Overlay::ImportDialog);
            Action::Render
        } else {
            Action::Render
        });
    }

    // --- Sub-menu: <leader>q -> q (quit app) ---
    if state.leader.q_pending {
        let matched = state
            .bindings
            .matches(Context::LeaderQuit, "quit_app", &key);
        clear_leader_state(state);
        if !matched {
            return Some(Action::Render);
        }
        let has_unsaved = state.tabs.iter().any(|t| {
            t.editor.as_ref().is_some_and(|e| e.modified)
                || t.body_editor.as_ref().is_some_and(|e| e.modified)
                || t.decl_editor.as_ref().is_some_and(|e| e.modified)
                || !t.grid_changes.is_empty()
        });
        if has_unsaved {
            if let Some(idx) = state.tabs.iter().position(|t| {
                t.editor.as_ref().is_some_and(|e| e.modified)
                    || t.body_editor.as_ref().is_some_and(|e| e.modified)
                    || t.decl_editor.as_ref().is_some_and(|e| e.modified)
                    || !t.grid_changes.is_empty()
            }) {
                state.active_tab_idx = idx;
                state.focus = Focus::TabContent;
            }
            state.overlay = Some(Overlay::ConfirmQuit);
            return Some(Action::Render);
        }
        return Some(Action::Quit);
    }

    // --- Root leader menu ---
    if state.leader.pending {
        state.leader.pending = false;
        state.leader.pressed_at = None;
        // <leader><leader> — compile sub-menu trigger
        if let KeyCode::Char(c) = key.code
            && c == vimltui::LEADER_KEY
        {
            state.leader.leader_pending = true;
            return Some(Action::Render);
        }
        let b = &state.bindings;
        if b.matches(Context::Leader, "open_buffer_submenu", &key) {
            state.leader.b_pending = true;
            return Some(Action::Render);
        }
        if b.matches(Context::Leader, "open_window_submenu", &key) {
            state.leader.w_pending = true;
            return Some(Action::Render);
        }
        if b.matches(Context::Leader, "open_snippet_submenu", &key) {
            state.leader.s_pending = true;
            return Some(Action::Render);
        }
        if b.matches(Context::Leader, "open_file_submenu", &key) {
            state.leader.f_pending = true;
            return Some(Action::Render);
        }
        if b.matches(Context::Leader, "open_quit_submenu", &key) {
            state.leader.q_pending = true;
            return Some(Action::Render);
        }
        if b.matches(Context::Leader, "open_script_connection_picker", &key) {
            return Some(Action::OpenScriptConnPicker);
        }
        if b.matches(Context::Leader, "open_theme_picker", &key) {
            return Some(Action::OpenThemePicker);
        }
        if b.matches(Context::Leader, "toggle_diagnostic_list", &key) {
            state.engine.diagnostic_list_visible = !state.engine.diagnostic_list_visible;
            state.engine.diagnostic_list_cursor = 0;
            return Some(Action::Render);
        }
        if b.matches(Context::Leader, "toggle_sidebar", &key) {
            state.sidebar_visible = !state.sidebar_visible;
            if state.sidebar_visible {
                state.focus = Focus::Sidebar;
            } else if matches!(state.focus, Focus::Sidebar | Focus::ScriptsPanel) {
                state.focus = Focus::TabContent;
            }
            return Some(Action::Render);
        }
        if b.matches(Context::Leader, "vertical_split", &key) {
            return Some(Action::CreateSplit);
        }
        if b.matches(Context::Leader, "move_tab_to_other_group", &key) {
            return Some(Action::MoveTabToOther);
        }
        if b.matches(Context::Leader, "execute_query", &key) {
            if let Some(tab) = state.active_tab_mut() {
                let tab_id = tab.id;
                if matches!(tab.kind, TabKind::Script { .. })
                    && let Some(editor) = tab.active_editor_mut()
                {
                    let (query, start_line) =
                        if matches!(editor.mode, vimltui::VimMode::Visual(_)) {
                            let q = editor.selected_text().unwrap_or_default();
                            let sl = editor
                                .visual_anchor
                                .map(|(r, _)| r.min(editor.cursor_row))
                                .unwrap_or(editor.cursor_row);
                            editor.mode = vimltui::VimMode::Normal;
                            editor.visual_anchor = None;
                            (q, sl)
                        } else {
                            query_block_at_cursor(&editor.lines, editor.cursor_row)
                        };
                    if !query.trim().is_empty() {
                        return Some(maybe_prompt_bind_vars(
                            state, tab_id, query, start_line, false,
                        ));
                    }
                }
            }
            return Some(Action::Render);
        }
        if b.matches(Context::Leader, "execute_query_new_tab", &key) {
            if let Some(tab) = state.active_tab_mut() {
                let tab_id = tab.id;
                if matches!(tab.kind, TabKind::Script { .. })
                    && let Some(editor) = tab.active_editor_mut()
                {
                    let (query, start_line) =
                        if matches!(editor.mode, vimltui::VimMode::Visual(_)) {
                            let q = editor.selected_text().unwrap_or_default();
                            let sl = editor
                                .visual_anchor
                                .map(|(r, _)| r.min(editor.cursor_row))
                                .unwrap_or(editor.cursor_row);
                            editor.mode = vimltui::VimMode::Normal;
                            editor.visual_anchor = None;
                            (q, sl)
                        } else {
                            query_block_at_cursor(&editor.lines, editor.cursor_row)
                        };
                    if !query.trim().is_empty() {
                        return Some(maybe_prompt_bind_vars(
                            state, tab_id, query, start_line, true,
                        ));
                    }
                }
            }
            return Some(Action::Render);
        }
        return Some(Action::Render);
    }

    // --- Activate leader on Space press ---
    if let KeyCode::Char(c) = key.code
        && c == vimltui::LEADER_KEY
        && !key.modifiers.contains(KeyModifiers::CONTROL)
    {
        state.leader.pending = true;
        state.leader.pressed_at = Some(std::time::Instant::now());
        return Some(Action::Render);
    }

    None
}

/// Find the query block around the cursor.
/// Blocks are separated by 2+ consecutive blank lines.
/// Returns (query_text, start_line_in_editor).
/// Insert a multi-line template at the current cursor position, entering Insert mode.
/// Use `$` as a cursor marker in the template — the cursor will be placed there.
/// If no `$` marker is present, cursor goes at the end of the first template line.
fn insert_template(editor: &mut vimltui::VimEditor, template: &str) {
    editor.save_undo();
    let row = editor.cursor_row;
    let col = editor.cursor_col;
    let line = editor.lines.get(row).cloned().unwrap_or_default();
    let before = &line[..col.min(line.len())];
    let after = &line[col.min(line.len())..];

    // Remove cursor marker from template, remember its position
    let clean_tpl = template.replace('$', "");
    let tpl_lines: Vec<&str> = clean_tpl.lines().collect();

    let mut new_lines = Vec::new();
    new_lines.push(format!("{before}{}", tpl_lines.first().unwrap_or(&"")));
    for tpl_line in tpl_lines
        .iter()
        .skip(1)
        .take(tpl_lines.len().saturating_sub(2))
    {
        new_lines.push((*tpl_line).to_string());
    }
    if tpl_lines.len() > 1 {
        let last_tpl = tpl_lines.last().unwrap_or(&"");
        new_lines.push(format!("{last_tpl}{after}"));
    } else {
        let first = new_lines.last_mut().unwrap();
        first.push_str(after);
    }

    editor.lines[row] = new_lines[0].clone();
    for (i, nl) in new_lines[1..].iter().enumerate() {
        editor.lines.insert(row + 1 + i, nl.clone());
    }

    // Find cursor position from $ marker
    if let Some(marker_pos) = template.find('$') {
        // Count which line and column the marker is on
        let tpl_before_marker = &template[..marker_pos];
        let marker_row = tpl_before_marker.matches('\n').count();
        let marker_col = tpl_before_marker
            .rfind('\n')
            .map_or(before.len() + tpl_before_marker.len(), |nl| {
                tpl_before_marker.len() - nl - 1
            });
        editor.cursor_row = row + marker_row;
        editor.cursor_col = marker_col;
    } else {
        // No marker: cursor at end of first template line
        let first_tpl = tpl_lines.first().unwrap_or(&"");
        editor.cursor_row = row;
        editor.cursor_col = before.len() + first_tpl.len();
    }

    editor.mode = vimltui::VimMode::Insert;
    editor.modified = true;
}

fn query_block_at_cursor(lines: &[String], cursor_row: usize) -> (String, usize) {
    if cursor_row >= lines.len() {
        return (String::new(), 0);
    }

    // Helper: true if a line is blank (empty or only whitespace)
    let is_blank = |i: usize| lines[i].trim().is_empty();

    // Scan upward from cursor: find separator (2+ consecutive blank lines) or buffer start
    let mut start = 0;
    let mut consecutive_blanks = 0;
    for i in (0..cursor_row).rev() {
        if is_blank(i) {
            consecutive_blanks += 1;
            if consecutive_blanks >= 2 {
                start = i + consecutive_blanks;
                break;
            }
        } else {
            consecutive_blanks = 0;
        }
    }

    // Scan downward from cursor: find separator (2+ consecutive blank lines) or buffer end
    let mut end = lines.len() - 1;
    consecutive_blanks = 0;
    for i in (cursor_row + 1)..lines.len() {
        if is_blank(i) {
            consecutive_blanks += 1;
            if consecutive_blanks >= 2 {
                end = i - consecutive_blanks;
                break;
            }
        } else {
            consecutive_blanks = 0;
        }
    }

    // Trim leading/trailing blank lines within the block
    while start <= end && is_blank(start) {
        start += 1;
    }
    while end > start && is_blank(end) {
        end -= 1;
    }

    if start > end {
        return (String::new(), 0);
    }

    (lines[start..=end].join("\n"), start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(String::from).collect()
    }

    #[test]
    fn single_block_cursor_at_last_line() {
        let l = lines(
            "SELECT\n    *\nFROM orders ord\nLEFT JOIN customers cus\n     ON cus.customer_id = ord.customer_id",
        );
        // cursor on line 4 (ON ...) should return the full block
        let (q, start) = query_block_at_cursor(&l, 4);
        assert_eq!(start, 0);
        assert!(q.starts_with("SELECT"), "got: {q}");
    }

    #[test]
    fn single_block_cursor_at_middle() {
        let l = lines(
            "SELECT\n    *\nFROM orders ord\nLEFT JOIN customers cus\n     ON cus.customer_id = ord.customer_id",
        );
        let (q, start) = query_block_at_cursor(&l, 2);
        assert_eq!(start, 0);
        assert!(q.starts_with("SELECT"), "got: {q}");
    }

    #[test]
    fn single_block_cursor_at_first_line() {
        let l = lines("SELECT\n    *\nFROM orders ord");
        let (q, start) = query_block_at_cursor(&l, 0);
        assert_eq!(start, 0);
        assert!(q.starts_with("SELECT"), "got: {q}");
    }

    #[test]
    fn two_blocks_separated_by_double_blank() {
        let l = lines("SELECT 1;\n\n\nSELECT 2;");
        // cursor on "SELECT 2;" (line 3)
        let (q, start) = query_block_at_cursor(&l, 3);
        assert_eq!(start, 3);
        assert_eq!(q, "SELECT 2;");
        // cursor on "SELECT 1;" (line 0)
        let (q, start) = query_block_at_cursor(&l, 0);
        assert_eq!(start, 0);
        assert_eq!(q, "SELECT 1;");
    }

    #[test]
    fn block_with_single_blank_line_inside() {
        // single blank line should NOT split the block
        let l = lines("SELECT\n    *\n\nFROM orders");
        let (q, start) = query_block_at_cursor(&l, 3);
        assert_eq!(start, 0);
        assert!(q.starts_with("SELECT"), "got: {q}");
    }

    #[test]
    fn cursor_on_from_line_returns_full_select() {
        // Exact reproduction of user's case:
        // line 0: "SELECT "
        // line 1: "    *"
        // line 2: "FROM orders ord"
        // line 3: "LEFT JOIN customers cus"
        // line 4: "     ON cus.customer_id = ord.customer_id"
        let l = lines(
            "SELECT \n    *\nFROM orders ord\nLEFT JOIN customers cus\n     ON cus.customer_id = ord.customer_id",
        );
        for cursor in 0..5 {
            let (q, start) = query_block_at_cursor(&l, cursor);
            assert_eq!(start, 0, "cursor={cursor}, start should be 0");
            assert!(q.starts_with("SELECT"), "cursor={cursor}, got: {q}");
            assert!(
                q.contains("ord.customer_id"),
                "cursor={cursor}, should contain full query"
            );
        }
    }

    #[test]
    fn editor_starts_with_blank_line_then_query() {
        // What if there's a blank line at the top of the editor?
        let l = lines("\nSELECT\n    *\nFROM orders");
        let (q, start) = query_block_at_cursor(&l, 3);
        assert_eq!(start, 1, "should skip leading blank");
        assert!(q.starts_with("SELECT"), "got: {q}");
    }

    #[test]
    fn trailing_whitespace_lines() {
        // Lines with trailing whitespace should NOT be considered blank
        let l = vec![
            "SELECT ".to_string(),
            "    *".to_string(),
            "FROM orders ord".to_string(),
        ];
        let (q, start) = query_block_at_cursor(&l, 2);
        assert_eq!(start, 0);
        assert!(q.starts_with("SELECT"), "got: {q}");
    }

    #[test]
    fn cursor_descending_through_block() {
        // Simulate user executing from each line going down
        let l = lines(
            "SELECT\n    *\nFROM orders ord\nLEFT JOIN customers cus\n     ON cus.customer_id = ord.customer_id",
        );
        for row in 0..l.len() {
            let (q, start) = query_block_at_cursor(&l, row);
            assert_eq!(
                start, 0,
                "row={row}: start should be 0, got {start}. Query: {q}"
            );
            assert_eq!(q.lines().count(), 5, "row={row}: should have 5 lines");
        }
    }

    #[test]
    fn multiple_queries_cursor_on_second() {
        let l = lines(
            "SELECT 1;\n\n\nSELECT\n    *\nFROM orders ord\nLEFT JOIN customers cus\n     ON cus.customer_id = ord.customer_id",
        );
        // cursor on "FROM orders ord" (line 5)
        let (q, start) = query_block_at_cursor(&l, 5);
        assert_eq!(start, 3, "second block starts at line 3");
        assert!(q.starts_with("SELECT"), "got: {q}");
        assert!(
            q.contains("ord.customer_id"),
            "should have full second query, got: {q}"
        );
    }
}
