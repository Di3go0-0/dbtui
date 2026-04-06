use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use vimltui::EditorAction;

use crate::ui::state::AppState;
use crate::ui::tabs::{SubView, TabKind};

use super::Action;

pub(super) fn handle_tab_editor(state: &mut AppState, key: KeyEvent) -> Action {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return Action::None;
    }

    let tab = &mut state.tabs[tab_idx];
    let tab_id = tab.id;

    let is_script = matches!(tab.kind, TabKind::Script { .. });

    // Determine if this is a source code tab (Package/Function/Procedure)
    let is_source_tab = matches!(
        tab.kind,
        TabKind::Package { .. } | TabKind::Function { .. } | TabKind::Procedure { .. }
    );

    let in_insert = tab
        .active_editor()
        .is_some_and(|e| matches!(e.mode, vimltui::VimMode::Insert | vimltui::VimMode::Replace));

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // --- Completion keys in Insert mode ---
    // Ctrl+Space: open/refresh completion
    // Ctrl+N: next item, Ctrl+P: previous item, Ctrl+Y: accept
    if in_insert {
        if ctrl {
            match key.code {
                KeyCode::Char(' ') => {
                    update_completion_impl(state, true);
                    return Action::Render;
                }
                KeyCode::Char('n') => {
                    if let Some(ref mut cmp) = state.completion {
                        cmp.next();
                    }
                    return Action::Render;
                }
                KeyCode::Char('p') => {
                    if let Some(ref mut cmp) = state.completion {
                        cmp.prev();
                    }
                    return Action::Render;
                }
                KeyCode::Char('y') => {
                    if let Some(cmp) = state.completion.take() {
                        accept_completion(state, &cmp);
                        // Re-trigger completion (e.g., after alias. -> show columns)
                        if let Some(action) = update_completion_impl(state, false) {
                            return action;
                        }
                    }
                    return Action::Render;
                }
                _ => {}
            }
        }
        // Enter accepts completion if popup is open
        if key.code == KeyCode::Enter && state.completion.is_some() {
            if let Some(cmp) = state.completion.take() {
                accept_completion(state, &cmp);
                // Re-trigger completion (e.g., after alias. -> show columns)
                if let Some(action) = update_completion_impl(state, false) {
                    return action;
                }
            }
            return Action::Render;
        }
        // Escape closes completion
        if key.code == KeyCode::Esc && state.completion.is_some() {
            state.completion = None;
        }
    }

    // Pass key to editor and collect result + state
    let (action, still_insert, needs_diag) = {
        let tab = &mut state.tabs[tab_idx];
        if let Some(editor) = tab.active_editor_mut() {
            let action = match editor.handle_key(key) {
                EditorAction::Handled => Action::Render,
                EditorAction::Unhandled(_) => Action::None,
                EditorAction::Save => {
                    if is_source_tab {
                        Action::ValidateAndSave { tab_id }
                    } else {
                        Action::SaveScript
                    }
                }
                EditorAction::Close => Action::CloseTab,
                EditorAction::ForceClose => Action::Quit,
                EditorAction::SaveAndClose => {
                    if is_script {
                        return Action::SaveScript;
                    }
                    Action::CloseTab
                }
            };
            let still_insert = matches!(editor.mode, vimltui::VimMode::Insert);
            // Only run diagnostics on Insert->Normal transition and if metadata is loaded
            let needs_diag = !still_insert && in_insert && editor.modified && state.metadata_ready;
            (action, still_insert, needs_diag)
        } else {
            return Action::None;
        }
    };
    // tab/editor borrows are dropped here

    // Update diff signs for source editors (packages, functions, procedures)
    {
        let tab = &state.tabs[tab_idx];
        let original = match &tab.active_sub_view {
            Some(SubView::PackageDeclaration) | Some(SubView::TypeDeclaration) => {
                tab.original_decl.clone()
            }
            Some(SubView::PackageBody) | Some(SubView::TypeBody) => tab.original_body.clone(),
            None if matches!(
                tab.kind,
                TabKind::Function { .. } | TabKind::Procedure { .. }
            ) =>
            {
                tab.original_source.clone()
            }
            _ => None,
        };
        if let Some(orig) = original {
            let tab = &mut state.tabs[tab_idx];
            if let Some(editor) = tab.active_editor_mut() {
                let signs = super::compute_diff_signs(&orig, &editor.lines);
                if signs.is_empty() {
                    editor.gutter = None;
                } else {
                    let mut config = editor.gutter.take().unwrap_or_default();
                    config.signs = signs;
                    editor.gutter = Some(config);
                }
            }
        }
    }

    if still_insert {
        // Auto-update completion while typing in Insert mode
        if let Some(cache_action) = update_completion(state) {
            return cache_action;
        }
    } else {
        state.completion = None;
    }

    if needs_diag {
        let tab = &state.tabs[tab_idx];
        // Skip diagnostics for PL/SQL source tabs (sqlparser can't parse them)
        let is_plsql = matches!(
            tab.kind,
            TabKind::Package { .. } | TabKind::Function { .. } | TabKind::Procedure { .. }
        );
        if is_plsql {
            state.diagnostics.clear();
        } else {
            let lines = tab
                .active_editor()
                .map(|e| e.lines.clone())
                .unwrap_or_default();
            // Use the new sql_engine diagnostic provider
            let dialect_box = state
                .db_type
                .map(crate::sql_engine::dialect::dialect_for)
                .unwrap_or_else(|| Box::new(crate::sql_engine::dialect::OracleDialect));
            let provider = crate::sql_engine::diagnostics::DiagnosticProvider::new(
                dialect_box.as_ref(),
                &state.metadata_index,
            );
            let engine_diags = provider.check_local(&lines);
            // Convert sql_engine diagnostics to UI diagnostics
            state.diagnostics = engine_diags
                .into_iter()
                .map(|d| crate::ui::diagnostics::Diagnostic {
                    row: d.row,
                    col_start: d.col_start,
                    col_end: d.col_end,
                    message: d.message,
                })
                .collect();
        }
    }

    action
}

