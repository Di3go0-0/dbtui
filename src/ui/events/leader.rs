use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::ui::state::{AppState, ExportDialogState, ImportDialogState, Overlay};
use crate::ui::tabs::TabKind;

use super::overlays::maybe_prompt_bind_vars;
use super::Action;

// --- Leader key for non-editor views ---

/// Resolve a leader sub-menu: clear pending flags, check if the key matches
/// the expected char, and return the action if so (or Render otherwise).
pub(super) fn resolve_leader_submenu(
    state: &mut AppState,
    key_code: KeyCode,
    expected: char,
    action: Action,
) -> Option<Action> {
    state.leader_leader_pending = false;
    state.leader_b_pending = false;
    state.leader_w_pending = false;
    state.leader_s_pending = false;
    state.leader_pending = false;
    state.leader_pressed_at = None;
    Some(if let KeyCode::Char(c) = key_code {
        if c == expected {
            action
        } else {
            Action::Render
        }
    } else {
        Action::Render
    })
}

/// Global leader key handler — works from any panel.
/// Returns Some(Action) if the key was consumed, None otherwise.
pub(super) fn handle_global_leader(state: &mut AppState, key: KeyEvent) -> Option<Action> {
    // --- Sub-menu: <leader><leader> -> s ---
    if state.leader_leader_pending {
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
        return resolve_leader_submenu(state, key.code, 's', action);
    }

    // --- Sub-menu: <leader>s -> s (SELECT template) ---
    if state.leader_s_pending {
        state.leader_s_pending = false;
        state.leader_b_pending = false;
        state.leader_w_pending = false;
        state.leader_pending = false;
        state.leader_pressed_at = None;
        if let KeyCode::Char('s') = key.code
            && let Some(tab) = state.active_tab_mut()
            && matches!(tab.kind, TabKind::Script { .. })
            && let Some(editor) = tab.active_editor_mut()
        {
            let template = "SELECT\n    *\nFROM ";
            editor.save_undo();
            let row = editor.cursor_row;
            let col = editor.cursor_col;
            let line = editor.lines.get(row).cloned().unwrap_or_default();
            let before = &line[..col.min(line.len())];
            let after = &line[col.min(line.len())..];

            let tpl_lines: Vec<&str> = template.lines().collect();
            let mut new_lines = Vec::new();
            new_lines.push(format!("{before}{}", tpl_lines[0]));
            for tpl_line in &tpl_lines[1..tpl_lines.len() - 1] {
                new_lines.push((*tpl_line).to_string());
            }
            let last_tpl = tpl_lines.last().unwrap_or(&"");
            new_lines.push(format!("{last_tpl}{after}"));

            editor.lines[row] = new_lines[0].clone();
            for (i, nl) in new_lines[1..].iter().enumerate() {
                editor.lines.insert(row + 1 + i, nl.clone());
            }

            editor.cursor_row = row + tpl_lines.len() - 1;
            editor.cursor_col = last_tpl.len();
            editor.mode = vimltui::VimMode::Insert;
        }
        return Some(Action::Render);
    }

    // --- Sub-menu: <leader>b -> d ---
    if state.leader_b_pending {
        return resolve_leader_submenu(state, key.code, 'd', Action::CloseTab);
    }

    // --- Sub-menu: <leader>w -> d ---
    if state.leader_w_pending {
        return resolve_leader_submenu(state, key.code, 'd', Action::CloseResultTab);
    }

    // --- Root leader menu ---
    if state.leader_pending {
        state.leader_pending = false;
        state.leader_pressed_at = None;
        return Some(match key.code {
            KeyCode::Char(c) if c == vimltui::LEADER_KEY => {
                state.leader_leader_pending = true;
                Action::Render
            }
            KeyCode::Char('b') => {
                state.leader_b_pending = true;
                Action::Render
            }
            KeyCode::Char('w') => {
                state.leader_w_pending = true;
                Action::Render
            }
            KeyCode::Char('s') => {
                state.leader_s_pending = true;
                Action::Render
            }
            KeyCode::Char('c') => Action::OpenScriptConnPicker,
            KeyCode::Char('t') => Action::OpenThemePicker,
            KeyCode::Char('e') => {
                state.export_dialog = Some(ExportDialogState::new());
                state.overlay = Some(Overlay::ExportDialog);
                Action::Render
            }
            KeyCode::Char('i') => {
                state.import_dialog = Some(ImportDialogState::new());
                state.overlay = Some(Overlay::ImportDialog);
                Action::Render
            }
            KeyCode::Enter => {
                // Execute query (script tabs only)
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
                Action::Render
            }
            KeyCode::Char('/') => {
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
                Action::Render
            }
            _ => Action::Render,
        });
    }

    // --- Activate leader on Space press ---
    if let KeyCode::Char(c) = key.code
        && c == vimltui::LEADER_KEY
        && !key.modifiers.contains(KeyModifiers::CONTROL)
    {
        state.leader_pending = true;
        state.leader_pressed_at = Some(std::time::Instant::now());
        return Some(Action::Render);
    }

    None
}

/// Find the query block around the cursor.
/// Blocks are separated by 2+ consecutive blank lines.
/// Returns (query_text, start_line_in_editor).
fn query_block_at_cursor(lines: &[String], cursor_row: usize) -> (String, usize) {
    let row = cursor_row;
    if row >= lines.len() {
        return (String::new(), 0);
    }

    // Scan upward: find start of block (after 2+ blank lines or buffer start)
    let mut start = row;
    let mut blanks = 0;
    if start > 0 {
        let mut i = row;
        while i > 0 {
            i -= 1;
            if lines[i].trim().is_empty() {
                blanks += 1;
                if blanks >= 2 {
                    start = i + blanks; // skip the blank lines
                    break;
                }
            } else {
                blanks = 0;
                start = i;
            }
        }
        if blanks < 2 {
            start = if lines[0].trim().is_empty() && blanks >= 1 {
                // Started from a blank region at top
                row.saturating_sub(blanks) + 1
            } else {
                0
            };
        }
    }

    // Scan downward: find end of block (before 2+ blank lines or buffer end)
    let mut end = row;
    blanks = 0;
    for (i, line) in lines.iter().enumerate().skip(row + 1) {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks >= 2 {
                break;
            }
        } else {
            blanks = 0;
            end = i;
        }
    }

    // Skip leading/trailing blank lines within the block
    while start <= end && lines[start].trim().is_empty() {
        start += 1;
    }
    while end > start && lines[end].trim().is_empty() {
        end -= 1;
    }

    if start > end {
        return (String::new(), 0);
    }

    (lines[start..=end].join("\n"), start)
}
