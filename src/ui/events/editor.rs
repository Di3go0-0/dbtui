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
    let db_type = state.conn.db_type;

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
                    // Forced completion (Ctrl+Space). If the analyzer needs to
                    // load something on demand (column cache, package members,
                    // etc.) we still want that action to fire so the next
                    // refresh has the data, otherwise the popup stays empty
                    // forever for unloaded targets.
                    if let Some(action) = update_completion_impl(state, true) {
                        return action;
                    }
                    return Action::Render;
                }
                KeyCode::Char('n') => {
                    if let Some(ref mut cmp) = state.engine.completion {
                        cmp.next();
                    }
                    return Action::Render;
                }
                KeyCode::Char('p') => {
                    if let Some(ref mut cmp) = state.engine.completion {
                        cmp.prev();
                    }
                    return Action::Render;
                }
                KeyCode::Char('y') => {
                    if let Some(cmp) = state.engine.completion.take() {
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
        if key.code == KeyCode::Enter && state.engine.completion.is_some() {
            if let Some(cmp) = state.engine.completion.take() {
                accept_completion(state, &cmp);
                // Re-trigger completion (e.g., after alias. -> show columns)
                if let Some(action) = update_completion_impl(state, false) {
                    return action;
                }
            }
            return Action::Render;
        }
        // Escape closes completion
        if key.code == KeyCode::Esc && state.engine.completion.is_some() {
            state.engine.completion = None;
        }
    }

    // --- Normal mode: diagnostic navigation, sub-views, hover ---
    if !in_insert {
        // Clear hover on any keypress
        state.engine.diagnostic_hover = None;

        // Ctrl+] / Ctrl+[ → navigate diagnostics
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char(']') | KeyCode::Char('['))
            && !state.engine.diagnostics.is_empty()
        {
            let cur = state.engine.diagnostic_list_cursor;
            let len = state.engine.diagnostics.len();
            let idx = if matches!(key.code, KeyCode::Char(']')) {
                if cur + 1 < len { cur + 1 } else { 0 }
            } else if cur == 0 {
                len - 1
            } else {
                cur - 1
            };
            state.engine.diagnostic_list_cursor = idx;
            let target_row = state.engine.diagnostics[idx].row;
            let target_col = state.engine.diagnostics[idx].col_start;
            if let Some(editor) = state.tabs[tab_idx].active_editor_mut() {
                editor.cursor_row = target_row;
                editor.cursor_col = target_col;
                editor.ensure_cursor_visible();
            }
            return Action::Render;
        }

        // ] / [ sub-view switching is handled globally in handle_global_normal_keys
    }

    // Pass key to editor and collect result + state
    let (action, still_insert, needs_diag, leaving_insert) = {
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
                EditorAction::ForceClose => Action::CloseTab,
                EditorAction::SaveAndClose => {
                    if is_script {
                        return Action::SaveScript;
                    }
                    Action::CloseTab
                }
                EditorAction::ToggleComment => {
                    toggle_line_comment(editor, db_type);
                    Action::Render
                }
                EditorAction::ToggleBlockComment { start_row, end_row } => {
                    toggle_block_comment(editor, db_type, start_row, end_row);
                    Action::Render
                }
                EditorAction::Hover => {
                    // K → show diagnostic hover for current line
                    let row = editor.cursor_row;
                    if let Some(diag) = state.engine.diagnostics.iter().find(|d| d.row == row) {
                        state.engine.diagnostic_hover =
                            Some((row, format!("[{}] {}", diag.source.label(), diag.message)));
                    }
                    Action::Render
                }
                EditorAction::GoToDefinition => {
                    // Future: jump to table/object definition
                    Action::None
                }
            };
            // Auto-pair: insert closing bracket after opening bracket
            if matches!(editor.mode, vimltui::VimMode::Insert)
                && let KeyCode::Char(ch) = key.code
                && !key.modifiers.contains(KeyModifiers::CONTROL)
            {
                let close = match ch {
                    '(' => Some(')'),
                    '[' => Some(']'),
                    '{' => Some('}'),
                    '\'' => Some('\''),
                    _ => None,
                };
                if let Some(c) = close {
                    let row = editor.cursor_row;
                    let col = editor.cursor_col;
                    if row < editor.lines.len() {
                        let line = &mut editor.lines[row];
                        if col <= line.len() {
                            line.insert(col, c);
                            // Cursor stays between the pair
                        }
                    }
                }
            }
            let still_insert = matches!(editor.mode, vimltui::VimMode::Insert);
            // Run diagnostics on Insert->Normal transitions immediately, and
            // also while still in Insert mode but throttled to ~150ms so the
            // user gets near-live feedback without re-parsing on every key.
            let modified_in_insert = editor.modified && in_insert && state.metadata_ready;
            let leaving_insert = !still_insert && modified_in_insert;
            let typing_in_insert = still_insert && modified_in_insert;
            let now = std::time::Instant::now();
            let debounce_elapsed = state
                .engine
                .last_diagnostic_run
                .map(|t| now.duration_since(t) >= std::time::Duration::from_millis(150))
                .unwrap_or(true);
            let needs_diag = leaving_insert || (typing_in_insert && debounce_elapsed);
            (action, still_insert, needs_diag, leaving_insert)
        } else {
            return Action::None;
        }
    };
    // tab/editor borrows are dropped here

    // Check if content reverted to saved state (undo back to original)
    if !still_insert && in_insert {
        state.tabs[tab_idx].check_modified();
    }

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
        state.engine.completion = None;
    }

    if needs_diag {
        let tab = &state.tabs[tab_idx];
        // Skip diagnostics for PL/SQL source tabs (sqlparser can't parse them)
        let is_plsql = matches!(
            tab.kind,
            TabKind::Package { .. } | TabKind::Function { .. } | TabKind::Procedure { .. }
        );
        if is_plsql {
            state.engine.diagnostics.clear();
        } else {
            let lines = tab
                .active_editor()
                .map(|e| e.lines.clone())
                .unwrap_or_default();
            // Use the new sql_engine diagnostic provider
            let eff_conn = state
                .tabs
                .get(tab_idx)
                .and_then(|t| t.kind.conn_name().map(|s| s.to_string()))
                .or_else(|| state.conn.name.clone());
            let empty_idx = crate::sql_engine::metadata::MetadataIndex::new();
            let metadata_idx = eff_conn
                .as_ref()
                .and_then(|cn| state.engine.metadata_indexes.get(cn))
                .unwrap_or(&empty_idx);
            let db_type = metadata_idx.db_type();
            let dialect_box = db_type
                .map(crate::sql_engine::dialect::dialect_for)
                .unwrap_or_else(|| Box::new(crate::sql_engine::dialect::OracleDialect));
            let provider = crate::sql_engine::diagnostics::DiagnosticProvider::new(
                dialect_box.as_ref(),
                metadata_idx,
            );
            let engine_diags = provider.check_local(&lines);
            state.engine.diagnostics = engine_diags
                .into_iter()
                .map(crate::ui::diagnostics::Diagnostic::from_engine)
                .collect();

            // Build gutter signs from diagnostics
            apply_diagnostic_gutter_signs(state, tab_idx);

            // Pass 4: schedule server-side compile check (async, debounced).
            // Only dispatch when leaving insert mode and if enough time has
            // passed since the last server diagnostic request (300ms debounce).
            if leaving_insert {
                let now = std::time::Instant::now();
                let server_debounce_ok = state
                    .engine
                    .last_server_diag_dispatch
                    .map(|t| now.duration_since(t) >= std::time::Duration::from_millis(300))
                    .unwrap_or(true);
                if server_debounce_ok && let Some(conn_name) = eff_conn.clone() {
                    let sql = lines.join("\n");
                    if !sql.trim().is_empty() {
                        state.engine.pending_server_diag = Some((sql, conn_name));
                    }
                }
            }
        }
        // Mark the run so the in-insert-mode debounce knows when to fire next.
        state.engine.last_diagnostic_run = Some(std::time::Instant::now());
    }

    action
}