/// Update completion popup (auto-trigger, requires prefix).
pub(super) fn update_completion(state: &mut AppState) -> Option<Action> {
    update_completion_impl(state, false)
}

/// Update completion popup. `force=true` opens even without prefix (Ctrl+Space).
pub(super) fn update_completion_impl(state: &mut AppState, force: bool) -> Option<Action> {
    use crate::sql_engine::analyzer::SemanticAnalyzer;
    use crate::sql_engine::completion::{CompletionItemKind, CompletionProvider};
    use crate::sql_engine::dialect;
    use crate::sql_engine::tokenizer;
    use crate::ui::completion::{CompletionItem, CompletionKind, CompletionState};

    let tab = match state.tabs.get(state.active_tab_idx) {
        Some(t) => t,
        None => {
            state.completion = None;
            return None;
        }
    };
    let editor = match tab.active_editor() {
        Some(e) => e,
        None => {
            state.completion = None;
            return None;
        }
    };

    let row = editor.cursor_row;
    let col = editor.cursor_col;
    let line = editor.current_line();
    // Extract prefix from the current line directly
    let bytes = line.as_bytes();
    let end = col.min(bytes.len());
    let mut pstart = end;
    while pstart > 0 && (bytes[pstart - 1].is_ascii_alphanumeric() || bytes[pstart - 1] == b'_') {
        pstart -= 1;
    }
    let prefix = line[pstart..end].to_string();
    let start_col = pstart;
    let dot_mode = prefix.is_empty() && col > 0 && bytes.get(col - 1) == Some(&b'.');

    // Allow empty prefix for dot completions or forced mode (Ctrl+Space)
    if prefix.is_empty() && !dot_mode && !force {
        // Auto-correct case if the old prefix is an exact case-insensitive match
        if let Some(cmp) = state.completion.take() {
            let old_prefix_upper = cmp.prefix.to_uppercase();
            if !old_prefix_upper.is_empty() {
                let exact: Vec<_> = cmp
                    .items
                    .iter()
                    .filter(|item| item.label.to_uppercase() == old_prefix_upper)
                    .collect();
                if exact.len() == 1 && exact[0].label != cmp.prefix {
                    let tab = state.tabs.get_mut(state.active_tab_idx);
                    if let Some(editor) = tab.and_then(|t| t.active_editor_mut()) {
                        let r = cmp.origin_row;
                        if r < editor.lines.len() {
                            let line = &editor.lines[r];
                            let start = cmp.origin_col.min(line.len());
                            let end = (start + cmp.prefix.len()).min(line.len());
                            let mut new_line = String::with_capacity(line.len());
                            new_line.push_str(&line[..start]);
                            new_line.push_str(&exact[0].label);
                            new_line.push_str(&line[end..]);
                            editor.lines[r] = new_line;
                            let diff = exact[0].label.len() as isize - cmp.prefix.len() as isize;
                            if editor.cursor_row == r && editor.cursor_col > start {
                                editor.cursor_col =
                                    (editor.cursor_col as isize + diff).max(0) as usize;
                            }
                        }
                    }
                }
            }
        }
        return None;
    }

    // Clone only the query block lines (not the entire file)
    let editor_row = row;
    let total_lines = editor.lines.len();
    let mut block_start = row;
    while block_start > 0 && !editor.lines[block_start - 1].trim().is_empty() {
        block_start -= 1;
    }
    let mut block_end = row + 1;
    while block_end < total_lines && !editor.lines[block_end].trim().is_empty() {
        block_end += 1;
    }
    let lines: Vec<String> = editor.lines[block_start..block_end].to_vec();
    let block_row = row - block_start;

    // Use the new sql_engine for completion
    let dialect_box = state
        .db_type
        .map(dialect::dialect_for)
        .unwrap_or_else(|| Box::new(dialect::OracleDialect));
    let analyzer = SemanticAnalyzer::new(dialect_box.as_ref(), &state.metadata_index);
    let ctx = analyzer.analyze(&lines, block_row, col);
    let provider = CompletionProvider::new(dialect_box.as_ref(), &state.metadata_index);
    let scored_items = provider.complete(&ctx);

    // Convert ScoredItem -> UI CompletionItem
    let items: Vec<CompletionItem> = scored_items
        .into_iter()
        .map(|si| {
            let kind = match si.kind {
                CompletionItemKind::Keyword => CompletionKind::Keyword,
                CompletionItemKind::Schema => CompletionKind::Schema,
                CompletionItemKind::Table => CompletionKind::Table,
                CompletionItemKind::View => CompletionKind::View,
                CompletionItemKind::Column => CompletionKind::Column,
                CompletionItemKind::Package => CompletionKind::Package,
                CompletionItemKind::Function => CompletionKind::Function,
                CompletionItemKind::Procedure => CompletionKind::Procedure,
                CompletionItemKind::Alias => CompletionKind::Alias,
                CompletionItemKind::ForeignKeyJoin => CompletionKind::Table,
            };
            CompletionItem {
                label: si.label,
                kind,
            }
        })
        .collect();

    // If no column completions found and cursor is in a dot context,
    // trigger on-demand column loading
    let has_dot = dot_mode || {
        let before = &lines[block_row][..col.min(lines[block_row].len())];
        tokenizer::identifier_before_dot(before).is_some()
    };

    let cache_action = if items.is_empty() && has_dot {
        let before = &lines[block_row][..col.min(lines[block_row].len())];
        if let Some((table_ref, _)) = tokenizer::identifier_before_dot(before) {
            if state.metadata_index.is_known_schema(table_ref) {
                // Schema is known but has no objects loaded yet -- trigger on-demand load
                Some(Action::CacheSchemaObjects {
                    schema: table_ref.to_string(),
                })
            } else {
                // Not a schema -- try to resolve as table/alias for column loading
                resolve_table_for_cache(state, &lines, block_row, table_ref).and_then(
                    |(schema, table)| {
                        let key = format!("{}.{}", schema.to_uppercase(), table.to_uppercase());
                        if !state.column_cache.contains_key(&key)
                            && !state.metadata_index.has_columns_cached(&schema, &table)
                        {
                            Some(Action::CacheColumns { schema, table })
                        } else {
                            None
                        }
                    },
                )
            }
        } else {
            None
        }
    } else {
        None
    };

    if items.is_empty() {
        state.completion = None;
        return cache_action;
    }

    let prev_cursor = state
        .completion
        .as_ref()
        .map(|c| c.cursor.min(items.len().saturating_sub(1)))
        .unwrap_or(0);

    state.completion = Some(CompletionState {
        items,
        cursor: prev_cursor,
        prefix,
        origin_row: editor_row,
        origin_col: start_col,
    });

    cache_action
}

/// Resolve a table reference to (schema, table) for column cache loading.
pub(super) fn resolve_table_for_cache(
    state: &AppState,
    lines: &[String],
    _row: usize,
    table_ref: &str,
) -> Option<(String, String)> {
    // Try to resolve alias via the old resolver (still works fine)
    let block: Vec<String> = lines.to_vec();
    let resolved = crate::ui::completion::resolve_table_name(&block, table_ref);
    let table_name = resolved.as_deref().unwrap_or(table_ref);

    // Try MetadataIndex first, fall back to tree walk
    if let Some(schema) = state.metadata_index.resolve_schema_for(table_name) {
        return Some((schema.to_string(), table_name.to_string()));
    }
    let schema = crate::ui::completion::find_schema_for_table(state, table_name)?;
    Some((schema, table_name.to_string()))
}

/// Accept the selected completion item: replace prefix with completion text.
pub(super) fn accept_completion(
    state: &mut AppState,
    cmp: &crate::ui::completion::CompletionState,
) {
    use crate::ui::completion::CompletionKind;

    let item = match cmp.selected() {
        Some(s) => s.clone(),
        None => return,
    };

    // Append "." for alias/schema, "()" for functions/procedures and keywords that need parens
    let needs_parens = match item.kind {
        CompletionKind::Function | CompletionKind::Procedure => true,
        CompletionKind::Keyword => {
            matches!(
                item.label.as_str(),
                "IN" | "EXISTS" | "NOT IN" | "NOT EXISTS"
            )
        }
        _ => false,
    };
    // cursor_inside_parens: place cursor between () instead of after
    let (insert_text, cursor_inside_parens) = match item.kind {
        CompletionKind::Alias | CompletionKind::Schema => (format!("{}.", item.label), false),
        _ if needs_parens => (format!("{}()", item.label), true),
        _ => (item.label, false),
    };

    let tab = match state.tabs.get_mut(state.active_tab_idx) {
        Some(t) => t,
        None => return,
    };
    let editor = match tab.active_editor_mut() {
        Some(e) => e,
        None => return,
    };

    let row = cmp.origin_row;
    if row >= editor.lines.len() {
        return;
    }

    let line = &editor.lines[row];
    let start = cmp.origin_col.min(line.len());
    let end = editor.cursor_col.min(line.len());

    let mut new_line = String::with_capacity(line.len() + insert_text.len());
    new_line.push_str(&line[..start]);
    new_line.push_str(&insert_text);
    if end < line.len() {
        new_line.push_str(&line[end..]);
    }

    editor.lines[row] = new_line;
    editor.cursor_col = if cursor_inside_parens {
        // Place cursor between the parens: "COUNT(|)"
        start + insert_text.len() - 1
    } else {
        start + insert_text.len()
    };
    editor.modified = true;
}