/// Set diagnostic signs on the active editor's gutter (left of line numbers).
/// Uses the new `DiagnosticSign` API — separate from diff `GutterSign` (right of numbers).
pub fn apply_diagnostic_gutter_signs(state: &mut AppState, tab_idx: usize) {
    use crate::ui::diagnostics::Severity;
    use std::collections::HashMap;
    let mut diag_signs: HashMap<usize, vimltui::Diagnostic> = HashMap::new();
    for d in &state.engine.diagnostics {
        let sev = match d.severity {
            Severity::Error => vimltui::DiagnosticSeverity::Error,
            Severity::Warning | Severity::Info | Severity::Hint => {
                vimltui::DiagnosticSeverity::Warning
            }
        };
        diag_signs
            .entry(d.row)
            .and_modify(|existing| {
                if sev == vimltui::DiagnosticSeverity::Error {
                    existing.severity = vimltui::DiagnosticSeverity::Error;
                    existing.message = Some(d.message.clone());
                }
            })
            .or_insert(vimltui::Diagnostic {
                severity: sev,
                message: Some(d.message.clone()),
            });
    }

    if let Some(tab) = state.tabs.get_mut(tab_idx)
        && let Some(editor) = tab.active_editor_mut()
    {
        if diag_signs.is_empty() && editor.gutter.is_none() {
            return;
        }
        let mut config = editor.gutter.take().unwrap_or_default();
        config.diagnostics = diag_signs;
        if config.signs.is_empty() && config.diagnostics.is_empty() {
            editor.gutter = None;
        } else {
            editor.gutter = Some(config);
        }
    }
}

/// Update completion popup (auto-trigger, requires prefix).
pub(super) fn update_completion(state: &mut AppState) -> Option<Action> {
    update_completion_impl(state, false)
}

/// Update completion popup. `force=true` opens even without prefix (Ctrl+Space).
pub(crate) fn update_completion_impl(state: &mut AppState, force: bool) -> Option<Action> {
    use crate::sql_engine::analyzer::SemanticAnalyzer;
    use crate::sql_engine::completion::{CompletionItemKind, CompletionProvider};
    use crate::sql_engine::dialect;
    use crate::sql_engine::tokenizer;
    use crate::ui::completion::{CompletionItem, CompletionKind, CompletionState};

    let tab = match state.tabs.get(state.active_tab_idx) {
        Some(t) => t,
        None => {
            state.engine.completion = None;
            return None;
        }
    };
    let editor = match tab.active_editor() {
        Some(e) => e,
        None => {
            state.engine.completion = None;
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
    // After * in SELECT list: user likely wants to replace * with columns
    let star_mode = prefix.is_empty() && col > 0 && bytes.get(col - 1) == Some(&b'*');

    // Allow empty prefix for dot completions, star replacement, or forced mode (Ctrl+Space)
    if prefix.is_empty() && !dot_mode && !star_mode && !force {
        // Auto-correct case if the old prefix is an exact case-insensitive match
        if let Some(cmp) = state.engine.completion.take() {
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

    // Find the effective connection for this tab
    let eff_conn = state
        .active_tab()
        .and_then(|t| t.kind.conn_name().map(|s| s.to_string()))
        .or_else(|| state.conn.name.clone());

    let empty_idx = crate::sql_engine::metadata::MetadataIndex::new();
    let metadata_idx = eff_conn
        .as_ref()
        .and_then(|cn| state.engine.metadata_indexes.get(cn))
        .unwrap_or(&empty_idx);

    let db_type = metadata_idx.db_type();
    let dialect_box = db_type
        .map(dialect::dialect_for)
        .unwrap_or_else(|| Box::new(dialect::OracleDialect));
    let analyzer = SemanticAnalyzer::new(dialect_box.as_ref(), metadata_idx);
    let ctx = analyzer.analyze(&lines, block_row, col);
    let provider = CompletionProvider::new(dialect_box.as_ref(), metadata_idx);
    let scored_items = provider.complete(&ctx);

    // Determine if this is a table reference context (FROM/JOIN) for alias generation
    let is_table_ref = matches!(
        ctx.cursor_context,
        crate::sql_engine::context::CursorContext::TableRef
            | crate::sql_engine::context::CursorContext::SchemaDot {
                in_table_ref: true,
                ..
            }
    );
    // Collect existing aliases in the query to avoid conflicts
    let existing_aliases: Vec<String> = ctx.aliases.keys().cloned().collect();

    // Star expansion: when cursor is on `*` in a SELECT list and columns
    // are available, offer to replace `*` with the actual column list.
    let star_expansion_items: Vec<CompletionItem> = if star_mode
        && matches!(
            ctx.cursor_context,
            crate::sql_engine::context::CursorContext::SelectList
        )
        && !ctx.available_columns.is_empty()
    {
        let multiple_tables = ctx.table_refs.len() > 1;

        // Build a map from normalized table name -> preferred qualifier
        // (alias if present, otherwise table name).
        let mut table_qualifier: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for tref in &ctx.table_refs {
            let norm_name = dialect_box.normalize_identifier(&tref.reference.qualified_name.name);
            let qualifier = tref
                .reference
                .alias
                .as_ref()
                .unwrap_or(&tref.reference.qualified_name.name)
                .clone();
            table_qualifier.insert(norm_name, qualifier);
        }

        // Build the full comma-separated column list
        let mut col_parts: Vec<String> = Vec::new();
        for col_info in &ctx.available_columns {
            let norm_table = dialect_box.normalize_identifier(&col_info.table_name);
            if multiple_tables {
                let qualifier = table_qualifier
                    .get(&norm_table)
                    .cloned()
                    .unwrap_or_else(|| col_info.table_name.clone());
                col_parts.push(format!("{}.{}", qualifier, col_info.name));
            } else {
                col_parts.push(col_info.name.clone());
            }
        }

        let full_list = col_parts.join(", ");
        let mut items = vec![CompletionItem {
            label: full_list,
            kind: CompletionKind::Column,
            match_positions: vec![],
            detail: Some("Expand * to all columns".to_string()),
        }];

        // Also offer individual columns
        for col_info in &ctx.available_columns {
            let norm_table = dialect_box.normalize_identifier(&col_info.table_name);
            let label = if multiple_tables {
                let qualifier = table_qualifier
                    .get(&norm_table)
                    .cloned()
                    .unwrap_or_else(|| col_info.table_name.clone());
                format!("{}.{}", qualifier, col_info.name)
            } else {
                col_info.name.clone()
            };
            items.push(CompletionItem {
                label,
                kind: CompletionKind::Column,
                match_positions: vec![],
                detail: Some(col_info.data_type.clone()),
            });
        }

        items
    } else {
        Vec::new()
    };

    // Convert ScoredItem -> UI CompletionItem
    let items: Vec<CompletionItem> = if !star_expansion_items.is_empty() {
        star_expansion_items
    } else {
        scored_items
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
                    match_positions: si.match_positions,
                    detail: si.detail,
                }
            })
            .collect()
    };

    // If no column completions found and cursor is in a dot context,
    // trigger on-demand column loading
    let has_dot = dot_mode || {
        let before = &lines[block_row][..col.min(lines[block_row].len())];
        tokenizer::identifier_before_dot(before).is_some()
    };

    // Trigger on-demand column loading for SET clause (UPDATE table SET ...)
    let set_table_cache =
        if let crate::sql_engine::context::CursorContext::SetClause { ref target_table } =
            ctx.cursor_context
        {
            let has_cols = ctx
                .columns_for(&target_table.name, &|s| dialect_box.normalize_identifier(s))
                .is_empty();
            if has_cols {
                resolve_table_for_cache(state, &lines, block_row, &target_table.name).and_then(
                    |(schema, table)| {
                        let key = format!("{}.{}", schema.to_uppercase(), table.to_uppercase());
                        if !state.engine.column_cache.contains_key(&key)
                            && !metadata_idx.has_columns_cached(&schema, &table)
                        {
                            Some(Action::CacheColumns { schema, table })
                        } else {
                            None
                        }
                    },
                )
            } else {
                None
            }
        } else {
            None
        };

    let cache_action = if items.is_empty() && has_dot {
        let before = &lines[block_row][..col.min(lines[block_row].len())];
        if let Some((table_ref, _)) = tokenizer::identifier_before_dot(before) {
            if metadata_idx.is_known_schema(table_ref) {
                // Schema is known but has no objects loaded yet -- trigger on-demand load
                Some(Action::CacheSchemaObjects {
                    schema: table_ref.to_string(),
                })
            } else {
                // Not a schema -- try to resolve as table/alias for column loading
                resolve_table_for_cache(state, &lines, block_row, table_ref).and_then(
                    |(schema, table)| {
                        let key = format!("{}.{}", schema.to_uppercase(), table.to_uppercase());
                        if !state.engine.column_cache.contains_key(&key)
                            && !metadata_idx.has_columns_cached(&schema, &table)
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

    // On-demand package member loading: when the cursor is sitting after
    // schema.package. (or pkg.) and the metadata index has no cached
    // members for that package, fire a load. The next keystroke / refresh
    // will see the populated cache.
    let pkg_cache_action = if let crate::sql_engine::context::CursorContext::PackageDot {
        ref schema,
        ref package,
    } = ctx.cursor_context
    {
        let resolved_schema = schema
            .clone()
            .or_else(|| metadata_idx.schema_for_package(package).map(String::from))
            .unwrap_or_default();
        if !resolved_schema.is_empty()
            && metadata_idx
                .package_members(&resolved_schema, package)
                .is_empty()
        {
            Some(Action::LoadPackageMembers {
                schema: resolved_schema,
                package: package.clone(),
            })
        } else {
            None
        }
    } else {
        None
    };

    // On-demand table-function return-column loading: when the cursor is
    // at `alias.<cursor>` and `alias` resolves to a TABLE(pkg.fn()) ref
    // whose pseudo-columns haven't been cached yet, fire a load.
    let fn_cols_action =
        if let crate::sql_engine::context::CursorContext::ColumnDot { ref table_ref } =
            ctx.cursor_context
        {
            let normalized = dialect_box.normalize_identifier(table_ref);
            ctx.table_refs
                .iter()
                .find(|t| {
                    t.reference
                        .alias
                        .as_ref()
                        .map(|a| dialect_box.normalize_identifier(a) == normalized)
                        .unwrap_or(false)
                        || dialect_box.normalize_identifier(&t.reference.qualified_name.name)
                            == normalized
                })
                .and_then(|t| t.reference.function_call.as_ref())
                .and_then(|fc| {
                    if metadata_idx.has_function_return_columns_cached(
                        fc.schema.as_deref(),
                        fc.package.as_deref(),
                        &fc.function,
                    ) {
                        None
                    } else {
                        Some(Action::LoadFunctionReturnColumns {
                            schema: fc.schema.clone(),
                            package: fc.package.clone(),
                            function: fc.function.clone(),
                        })
                    }
                })
        } else {
            None
        };

    // On-demand column loading for tables in scope when in SelectList/Predicate
    // context — ensures columns are available even without typing "alias."
    let scope_cols_action = if matches!(
        ctx.cursor_context,
        crate::sql_engine::context::CursorContext::SelectList
            | crate::sql_engine::context::CursorContext::Predicate
            | crate::sql_engine::context::CursorContext::OrderGroupBy
    ) && ctx.available_columns.is_empty()
    {
        // Find the first table ref whose columns aren't cached and trigger load
        ctx.table_refs.iter().find_map(|tref| {
            if tref.reference.function_call.is_some() {
                return None;
            }
            let schema = tref.resolved_schema.as_deref()?;
            let table = &tref.reference.qualified_name.name;
            if metadata_idx.has_columns_cached(schema, table) {
                return None;
            }
            Some(Action::CacheColumns {
                schema: schema.to_string(),
                table: table.to_string(),
            })
        })
    } else {
        None
    };

    let cache_action = cache_action
        .or(set_table_cache)
        .or(pkg_cache_action)
        .or(fn_cols_action)
        .or(scope_cols_action);

    if items.is_empty() {
        state.engine.completion = None;
        return cache_action;
    }

    let prev_cursor = state
        .engine
        .completion
        .as_ref()
        .map(|c| c.cursor.min(items.len().saturating_sub(1)))
        .unwrap_or(0);

    // In star mode, back up origin_col by one to include `*` in the
    // replacement range so accepting a completion replaces the star.
    let effective_origin = if star_mode && start_col > 0 {
        start_col - 1
    } else {
        start_col
    };

    state.engine.completion = Some(CompletionState {
        items,
        cursor: prev_cursor,
        prefix,
        origin_row: editor_row,
        origin_col: effective_origin,
        table_ref_context: is_table_ref,
        existing_aliases,
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
    let eff_conn = state
        .active_tab()
        .and_then(|t| t.kind.conn_name().map(|s| s.to_string()))
        .or_else(|| state.conn.name.clone());
    let empty_idx = crate::sql_engine::metadata::MetadataIndex::new();
    let metadata_idx = eff_conn
        .as_ref()
        .and_then(|cn| state.engine.metadata_indexes.get(cn))
        .unwrap_or(&empty_idx);
    if let Some(schema) = metadata_idx.resolve_schema_for(table_name) {
        return Some((schema.to_string(), table_name.to_string()));
    }
    let schema = crate::ui::completion::find_schema_for_table(state, table_name)?;
    Some((schema, table_name.to_string()))
}

/// Toggle a line comment (`-- `) on the current line.
/// If the line already starts with `-- `, remove it. Otherwise, add it.
fn toggle_line_comment(
    editor: &mut vimltui::VimEditor,
    _db_type: Option<crate::core::models::DatabaseType>,
) {
    let row = editor.cursor_row;
    if row >= editor.lines.len() {
        return;
    }
    editor.save_undo();
    let line = &editor.lines[row];
    let trimmed = line.trim_start();
    if trimmed.starts_with("-- ") {
        // Remove comment: find the `-- ` and remove it
        if let Some(pos) = line.find("-- ") {
            let mut new_line = String::with_capacity(line.len());
            new_line.push_str(&line[..pos]);
            new_line.push_str(&line[pos + 3..]);
            editor.lines[row] = new_line;
            editor.cursor_col = editor.cursor_col.saturating_sub(3);
        }
    } else if trimmed.starts_with("--") {
        // Remove comment without trailing space
        if let Some(pos) = line.find("--") {
            let mut new_line = String::with_capacity(line.len());
            new_line.push_str(&line[..pos]);
            new_line.push_str(&line[pos + 2..]);
            editor.lines[row] = new_line;
            editor.cursor_col = editor.cursor_col.saturating_sub(2);
        }
    } else {
        // Add comment: preserve leading whitespace
        let indent = line.len() - trimmed.len();
        let mut new_line = String::with_capacity(line.len() + 3);
        new_line.push_str(&line[..indent]);
        new_line.push_str("-- ");
        new_line.push_str(trimmed);
        editor.lines[row] = new_line;
        editor.cursor_col += 3;
    }
    editor.modified = true;
}

/// Toggle block comments on a range of lines.
/// If ALL lines are commented, uncomment them. Otherwise, comment all.
fn toggle_block_comment(
    editor: &mut vimltui::VimEditor,
    _db_type: Option<crate::core::models::DatabaseType>,
    start_row: usize,
    end_row: usize,
) {
    let end = end_row.min(editor.lines.len().saturating_sub(1));
    if start_row > end {
        return;
    }
    editor.save_undo();

    // Check if all lines in the range are already commented
    let all_commented = (start_row..=end).all(|r| {
        let trimmed = editor.lines[r].trim_start();
        trimmed.starts_with("--")
    });

    for r in start_row..=end {
        let line = &editor.lines[r];
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();

        if all_commented {
            // Uncomment
            let uncommented = trimmed
                .strip_prefix("-- ")
                .or_else(|| trimmed.strip_prefix("--"));
            if let Some(rest) = uncommented {
                let mut new_line = String::with_capacity(line.len());
                new_line.push_str(&line[..indent]);
                new_line.push_str(rest);
                editor.lines[r] = new_line;
            }
        } else {
            // Comment
            if !trimmed.starts_with("--") {
                let mut new_line = String::with_capacity(line.len() + 3);
                new_line.push_str(&line[..indent]);
                new_line.push_str("-- ");
                new_line.push_str(trimmed);
                editor.lines[r] = new_line;
            }
        }
    }
    editor.modified = true;
}

/// SQL reserved words that must never be used as aliases.
const RESERVED_ALIAS_WORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "AND",
    "OR",
    "NOT",
    "IN",
    "ON",
    "AS",
    "IS",
    "BY",
    "IF",
    "DO",
    "SET",
    "ALL",
    "ANY",
    "FOR",
    "TOP",
    "END",
    "ASC",
    "DESC",
    "JOIN",
    "LEFT",
    "RIGHT",
    "FULL",
    "INTO",
    "NULL",
    "LIKE",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "WITH",
    "OVER",
    "ORDER",
    "GROUP",
    "HAVING",
    "LIMIT",
    "UNION",
    "UPDATE",
    "DELETE",
    "INSERT",
    "CREATE",
    "ALTER",
    "DROP",
    "TABLE",
    "VIEW",
    "INDEX",
    "BETWEEN",
    "EXISTS",
    "DISTINCT",
    "VALUES",
    "INNER",
    "OUTER",
    "CROSS",
    "NATURAL",
    "USING",
    "OFFSET",
    "EXCEPT",
    "INTERSECT",
    "PRIMARY",
    "FOREIGN",
    "REFERENCES",
    "CONSTRAINT",
    "DEFAULT",
    "CHECK",
    "UNIQUE",
    "CASCADE",
    "TRUNCATE",
    "GRANT",
    "REVOKE",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "DECLARE",
    "FUNCTION",
    "PROCEDURE",
    "TRIGGER",
    "RETURN",
    "TO",
    "NO",
    "GO",
];

fn generate_table_alias(table_name: &str, existing: &[String]) -> String {
    let lower = table_name.to_lowercase();
    let parts: Vec<&str> = lower.split('_').filter(|p| !p.is_empty()).collect();

    let conflicts = |candidate: &str| -> bool {
        existing.iter().any(|a| a.eq_ignore_ascii_case(candidate))
            || RESERVED_ALIAS_WORDS
                .iter()
                .any(|r| r.eq_ignore_ascii_case(candidate))
    };

    // Build candidate list ordered by preference
    let mut candidates: Vec<String> = Vec::new();

    if parts.len() > 1 {
        // Multi-word: initials (2-3 chars)
        let initials: String = parts.iter().filter_map(|p| p.chars().next()).collect();
        // 2-char version
        if initials.len() >= 2 {
            candidates.push(initials.chars().take(2).collect());
        }
        // 3-char version
        if initials.len() >= 3 {
            candidates.push(initials.chars().take(3).collect());
        }
        // Fallback: first part 2 chars
        if parts[0].len() >= 2 {
            candidates.push(parts[0].chars().take(2).collect());
        }
        if parts[0].len() >= 3 {
            candidates.push(parts[0].chars().take(3).collect());
        }
    } else {
        // Single word: build progressively longer abbreviations
        let chars: Vec<char> = lower.chars().collect();
        // 2-char: first + next consonant
        if let Some(c) = chars[1..].iter().find(|c| !"aeiou".contains(**c)) {
            candidates.push(format!("{}{c}", chars[0]));
        }
        // 2-char: first 2 chars
        if chars.len() >= 2 {
            candidates.push(chars[..2].iter().collect());
        }
        // 3-char: first + 2 consonants
        let consonants: Vec<char> = chars[1..]
            .iter()
            .filter(|c| !"aeiou".contains(**c) && c.is_ascii_alphabetic())
            .copied()
            .collect();
        if consonants.len() >= 2 {
            candidates.push(format!("{}{}{}", chars[0], consonants[0], consonants[1]));
        }
        // 3-char: first 3 chars
        if chars.len() >= 3 {
            candidates.push(chars[..3].iter().collect());
        }
    }

    // Pick the first non-conflicting candidate
    for c in &candidates {
        if !conflicts(c) {
            return c.clone();
        }
    }

    // Last resort: first candidate + digit suffix
    let base = candidates
        .first()
        .cloned()
        .unwrap_or_else(|| "tb".to_string());
    for n in 2..100 {
        let candidate = format!("{base}{n}");
        if !conflicts(&candidate) {
            return candidate;
        }
    }

    base
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
        // Schema, Alias and Package all chain into another suggestion via "."
        // (schema → object, alias → column, package → member). Append the
        // dot so the user can keep typing without an extra keystroke.
        CompletionKind::Alias | CompletionKind::Schema | CompletionKind::Package => {
            (format!("{}.", item.label), false)
        }
        _ if needs_parens => (format!("{}()", item.label), true),
        // Tables/Views in FROM/JOIN context: append auto-generated alias
        CompletionKind::Table | CompletionKind::View if cmp.table_ref_context => {
            let alias = generate_table_alias(&item.label, &cmp.existing_aliases);
            (format!("{} {alias}", item.label), false)
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_single_word() {
        // "or" is reserved (SQL OR) → skips to next candidate
        let ord = generate_table_alias("orders", &[]);
        assert!(
            !RESERVED_ALIAS_WORDS
                .iter()
                .any(|r| r.eq_ignore_ascii_case(&ord)),
            "alias '{ord}' is a reserved word"
        );
        assert!(ord.len() <= 3, "orders → {ord}");

        let us = generate_table_alias("users", &[]);
        assert!(
            !RESERVED_ALIAS_WORDS
                .iter()
                .any(|r| r.eq_ignore_ascii_case(&us)),
            "alias '{us}' is a reserved word"
        );

        let cs = generate_table_alias("customers", &[]);
        assert_eq!(cs, "cs"); // c + s (not reserved)
    }

    #[test]
    fn alias_avoids_reserved_words() {
        // "or" (orders), "as" (assets), "in" (invoices), "on" (online_orders)
        // — all are SQL reserved words and must be skipped
        let alias = generate_table_alias("orders", &[]);
        assert_ne!(alias.to_uppercase(), "OR", "got {alias}");

        let alias = generate_table_alias("assets", &[]);
        assert_ne!(alias.to_uppercase(), "AS", "got {alias}");

        let alias = generate_table_alias("invoices", &[]);
        assert_ne!(alias.to_uppercase(), "IN", "got {alias}");

        let alias = generate_table_alias("online_orders", &[]);
        assert_ne!(alias.to_uppercase(), "ON", "got {alias}");
        // "oo" is not reserved → should be fine
    }

    #[test]
    fn alias_multi_word() {
        assert_eq!(generate_table_alias("customer_orders", &[]), "co");
        assert_eq!(generate_table_alias("order_line_items", &[]), "ol");
        assert_eq!(generate_table_alias("user_role_mapping", &[]), "ur");
    }

    #[test]
    fn alias_conflict_expands_to_3() {
        let alias = generate_table_alias("customers", &["cs".to_string()]);
        assert!(alias.len() <= 3, "got {alias}");
        assert_ne!(alias, "cs");
    }

    #[test]
    fn alias_no_duplicate_across_tables() {
        let existing = vec!["or".to_string()]; // from "orders"
        let alias = generate_table_alias("organisms", &existing);
        assert_ne!(alias, "or", "should not conflict");
    }

    #[test]
    fn alias_conflict_case_insensitive() {
        let existing = vec!["CO".to_string()];
        let alias = generate_table_alias("customer_orders", &existing);
        assert_ne!(alias.to_lowercase(), "co");
    }
}
