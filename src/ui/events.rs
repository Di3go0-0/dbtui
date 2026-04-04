use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::time::Duration;

use crate::ui::state::{AppState, Focus, LeafKind, Mode, Overlay, TreeNode};
use crate::ui::tabs::{SubView, TabId, TabKind, WorkspaceTab};
use vimltui::EditorAction;

pub enum Action {
    Quit,
    Render,
    None,
    LoadSchemas {
        conn_name: String,
    },
    SaveSchemaFilter,
    LoadChildren {
        schema: String,
        kind: String,
    },
    LoadTableData {
        tab_id: TabId,
        schema: String,
        table: String,
    },
    LoadPackageContent {
        tab_id: TabId,
        schema: String,
        name: String,
    },
    ExecuteQuery {
        tab_id: TabId,
        query: String,
        start_line: usize,
    },
    ExecuteQueryNewTab {
        tab_id: TabId,
        query: String,
        start_line: usize,
    },
    LoadSourceCode {
        tab_id: TabId,
        schema: String,
        name: String,
        obj_type: String,
    },
    OpenNewScript,
    OpenScript {
        name: String,
    },
    DeleteScript {
        name: String,
    },
    DuplicateScript {
        name: String,
    },
    RenameScript {
        old_name: String,
        new_name: String,
    },
    CloseTab,
    SaveScript,
    SaveScriptAs {
        name: String,
    },
    ConfirmCloseYes,
    ConfirmCloseNo,
    Connect,
    ConnectByName {
        name: String,
    },
    DisconnectByName {
        name: String,
    },
    SaveConnection,
    DeleteConnection {
        name: String,
    },
    CloseResultTab,
    OpenThemePicker,
    SetTheme {
        name: String,
    },
    ValidateAndSave {
        tab_id: TabId,
    },
    CompileToDb {
        tab_id: TabId,
    },
    OpenScriptConnPicker,
    SetScriptConnection {
        conn_name: String,
    },
    CacheColumns {
        schema: String,
        table: String,
    },
}

pub enum InputEvent {
    Key(KeyEvent),
    Paste(String),
}

pub fn poll_event(timeout: Duration) -> Option<InputEvent> {
    if event::poll(timeout).ok()? {
        match event::read().ok()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => return Some(InputEvent::Key(key)),
            Event::Paste(text) => return Some(InputEvent::Paste(text)),
            _ => {}
        }
    }
    None
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Action {
    // Check editor state for input blocking
    let (editor_in_insert, editor_in_special) = if state.focus == Focus::TabContent {
        if let Some(e) = state.active_tab().and_then(|t| t.active_editor()) {
            let in_insert = matches!(e.mode, vimltui::VimMode::Insert | vimltui::VimMode::Replace)
                || e.command_active
                || e.search.active;
            let in_special =
                !matches!(e.mode, vimltui::VimMode::Normal) || e.command_active || e.search.active;
            (in_insert, in_special)
        } else {
            (false, false)
        }
    } else {
        (false, false)
    };

    // Global leader key handling (works in Normal AND Visual mode)
    // Skip only if editor is in Insert/command/search mode
    if state.overlay.is_none()
        && !editor_in_insert
        && !state.tree_state.search_active
        && let Some(action) = handle_global_leader(state, key)
    {
        return action;
    }

    // Handle overlays first
    if let Some(overlay) = &state.overlay {
        return match overlay {
            Overlay::ConnectionDialog => handle_connection_dialog(state, key),
            Overlay::Help => handle_help_overlay(state, key),
            Overlay::ObjectFilter => handle_object_filter(state, key),
            Overlay::ConnectionMenu => handle_conn_menu(state, key),
            Overlay::GroupMenu => handle_group_menu(state, key),
            Overlay::ConfirmClose => handle_confirm_close(state, key),
            Overlay::ConfirmQuit => handle_confirm_quit(state, key),
            Overlay::SaveScriptName => handle_save_script_name(state, key),
            Overlay::ScriptConnection => handle_script_conn_picker(state, key),
            Overlay::ThemePicker => handle_theme_picker(state, key),
            Overlay::BindVariables => handle_bind_variables(state, key),
        };
    }

    // Handle sidebar search mode
    if state.tree_state.search_active {
        return handle_sidebar_search(state, key);
    }

    if let Some(action) = handle_global_normal_keys(state, key, editor_in_special) {
        return action;
    }

    if let Some(action) = handle_spatial_navigation(state, key, editor_in_special) {
        return action;
    }

    match state.focus {
        Focus::Sidebar => handle_sidebar(state, key),
        Focus::ScriptsPanel => handle_scripts_panel(state, key),
        Focus::TabContent => handle_tab_content(state, key),
    }
}

/// Handle global Normal-mode keys (quit, help, tab switching, etc.).
/// Returns Some(Action) if the key was consumed.
fn handle_global_normal_keys(
    state: &mut AppState,
    key: KeyEvent,
    in_editor_special_mode: bool,
) -> Option<Action> {
    if state.mode != Mode::Normal || in_editor_special_mode {
        return None;
    }

    match key.code {
        KeyCode::Char('q') => {
            // Check for unsaved changes
            let has_unsaved = state.tabs.iter().any(|t| {
                t.editor.as_ref().is_some_and(|e| e.modified)
                    || t.body_editor.as_ref().is_some_and(|e| e.modified)
                    || t.decl_editor.as_ref().is_some_and(|e| e.modified)
            });
            if has_unsaved {
                state.overlay = Some(Overlay::ConfirmQuit);
                return Some(Action::Render);
            }
            Some(Action::Quit)
        }
        KeyCode::Char('?') => {
            state.overlay = Some(Overlay::Help);
            Some(Action::Render)
        }
        KeyCode::Char('a') if state.focus == Focus::Sidebar => {
            let groups = state.available_groups();
            // Default group = the group the cursor is currently in
            let current_group = state
                .selected_tree_index()
                .and_then(|idx| {
                    let mut i = idx;
                    loop {
                        if let TreeNode::Group { name, .. } = &state.tree[i] {
                            return Some(name.clone());
                        }
                        if i == 0 {
                            break;
                        }
                        i -= 1;
                    }
                    None
                })
                .unwrap_or_else(|| "Default".to_string());
            state.connection_form = crate::ui::state::ConnectionFormState::new();
            state.connection_form.group = current_group;
            state.connection_form.group_options = groups;
            state.overlay = Some(Overlay::ConnectionDialog);
            Some(Action::Render)
        }
        KeyCode::Char('n') if state.focus == Focus::ScriptsPanel => Some(Action::OpenNewScript),
        KeyCode::Char('F') => Some(handle_filter_key(state)),
        KeyCode::Char(']') => {
            // Next tab
            if !state.tabs.is_empty() {
                state.active_tab_idx = (state.active_tab_idx + 1) % state.tabs.len();
                state.focus = Focus::TabContent;
            }
            Some(Action::Render)
        }
        KeyCode::Char('[') => {
            // Previous tab
            if !state.tabs.is_empty() {
                state.active_tab_idx = if state.active_tab_idx == 0 {
                    state.tabs.len() - 1
                } else {
                    state.active_tab_idx - 1
                };
                state.focus = Focus::TabContent;
            }
            Some(Action::Render)
        }
        KeyCode::Char('}') => {
            // If grid focused in script with result tabs, switch result tab
            if let Some(tab) = state.active_tab()
                && tab.grid_focused
                && tab.result_tabs.len() > 1
            {
                let tab = state.active_tab_mut().expect("checked");
                sync_grid_to_result_tab(tab);
                tab.active_result_idx = (tab.active_result_idx + 1) % tab.result_tabs.len();
                return Some(Action::Render);
            }
            // Otherwise, next sub-view
            if let Some(tab) = state.active_tab_mut() {
                tab.next_sub_view();
            }
            Some(Action::Render)
        }
        KeyCode::Char('{') => {
            if let Some(tab) = state.active_tab()
                && tab.grid_focused
                && tab.result_tabs.len() > 1
            {
                let tab = state.active_tab_mut().expect("checked");
                sync_grid_to_result_tab(tab);
                tab.active_result_idx = if tab.active_result_idx == 0 {
                    tab.result_tabs.len() - 1
                } else {
                    tab.active_result_idx - 1
                };
                return Some(Action::Render);
            }
            if let Some(tab) = state.active_tab_mut() {
                tab.prev_sub_view();
            }
            Some(Action::Render)
        }
        _ => None,
    }
}

/// Handle spatial focus switching with Ctrl+hjkl/arrows.
/// Layout:  Explorer | Script
///          Scripts  | Error | Query
/// Returns Some(Action) if the key was consumed.
fn handle_spatial_navigation(
    state: &mut AppState,
    key: KeyEvent,
    in_editor_special_mode: bool,
) -> Option<Action> {
    if !key.modifiers.contains(KeyModifiers::CONTROL) || in_editor_special_mode {
        return None;
    }

    use crate::ui::tabs::SubFocus;

    let sub = state
        .active_tab()
        .map(|t| t.sub_focus)
        .unwrap_or(SubFocus::Editor);
    let has_tabs = !state.tabs.is_empty();

    match key.code {
        KeyCode::Char('h') | KeyCode::Left => {
            match (state.focus, sub) {
                // Script -> Explorer
                (Focus::TabContent, SubFocus::Editor) => state.focus = Focus::Sidebar,
                // Error -> Scripts panel
                (Focus::TabContent, SubFocus::Results) => state.focus = Focus::ScriptsPanel,
                // Query -> Error
                (Focus::TabContent, SubFocus::QueryView) => {
                    if let Some(tab) = state.active_tab_mut() {
                        tab.sub_focus = SubFocus::Results;
                    }
                }
                _ => {}
            }
            Some(Action::Render)
        }
        KeyCode::Char('l') | KeyCode::Right => {
            match (state.focus, sub) {
                // Explorer -> Script
                (Focus::Sidebar, _) if has_tabs => {
                    state.focus = Focus::TabContent;
                    if let Some(tab) = state.active_tab_mut() {
                        tab.sub_focus = SubFocus::Editor;
                        tab.grid_focused = false;
                    }
                }
                // Scripts panel -> Error (if results exist)
                (Focus::ScriptsPanel, _) if has_tabs => {
                    let has_bottom = state
                        .active_tab()
                        .is_some_and(|t| !t.result_tabs.is_empty() || t.query_result.is_some());
                    if has_bottom {
                        state.focus = Focus::TabContent;
                        if let Some(tab) = state.active_tab_mut() {
                            tab.sub_focus = SubFocus::Results;
                            tab.grid_focused = true;
                        }
                    } else {
                        state.focus = Focus::TabContent;
                    }
                }
                // Error -> Query
                (Focus::TabContent, SubFocus::Results) => {
                    let has_query = state.active_tab().is_some_and(|t| {
                        let idx = t.active_result_idx;
                        idx < t.result_tabs.len() && t.result_tabs[idx].query_editor.is_some()
                    });
                    if has_query && let Some(tab) = state.active_tab_mut() {
                        tab.sub_focus = SubFocus::QueryView;
                    }
                }
                _ => {}
            }
            Some(Action::Render)
        }
        KeyCode::Char('j') | KeyCode::Down => {
            match (state.focus, sub) {
                // Explorer -> Scripts panel
                (Focus::Sidebar, _) => state.focus = Focus::ScriptsPanel,
                // Script -> Error/Results
                (Focus::TabContent, SubFocus::Editor) => {
                    let has_bottom = state
                        .active_tab()
                        .is_some_and(|t| !t.result_tabs.is_empty() || t.query_result.is_some());
                    if has_bottom && let Some(tab) = state.active_tab_mut() {
                        tab.sub_focus = SubFocus::Results;
                        tab.grid_focused = true;
                    }
                }
                _ => {}
            }
            Some(Action::Render)
        }
        KeyCode::Char('k') | KeyCode::Up => {
            match (state.focus, sub) {
                // Scripts panel -> Explorer
                (Focus::ScriptsPanel, _) => state.focus = Focus::Sidebar,
                // Error -> Script
                (Focus::TabContent, SubFocus::Results) => {
                    if let Some(tab) = state.active_tab_mut() {
                        tab.sub_focus = SubFocus::Editor;
                        tab.grid_focused = false;
                    }
                }
                // Query -> Script
                (Focus::TabContent, SubFocus::QueryView) => {
                    if let Some(tab) = state.active_tab_mut() {
                        tab.sub_focus = SubFocus::Editor;
                        tab.grid_focused = false;
                    }
                }
                _ => {}
            }
            Some(Action::Render)
        }
        _ => None,
    }
}

/// Check whether the sub-editor is in a state that allows exiting the sub-pane on Escape.
/// Returns true if the sub-editor is in Normal mode (not searching), or it is a data grid
/// not in visual mode -- meaning Escape should move focus back to the main editor.
fn should_exit_sub_pane(tab: &WorkspaceTab, sub_focus: crate::ui::tabs::SubFocus) -> bool {
    use crate::ui::tabs::SubFocus;

    let idx = tab.active_result_idx;
    match sub_focus {
        SubFocus::Results => {
            if idx < tab.result_tabs.len() {
                if let Some(editor) = &tab.result_tabs[idx].error_editor {
                    matches!(editor.mode, vimltui::VimMode::Normal) && !editor.search.active
                } else {
                    // Data grid: check visual mode
                    !tab.grid_visual_mode
                }
            } else {
                !tab.grid_visual_mode
            }
        }
        SubFocus::QueryView => {
            if idx < tab.result_tabs.len() {
                if let Some(editor) = &tab.result_tabs[idx].query_editor {
                    matches!(editor.mode, vimltui::VimMode::Normal) && !editor.search.active
                } else {
                    true
                }
            } else {
                true
            }
        }
        _ => true,
    }
}

// --- Tab Content Dispatch ---

fn handle_tab_content(state: &mut AppState, key: KeyEvent) -> Action {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return Action::None;
    }

    let sub_view = state.tabs[tab_idx].active_sub_view.clone();

    // Leader keys are handled globally in handle_key() before reaching here

    match sub_view {
        Some(SubView::TableData) => handle_tab_data_grid(state, key),
        Some(SubView::TableProperties) => {
            // Properties is read-only, no special keys
            Action::None
        }
        Some(SubView::TableDDL) => handle_tab_editor(state, key),
        Some(SubView::PackageBody) | Some(SubView::PackageDeclaration) => {
            handle_tab_editor(state, key)
        }
        Some(SubView::PackageFunctions) | Some(SubView::PackageProcedures) => {
            handle_tab_package_list(state, key)
        }
        None => {
            // Script / Function / Procedure
            // Ctrl+hjkl navigation is handled globally above.
            use crate::ui::tabs::SubFocus;

            let tab = &state.tabs[state.active_tab_idx];
            let has_bottom = tab.query_result.is_some() || !tab.result_tabs.is_empty();
            let sub_focus = tab.sub_focus;

            // For error/query vim editors: only exit pane on Escape if editor is in Normal mode
            // (let Escape pass through to the vim editor first to exit visual/search)
            // For data grid Results: Escape exits visual mode first, then exits pane
            if (sub_focus == SubFocus::Results || sub_focus == SubFocus::QueryView)
                && key.code == KeyCode::Esc
                && should_exit_sub_pane(&state.tabs[state.active_tab_idx], sub_focus)
            {
                let tab = &mut state.tabs[state.active_tab_idx];
                tab.sub_focus = SubFocus::Editor;
                tab.grid_focused = false;
                return Action::Render;
            }
            // Otherwise fall through to let the sub-editor handle Escape

            match sub_focus {
                SubFocus::Editor if !has_bottom => handle_tab_editor(state, key),
                SubFocus::Editor => handle_tab_editor(state, key),
                SubFocus::Results => {
                    // Leader keys handled globally. Error editor?
                    let has_error = {
                        let tab = &state.tabs[state.active_tab_idx];
                        let idx = tab.active_result_idx;
                        idx < tab.result_tabs.len() && tab.result_tabs[idx].error_editor.is_some()
                    };
                    if has_error {
                        let tab = &mut state.tabs[state.active_tab_idx];
                        let idx = tab.active_result_idx;
                        if let Some(editor) = tab.result_tabs[idx].error_editor.as_mut() {
                            editor.handle_key(key);
                        }
                        return Action::Render;
                    }

                    handle_tab_data_grid(state, key)
                }
                SubFocus::QueryView => {
                    // Leader keys handled globally.
                    let tab = &mut state.tabs[state.active_tab_idx];
                    let idx = tab.active_result_idx;
                    if idx < tab.result_tabs.len()
                        && let Some(editor) = tab.result_tabs[idx].query_editor.as_mut()
                    {
                        editor.handle_key(key);
                    }
                    Action::Render
                }
            }
        }
    }
}

fn handle_tab_editor(state: &mut AppState, key: KeyEvent) -> Action {
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
                        // Re-trigger completion (e.g., after alias. → show columns)
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
                // Re-trigger completion (e.g., after alias. → show columns)
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
            // Only run diagnostics on Insert→Normal transition and if metadata is loaded
            let needs_diag = !still_insert && in_insert && editor.modified && state.metadata_ready;
            (action, still_insert, needs_diag)
        } else {
            return Action::None;
        }
    };
    // tab/editor borrows are dropped here

    if still_insert {
        // Auto-update completion while typing in Insert mode
        if let Some(cache_action) = update_completion(state) {
            return cache_action;
        }
    } else {
        state.completion = None;
    }

    if needs_diag {
        let lines = state.tabs[tab_idx]
            .active_editor()
            .map(|e| e.lines.clone())
            .unwrap_or_default();
        state.diagnostics = crate::ui::diagnostics::check_sql(state, &lines);
    }

    action
}

/// Update completion popup (auto-trigger, requires prefix).
fn update_completion(state: &mut AppState) -> Option<Action> {
    update_completion_impl(state, false)
}

/// Update completion popup. `force=true` opens even without prefix (Ctrl+Space).
fn update_completion_impl(state: &mut AppState, force: bool) -> Option<Action> {
    use crate::ui::completion::{
        CompletionState, build_completions, build_completions_forced, is_after_dot,
        word_prefix_at_cursor,
    };

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
    let (prefix, start_col) = word_prefix_at_cursor(line, col);
    let dot_mode = prefix.is_empty() && is_after_dot(line, col);

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
                            let diff =
                                exact[0].label.len() as isize - cmp.prefix.len() as isize;
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
    let editor_row = row; // preserve real editor row for origin_row
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
    let row = row - block_start;
    let items = if force {
        build_completions_forced(state, &lines, row, col)
    } else {
        build_completions(state, &lines, row, col)
    };

    // If no column completions found and cursor is in a dot context,
    // trigger on-demand column loading
    let has_dot = dot_mode || {
        // Check if there's a dot between prefix start and the identifier before it
        let before = &lines[row][..col.min(lines[row].len())];
        let bytes = before.as_bytes();
        let mut p = col.min(bytes.len());
        while p > 0 && (bytes[p - 1].is_ascii_alphanumeric() || bytes[p - 1] == b'_') {
            p -= 1;
        }
        p > 0 && bytes[p - 1] == b'.'
    };

    let cache_action = if items.is_empty() && has_dot {
        // Extract identifier before the dot
        let before = &lines[row][..col.min(lines[row].len())];
        let bytes = before.as_bytes();
        let mut p = col.min(bytes.len());
        // Skip prefix
        while p > 0 && (bytes[p - 1].is_ascii_alphanumeric() || bytes[p - 1] == b'_') {
            p -= 1;
        }
        // Skip dot
        if p > 0 && bytes[p - 1] == b'.' {
            p -= 1;
        }
        // Extract table ref before dot
        let te = p;
        let mut ts = te;
        while ts > 0 && (bytes[ts - 1].is_ascii_alphanumeric() || bytes[ts - 1] == b'_') {
            ts -= 1;
        }
        let table_ref = &before[ts..te];

        if !table_ref.is_empty() && !crate::ui::completion::is_known_schema(state, table_ref) {
            let resolved = resolve_table_for_cache(state, &lines, row, table_ref);
            if let Some((schema, table)) = resolved {
                let key = format!("{}.{}", schema.to_uppercase(), table.to_uppercase());
                if !state.column_cache.contains_key(&key) {
                    Some(Action::CacheColumns { schema, table })
                } else {
                    None
                }
            } else {
                None
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
        prefix: prefix.to_string(),
        origin_row: editor_row,
        origin_col: start_col,
    });

    cache_action
}

/// Resolve a table reference to (schema, table) for column cache loading.
fn resolve_table_for_cache(
    state: &AppState,
    lines: &[String],
    _row: usize,
    table_ref: &str,
) -> Option<(String, String)> {
    use crate::ui::completion::find_schema_for_table;

    // Scope to query block
    let block: Vec<String> = lines.to_vec(); // already scoped by build_completions

    // Try to resolve alias
    let resolved = crate::ui::completion::resolve_table_name(&block, table_ref);
    let table_name = resolved.as_deref().unwrap_or(table_ref);

    // Find schema in tree
    let schema = find_schema_for_table(state, table_name)?;
    Some((schema, table_name.to_string()))
}

/// Accept the selected completion item: replace prefix with completion text.
fn accept_completion(state: &mut AppState, cmp: &crate::ui::completion::CompletionState) {
    use crate::ui::completion::CompletionKind;

    let item = match cmp.selected() {
        Some(s) => s.clone(),
        None => return,
    };

    // Append "." for alias/schema, "()" for functions/procedures and keywords that need parens
    let needs_parens = match item.kind {
        CompletionKind::Function | CompletionKind::Procedure => true,
        CompletionKind::Keyword => matches!(
            item.label.as_str(),
            "IN" | "EXISTS" | "NOT IN"
        ),
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

fn handle_tab_data_grid(state: &mut AppState, key: KeyEvent) -> Action {
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

        // Handle result tab switching with { and }
        match key.code {
            KeyCode::Char('}') => {
                if tab.result_tabs.len() > 1 {
                    sync_grid_to_result_tab(tab);
                    tab.active_result_idx = (tab.active_result_idx + 1) % tab.result_tabs.len();
                }
                return Action::Render;
            }
            KeyCode::Char('{') => {
                if tab.result_tabs.len() > 1 {
                    sync_grid_to_result_tab(tab);
                    tab.active_result_idx = if tab.active_result_idx == 0 {
                        tab.result_tabs.len() - 1
                    } else {
                        tab.active_result_idx - 1
                    };
                }
                return Action::Render;
            }
            _ => {}
        }
    }

    let tab = &mut state.tabs[tab_idx];
    let row_count = tab.query_result.as_ref().map(|r| r.rows.len()).unwrap_or(0);
    let col_count = tab
        .query_result
        .as_ref()
        .map(|r| r.columns.len())
        .unwrap_or(0);
    let vh = tab.grid_visible_height.max(1);
    let visual = tab.grid_visual_mode;

    let action = match key.code {
        // --- Toggle visual mode ---
        KeyCode::Char('v') => {
            if visual {
                // Exit visual
                tab.grid_visual_mode = false;
                tab.grid_selection_anchor = None;
            } else {
                // Enter visual, anchor at current cell
                tab.grid_visual_mode = true;
                tab.grid_selection_anchor = Some((tab.grid_selected_row, tab.grid_selected_col));
            }
            Action::Render
        }
        // --- Movement (extends selection in visual mode) ---
        KeyCode::Char('j') | KeyCode::Down => {
            if tab.grid_selected_row + 1 < row_count {
                tab.grid_selected_row += 1;
                if tab.grid_selected_row >= tab.grid_scroll_row + vh {
                    tab.grid_scroll_row = tab.grid_selected_row - vh + 1;
                }
            }
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if tab.grid_selected_row > 0 {
                tab.grid_selected_row -= 1;
                if tab.grid_selected_row < tab.grid_scroll_row {
                    tab.grid_scroll_row = tab.grid_selected_row;
                }
            }
            Action::Render
        }
        KeyCode::Char('h') | KeyCode::Left => {
            if tab.grid_selected_col > 0 {
                tab.grid_selected_col -= 1;
            }
            Action::Render
        }
        KeyCode::Char('l') | KeyCode::Right => {
            if col_count > 0 && tab.grid_selected_col + 1 < col_count {
                tab.grid_selected_col += 1;
            }
            Action::Render
        }
        // --- Next/prev cell (e/b) wrapping across rows ---
        KeyCode::Char('e') => {
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
        KeyCode::Char('b') => {
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
        // --- Half-page scroll ---
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = vh / 2;
            tab.grid_selected_row = (tab.grid_selected_row + half).min(row_count.saturating_sub(1));
            tab.grid_scroll_row = tab.grid_selected_row.saturating_sub(vh / 2);
            Action::Render
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = vh / 2;
            tab.grid_selected_row = tab.grid_selected_row.saturating_sub(half);
            tab.grid_scroll_row = tab.grid_selected_row.saturating_sub(vh / 2);
            Action::Render
        }
        // --- Jump to top/bottom ---
        KeyCode::Char('g') => {
            tab.grid_selected_row = 0;
            tab.grid_selected_col = 0;
            tab.grid_scroll_row = 0;
            Action::Render
        }
        KeyCode::Char('G') => {
            if row_count > 0 {
                tab.grid_selected_row = row_count - 1;
                tab.grid_scroll_row = row_count.saturating_sub(vh);
            }
            Action::Render
        }
        // --- Copy ---
        KeyCode::Char('y') => {
            grid_yank(tab);
            // Exit visual mode after yank
            tab.grid_visual_mode = false;
            tab.grid_selection_anchor = None;
            Action::Render
        }
        // --- Escape: exit visual or exit grid ---
        KeyCode::Esc => {
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

    if matches!(key.code, KeyCode::Char('y')) {
        state.status_message = "Copied to clipboard".to_string();
    }

    // Sync grid state back to the active result tab for scripts
    if is_script {
        let tab = &mut state.tabs[tab_idx];
        sync_grid_to_result_tab(tab);
    }

    action
}

fn sync_grid_to_result_tab(tab: &mut WorkspaceTab) {
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
fn grid_yank(tab: &WorkspaceTab) {
    let result = match &tab.query_result {
        Some(r) => r,
        None => return,
    };
    if result.rows.is_empty() {
        return;
    }

    let (sr, sc, er, ec) = match tab.grid_selection_anchor {
        Some((ar, ac)) => {
            let r1 = ar.min(tab.grid_selected_row);
            let r2 = ar.max(tab.grid_selected_row);
            let c1 = ac.min(tab.grid_selected_col);
            let c2 = ac.max(tab.grid_selected_col);
            (r1, c1, r2, c2)
        }
        None => {
            // No selection: copy entire row
            let col_count = result.columns.len().saturating_sub(1);
            (tab.grid_selected_row, 0, tab.grid_selected_row, col_count)
        }
    };

    let mut text = String::new();
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

    if !text.is_empty() {
        copy_to_clipboard(&text);
    }
}

/// Copy text to system clipboard (reusable, not tied to VimEditor)
fn copy_to_clipboard(text: &str) {
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
                use std::io::Write;
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
            return;
        }
    }
}

fn handle_tab_package_list(state: &mut AppState, key: KeyEvent) -> Action {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return Action::None;
    }

    let tab = &mut state.tabs[tab_idx];
    let list_len = match &tab.active_sub_view {
        Some(SubView::PackageFunctions) => tab.package_functions.len(),
        Some(SubView::PackageProcedures) => tab.package_procedures.len(),
        _ => 0,
    };

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if list_len > 0 && tab.package_list_cursor + 1 < list_len {
                tab.package_list_cursor += 1;
            }
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if tab.package_list_cursor > 0 {
                tab.package_list_cursor -= 1;
            }
            Action::Render
        }
        KeyCode::Char('g') => {
            tab.package_list_cursor = 0;
            Action::Render
        }
        KeyCode::Char('G') => {
            if list_len > 0 {
                tab.package_list_cursor = list_len - 1;
            }
            Action::Render
        }
        KeyCode::Enter | KeyCode::Char('l') => {
            // Jump to declaration and search for the selected function/procedure
            let selected_name = match &tab.active_sub_view {
                Some(SubView::PackageFunctions) => {
                    tab.package_functions.get(tab.package_list_cursor).cloned()
                }
                Some(SubView::PackageProcedures) => {
                    tab.package_procedures.get(tab.package_list_cursor).cloned()
                }
                _ => None,
            };
            if let Some(name) = selected_name {
                // Switch to Declaration view and search for the name
                tab.active_sub_view = Some(SubView::PackageDeclaration);
                if let Some(editor) = tab.decl_editor.as_mut() {
                    editor.search.pattern = name;
                    editor.search.forward = true;
                    editor.cursor_row = 0;
                    editor.cursor_col = 0;
                    editor.jump_to_next_match();
                }
            }
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Leader key for non-editor views ---

/// Resolve a leader sub-menu: clear pending flags, check if the key matches
/// the expected char, and return the action if so (or Render otherwise).
fn resolve_leader_submenu(
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
fn handle_global_leader(state: &mut AppState, key: KeyEvent) -> Option<Action> {
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

// --- Confirm Close Overlay ---

fn handle_confirm_close(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('y') => {
            state.overlay = None;
            Action::ConfirmCloseYes
        }
        KeyCode::Char('n') => {
            state.overlay = None;
            Action::ConfirmCloseNo
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            state.overlay = None;
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Confirm Quit Overlay ---

fn handle_confirm_quit(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            state.overlay = None;
            Action::Quit
        }
        KeyCode::Esc | KeyCode::Char('n') => {
            state.overlay = None;
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Save Script Name Prompt ---

fn handle_save_script_name(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.scripts_save_name = None;
            state.overlay = None;
            Action::Render
        }
        KeyCode::Enter => {
            if let Some(name) = state.scripts_save_name.take() {
                state.overlay = None;
                if !name.is_empty() {
                    return Action::SaveScriptAs { name };
                }
            }
            Action::Render
        }
        KeyCode::Backspace => {
            if let Some(ref mut buf) = state.scripts_save_name {
                buf.pop();
            }
            Action::Render
        }
        KeyCode::Char(c) => {
            if let Some(ref mut buf) = state.scripts_save_name {
                buf.push(c);
            }
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Scripts Panel ---

fn handle_scripts_panel(state: &mut AppState, key: KeyEvent) -> Action {
    // Delete confirmation mode
    if state.scripts_confirm_delete.is_some() {
        return handle_scripts_confirm_delete(state, key);
    }

    // Rename mode: capture text input
    if state.scripts_renaming.is_some() {
        return handle_scripts_rename(state, key);
    }

    let count = state.scripts_list.len();

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if count > 0 && state.scripts_cursor + 1 < count {
                state.scripts_cursor += 1;
            }
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if state.scripts_cursor > 0 {
                state.scripts_cursor -= 1;
            }
            Action::Render
        }
        KeyCode::Char('g') => {
            state.scripts_cursor = 0;
            state.scripts_offset = 0;
            Action::Render
        }
        KeyCode::Char('G') => {
            if count > 0 {
                state.scripts_cursor = count - 1;
            }
            Action::Render
        }
        KeyCode::Enter | KeyCode::Char('l') => {
            // Open the selected script
            if let Some(name) = state.scripts_list.get(state.scripts_cursor).cloned() {
                let script_name = name.strip_suffix(".sql").unwrap_or(&name).to_string();
                Action::OpenScript { name: script_name }
            } else {
                Action::None
            }
        }
        KeyCode::Char('d') => {
            // Ask for delete confirmation
            if let Some(name) = state.scripts_list.get(state.scripts_cursor).cloned() {
                state.scripts_confirm_delete = Some(name);
            }
            Action::Render
        }
        KeyCode::Char('D') => {
            // Duplicate selected script
            if let Some(name) = state.scripts_list.get(state.scripts_cursor).cloned() {
                Action::DuplicateScript { name }
            } else {
                Action::None
            }
        }
        KeyCode::Char('r') => {
            // Start rename
            if let Some(name) = state.scripts_list.get(state.scripts_cursor).cloned() {
                let display_name = name.strip_suffix(".sql").unwrap_or(&name).to_string();
                state.scripts_rename_buf = display_name.clone();
                state.scripts_renaming = Some(name);
            }
            Action::Render
        }
        KeyCode::Char('n') => Action::OpenNewScript,
        _ => Action::None,
    }
}

fn handle_scripts_confirm_delete(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            if let Some(name) = state.scripts_confirm_delete.take() {
                return Action::DeleteScript { name };
            }
            Action::Render
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            state.scripts_confirm_delete = None;
            Action::Render
        }
        _ => Action::None,
    }
}

fn handle_scripts_rename(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.scripts_renaming = None;
            state.scripts_rename_buf.clear();
            Action::Render
        }
        KeyCode::Enter => {
            let new_name = state.scripts_rename_buf.clone();
            if let Some(old_name) = state.scripts_renaming.take() {
                state.scripts_rename_buf.clear();
                if !new_name.is_empty() {
                    return Action::RenameScript { old_name, new_name };
                }
            }
            Action::Render
        }
        KeyCode::Backspace => {
            state.scripts_rename_buf.pop();
            Action::Render
        }
        KeyCode::Char(c) => {
            state.scripts_rename_buf.push(c);
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Group Rename ---

fn handle_group_rename(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.group_renaming = None;
            state.group_rename_buf.clear();
            Action::Render
        }
        KeyCode::Enter => {
            let new_name = state.group_rename_buf.trim().to_string();
            if let Some(old_name) = state.group_renaming.take() {
                state.group_rename_buf.clear();
                if !new_name.is_empty() && new_name != old_name {
                    // Rename in tree
                    for node in &mut state.tree {
                        if let TreeNode::Group { name, .. } = node
                            && *name == old_name
                        {
                            *name = new_name.clone();
                        }
                    }
                    // Rename in saved connections
                    for conn in &mut state.saved_connections {
                        if conn.group == old_name {
                            conn.group = new_name.clone();
                        }
                    }
                    // Persist connections and groups
                    if let Ok(store) = crate::core::storage::ConnectionStore::new() {
                        let _ = store.save(&state.saved_connections, "");
                        let _ = store.save_groups(&persist_group_names(state));
                    }
                    state.status_message = format!("Group renamed: '{old_name}' → '{new_name}'");
                }
            }
            Action::Render
        }
        KeyCode::Backspace => {
            state.group_rename_buf.pop();
            Action::Render
        }
        KeyCode::Char(c) => {
            state.group_rename_buf.push(c);
            Action::Render
        }
        _ => Action::None,
    }
}

fn handle_group_create(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.group_creating = false;
            state.group_rename_buf.clear();
            Action::Render
        }
        KeyCode::Enter => {
            let name = state.group_rename_buf.trim().to_string();
            state.group_creating = false;
            state.group_rename_buf.clear();
            if !name.is_empty() {
                // Check if group already exists
                let exists = state.tree.iter().any(
                    |n| matches!(n, TreeNode::Group { name: gn, .. } if gn == &name),
                );
                if exists {
                    state.status_message = format!("Group '{name}' already exists");
                } else {
                    // Insert new empty group at the end of the tree
                    state.tree.push(TreeNode::Group {
                        name: name.clone(),
                        expanded: true,
                    });
                    // Persist groups so empty groups survive restart
                    if let Ok(store) = crate::core::storage::ConnectionStore::new() {
                        let _ = store.save_groups(&persist_group_names(state));
                    }
                    state.status_message = format!("Group '{name}' created");
                }
            }
            Action::Render
        }
        KeyCode::Backspace => {
            state.group_rename_buf.pop();
            Action::Render
        }
        KeyCode::Char(c) => {
            state.group_rename_buf.push(c);
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Filter Key ---

fn handle_filter_key(state: &mut AppState) -> Action {
    if let Some(idx) = state.selected_tree_index() {
        // Prefix filter keys with connection name so each connection has independent filters
        let conn_prefix = state.connection_for_tree_idx(idx).unwrap_or("").to_string();

        match &state.tree[idx] {
            TreeNode::Group { .. } => {}
            TreeNode::Connection { .. } | TreeNode::Schema { .. } => {
                let schemas = state.schema_names_for_conn(&conn_prefix);
                if !schemas.is_empty() {
                    let key = format!("{conn_prefix}::schemas");
                    state.object_filter.open_for(&key, schemas);
                    state.overlay = Some(Overlay::ObjectFilter);
                }
            }
            TreeNode::Category { schema, kind, .. } => {
                let base_key = kind.filter_key(schema);
                let key = format!("{conn_prefix}::{base_key}");
                let items = state.leaves_under_category(idx);
                if !items.is_empty() {
                    state.object_filter.open_for(&key, items);
                    state.overlay = Some(Overlay::ObjectFilter);
                }
            }
            TreeNode::Leaf { schema, kind, .. } => {
                let base_key = match kind {
                    LeafKind::Table => format!("{schema}.Tables"),
                    LeafKind::View => format!("{schema}.Views"),
                    LeafKind::Package => format!("{schema}.Packages"),
                    LeafKind::Procedure => format!("{schema}.Procedures"),
                    LeafKind::Function => format!("{schema}.Functions"),
                };
                let cat_key = format!("{conn_prefix}::{base_key}");
                let mut walk = idx;
                while walk > 0 {
                    walk -= 1;
                    if matches!(&state.tree[walk], TreeNode::Category { .. }) {
                        let items = state.leaves_under_category(walk);
                        if !items.is_empty() {
                            state.object_filter.open_for(&cat_key, items);
                            state.overlay = Some(Overlay::ObjectFilter);
                        }
                        break;
                    }
                }
            }
        }
    } else if !state.tree.is_empty() {
        let schemas = state.all_schema_names();
        if !schemas.is_empty() {
            state.object_filter.open_for("schemas", schemas);
            state.overlay = Some(Overlay::ObjectFilter);
        }
    }
    Action::Render
}

// --- Schema Filter ---

fn handle_object_filter(state: &mut AppState, key: KeyEvent) -> Action {
    if state.object_filter.search_active {
        return handle_object_filter_search(state, key);
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            state.overlay = None;
            Action::SaveSchemaFilter
        }
        KeyCode::Char('j') | KeyCode::Down => {
            state.object_filter.move_down();
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.object_filter.move_up();
            Action::Render
        }
        KeyCode::Char('g') => {
            state.object_filter.go_top();
            Action::Render
        }
        KeyCode::Char('G') => {
            state.object_filter.go_bottom();
            Action::Render
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = state.object_filter.visible_height / 2;
            let count = state.object_filter.display_list().len();
            state.object_filter.cursor =
                (state.object_filter.cursor + half).min(count.saturating_sub(1));
            state.object_filter.offset = state
                .object_filter
                .cursor
                .saturating_sub(state.object_filter.visible_height / 2);
            Action::Render
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = state.object_filter.visible_height / 2;
            state.object_filter.cursor = state.object_filter.cursor.saturating_sub(half);
            state.object_filter.offset = state
                .object_filter
                .cursor
                .saturating_sub(state.object_filter.visible_height / 2);
            Action::Render
        }
        KeyCode::Char(' ') => {
            state.object_filter.toggle_at_cursor();
            Action::SaveSchemaFilter
        }
        KeyCode::Char('a') => {
            state.object_filter.select_all();
            Action::SaveSchemaFilter
        }
        KeyCode::Char('/') => {
            state.object_filter.search_active = true;
            state.object_filter.search_query.clear();
            state.object_filter.cursor = 0;
            state.object_filter.offset = 0;
            Action::Render
        }
        KeyCode::Enter => {
            state.overlay = None;
            Action::SaveSchemaFilter
        }
        _ => Action::None,
    }
}

fn handle_object_filter_search(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.object_filter.search_active = false;
            state.object_filter.search_query.clear();
            state.object_filter.cursor = 0;
            state.object_filter.offset = 0;
            Action::Render
        }
        KeyCode::Enter => {
            state.object_filter.search_active = false;
            Action::Render
        }
        KeyCode::Backspace => {
            state.object_filter.search_query.pop();
            state.object_filter.cursor = 0;
            state.object_filter.offset = 0;
            Action::Render
        }
        KeyCode::Char(c) => {
            state.object_filter.search_query.push(c);
            state.object_filter.cursor = 0;
            state.object_filter.offset = 0;
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Sidebar Search ---

fn handle_sidebar_search(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.tree_state.search_active = false;
            state.tree_state.search_query.clear();
            state.tree_state.search_matches.clear();
            Action::Render
        }
        KeyCode::Enter => {
            state.tree_state.search_active = false;
            Action::Render
        }
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let count = state.visible_tree().len();
            state.tree_state.next_match(count);
            Action::Render
        }
        KeyCode::Backspace => {
            state.tree_state.search_query.pop();
            update_search_and_jump(state);
            Action::Render
        }
        KeyCode::Char(c) => {
            state.tree_state.search_query.push(c);
            update_search_and_jump(state);
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Connection Dialog ---

fn handle_connection_dialog(state: &mut AppState, key: KeyEvent) -> Action {
    if state.connection_form.show_saved_list {
        return handle_saved_connections_list(state, key);
    }

    if state.connection_form.read_only {
        return match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                state.overlay = None;
                Action::Render
            }
            _ => Action::None,
        };
    }

    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            Action::Render
        }
        KeyCode::Tab => {
            state.connection_form.next_field();
            Action::Render
        }
        KeyCode::BackTab => {
            state.connection_form.prev_field();
            Action::Render
        }
        KeyCode::Enter => {
            if state.connection_form.name.is_empty() {
                state.connection_form.error_message = "Name is required".to_string();
                return Action::Render;
            }
            if state.connection_form.host.is_empty() {
                state.connection_form.error_message = "Host is required".to_string();
                return Action::Render;
            }
            if state.connection_form.username.is_empty() {
                state.connection_form.error_message = "Username is required".to_string();
                return Action::Render;
            }
            state.connection_form.error_message.clear();
            state.connection_form.connecting = true;
            Action::Connect
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.connection_form.password_visible = !state.connection_form.password_visible;
            Action::Render
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.connection_form.cycle_db_type();
            Action::Render
        }
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.connection_form.cycle_group();
            Action::Render
        }
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Action::SaveConnection
        }
        KeyCode::Char(c) => {
            // Fields 1 (Type) and 7 (Group) are selectors, not text input
            if state.connection_form.selected_field == 1
                || state.connection_form.selected_field == 7
            {
                return Action::None;
            }
            state.connection_form.active_field_mut().push(c);
            state.connection_form.error_message.clear();
            Action::Render
        }
        KeyCode::Backspace => {
            if state.connection_form.selected_field != 1
                && state.connection_form.selected_field != 7
            {
                state.connection_form.active_field_mut().pop();
            }
            Action::Render
        }
        _ => Action::None,
    }
}

fn handle_saved_connections_list(state: &mut AppState, key: KeyEvent) -> Action {
    let count = state.saved_connections.len();
    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            Action::Render
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if count > 0 {
                state.connection_form.saved_cursor =
                    (state.connection_form.saved_cursor + 1) % (count + 1);
            }
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if state.connection_form.saved_cursor == 0 {
                state.connection_form.saved_cursor = count;
            } else {
                state.connection_form.saved_cursor -= 1;
            }
            Action::Render
        }
        KeyCode::Enter => {
            let cursor = state.connection_form.saved_cursor;
            if cursor < count {
                let config = state.saved_connections[cursor].clone();
                let groups = state.available_groups();
                state.connection_form = crate::ui::state::ConnectionFormState::from_config(&config);
                state.connection_form.group_options = groups;
                state.connection_form.connecting = true;
                Action::Connect
            } else {
                state.connection_form.show_saved_list = false;
                Action::Render
            }
        }
        KeyCode::Char('n') => {
            state.connection_form.show_saved_list = false;
            Action::Render
        }
        KeyCode::Char('d') => {
            let cursor = state.connection_form.saved_cursor;
            if cursor < count {
                let name = state.saved_connections[cursor].name.clone();
                state.saved_connections.remove(cursor);
                if let Ok(store) = crate::core::storage::ConnectionStore::new() {
                    let _ = store.save(&state.saved_connections, "");
                }
                state.status_message = format!("Connection '{name}' deleted");
                if state.connection_form.saved_cursor >= state.saved_connections.len()
                    && state.connection_form.saved_cursor > 0
                {
                    state.connection_form.saved_cursor -= 1;
                }
                if state.saved_connections.is_empty() {
                    state.connection_form.show_saved_list = false;
                }
            }
            Action::Render
        }
        _ => Action::None,
    }
}

fn handle_conn_menu(state: &mut AppState, key: KeyEvent) -> Action {
    use crate::ui::state::ConnMenuAction;

    let actions = ConnMenuAction::all();
    let count = actions.len();

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            state.overlay = None;
            Action::Render
        }
        KeyCode::Char('j') | KeyCode::Down => {
            state.conn_menu.cursor = (state.conn_menu.cursor + 1) % count;
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.conn_menu.cursor = if state.conn_menu.cursor == 0 {
                count - 1
            } else {
                state.conn_menu.cursor - 1
            };
            Action::Render
        }
        KeyCode::Enter => {
            let selected = &actions[state.conn_menu.cursor];
            let name = state.conn_menu.conn_name.clone();
            state.overlay = None;

            match selected {
                ConnMenuAction::View => {
                    if let Some(config) = state.saved_connections.iter().find(|c| c.name == name) {
                        let groups = state.available_groups();
                        let mut form = crate::ui::state::ConnectionFormState::from_config(config);
                        form.password = "********".to_string();
                        form.password_visible = false;
                        form.read_only = true;
                        form.group_options = groups;
                        state.connection_form = form;
                        state.overlay = Some(Overlay::ConnectionDialog);
                    }
                    Action::Render
                }
                ConnMenuAction::Edit => {
                    if let Some(config) = state.saved_connections.iter().find(|c| c.name == name) {
                        let groups = state.available_groups();
                        state.connection_form =
                            crate::ui::state::ConnectionFormState::for_edit(config);
                        state.connection_form.group_options = groups;
                        state.overlay = Some(Overlay::ConnectionDialog);
                    }
                    Action::Render
                }
                ConnMenuAction::Connect => Action::ConnectByName { name },
                ConnMenuAction::Disconnect => Action::DisconnectByName { name },
                ConnMenuAction::Restart => Action::ConnectByName { name },
                ConnMenuAction::Delete => Action::DeleteConnection { name },
            }
        }
        _ => Action::None,
    }
}

fn handle_group_menu(state: &mut AppState, key: KeyEvent) -> Action {
    use crate::ui::state::GroupMenuAction;

    let actions = GroupMenuAction::all();
    let count = actions.len();

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            state.overlay = None;
            Action::Render
        }
        KeyCode::Char('j') | KeyCode::Down => {
            state.group_menu.cursor = (state.group_menu.cursor + 1) % count;
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.group_menu.cursor = if state.group_menu.cursor == 0 {
                count - 1
            } else {
                state.group_menu.cursor - 1
            };
            Action::Render
        }
        KeyCode::Enter => {
            let selected_idx = state.group_menu.cursor;
            let group_name = state.group_menu.group_name.clone();
            let is_empty = state.group_menu.is_empty;
            state.overlay = None;

            match &actions[selected_idx] {
                GroupMenuAction::Rename => {
                    state.group_renaming = Some(group_name.clone());
                    state.group_rename_buf = group_name;
                    Action::Render
                }
                GroupMenuAction::Delete => {
                    if !is_empty {
                        state.status_message =
                            "Cannot delete group with connections".to_string();
                        return Action::Render;
                    }
                    // Remove the empty group node from tree
                    state
                        .tree
                        .retain(|n| !matches!(n, TreeNode::Group { name, .. } if name == &group_name));
                    state.status_message = format!("Group '{group_name}' deleted");
                    // Persist groups
                    if let Ok(store) = crate::core::storage::ConnectionStore::new() {
                        let _ = store.save_groups(&persist_group_names(state));
                    }
                    Action::Render
                }
                GroupMenuAction::NewGroup => {
                    state.group_creating = true;
                    state.group_rename_buf.clear();
                    Action::Render
                }
            }
        }
        _ => Action::None,
    }
}

fn handle_help_overlay(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
            state.overlay = None;
            Action::Render
        }
        _ => Action::None,
    }
}

fn update_search_and_jump(state: &mut AppState) {
    let query = state.tree_state.search_query.to_lowercase();
    let visible = state.visible_tree();
    let mut matches = Vec::new();
    for (vis_idx, (_, node)) in visible.iter().enumerate() {
        if !query.is_empty() && node.display_name().to_lowercase().contains(&query) {
            matches.push(vis_idx);
        }
    }
    let count = visible.len();
    drop(visible);

    state.tree_state.search_matches = matches;
    state.tree_state.search_match_idx = 0;
    if let Some(&first) = state.tree_state.search_matches.first() {
        state.tree_state.cursor = first;
        state.tree_state.center_scroll(count);
    }
}

// --- Sidebar (Neovim-like) ---

fn handle_sidebar(state: &mut AppState, key: KeyEvent) -> Action {
    // Group rename mode
    if state.group_renaming.is_some() {
        return handle_group_rename(state, key);
    }
    // Group create mode
    if state.group_creating {
        return handle_group_create(state, key);
    }

    let visible_count = state.visible_tree().len();
    if visible_count == 0 {
        return Action::None;
    }

    // Ctrl+d/u half-page
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('d') => {
                state.tree_state.half_page_down(visible_count);
                return Action::Render;
            }
            KeyCode::Char('u') => {
                state.tree_state.half_page_up(visible_count);
                return Action::Render;
            }
            _ => {}
        }
    }

    // Reset pending_d if any key other than 'd' is pressed
    if key.code != KeyCode::Char('d') {
        state.tree_state.pending_d = false;
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.tree_state.move_down(visible_count);
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.tree_state.move_up();
            Action::Render
        }
        KeyCode::Char('g') => {
            state.tree_state.go_top();
            Action::Render
        }
        KeyCode::Char('G') => {
            state.tree_state.go_bottom(visible_count);
            Action::Render
        }
        KeyCode::Char('l') | KeyCode::Enter => {
            if let Some(idx) = state.selected_tree_index() {
                handle_tree_action(state, idx)
            } else {
                Action::None
            }
        }
        KeyCode::Char('h') => {
            if let Some(idx) = state.selected_tree_index()
                && state.tree[idx].is_expanded()
            {
                state.tree[idx].toggle_expand();
            }
            Action::Render
        }
        KeyCode::Char('d') => {
            if state.tree_state.pending_d {
                state.tree_state.pending_d = false;
                if let Some(idx) = state.selected_tree_index() {
                    let mut walk = idx;
                    loop {
                        if let TreeNode::Connection { name, .. } = &state.tree[walk] {
                            return Action::DeleteConnection { name: name.clone() };
                        }
                        if walk == 0 {
                            break;
                        }
                        walk -= 1;
                    }
                }
                Action::Render
            } else {
                state.tree_state.pending_d = true;
                Action::Render
            }
        }
        KeyCode::Char('m') => {
            if let Some(idx) = state.selected_tree_index() {
                // If on a Group node, open group menu
                if let TreeNode::Group { name, .. } = &state.tree[idx] {
                    let group_name = name.clone();
                    let has_children = idx + 1 < state.tree.len()
                        && state.tree[idx + 1].depth() > state.tree[idx].depth();
                    state.group_menu.group_name = group_name;
                    state.group_menu.cursor = 0;
                    state.group_menu.is_empty = !has_children;
                    state.overlay = Some(Overlay::GroupMenu);
                    return Action::Render;
                }
                let mut walk = idx;
                loop {
                    if let TreeNode::Connection { name, status, .. } = &state.tree[walk] {
                        let conn_name = name.clone();
                        state.conn_menu.conn_name = conn_name;
                        state.conn_menu.cursor = 0;
                        state.conn_menu.is_connected =
                            *status == crate::ui::state::ConnStatus::Connected;
                        state.overlay = Some(Overlay::ConnectionMenu);
                        return Action::Render;
                    }
                    if walk == 0 {
                        break;
                    }
                    walk -= 1;
                }
            }
            Action::None
        }
        KeyCode::Char('/') => {
            state.tree_state.search_active = true;
            state.tree_state.search_query.clear();
            state.tree_state.search_matches.clear();
            Action::Render
        }
        _ => Action::None,
    }
}

/// Find the connection name for a tree node by walking up to the Connection node
fn find_conn_name_for(state: &AppState, mut idx: usize) -> String {
    loop {
        if let TreeNode::Connection { name, .. } = &state.tree[idx] {
            return name.clone();
        }
        if idx == 0 {
            break;
        }
        idx -= 1;
    }
    state.connection_name.clone().unwrap_or_default()
}

fn handle_tree_action(state: &mut AppState, idx: usize) -> Action {
    if idx >= state.tree.len() {
        return Action::None;
    }

    let node = &state.tree[idx];
    match node {
        TreeNode::Connection { expanded, name, .. } if !expanded => {
            let conn_name = name.clone();
            state.tree[idx].toggle_expand();
            Action::LoadSchemas { conn_name }
        }
        TreeNode::Schema { expanded, name, .. } if !expanded => {
            let schema = name.clone();
            state.tree[idx].toggle_expand();
            let has_children =
                idx + 1 < state.tree.len() && state.tree[idx + 1].depth() > state.tree[idx].depth();
            if !has_children {
                insert_categories(state, idx, &schema);
            }
            Action::Render
        }
        TreeNode::Category {
            expanded,
            schema,
            kind,
            ..
        } if !expanded => {
            let schema = schema.clone();
            let kind_str = format!("{:?}", kind);
            state.tree[idx].toggle_expand();
            Action::LoadChildren {
                schema,
                kind: kind_str,
            }
        }
        TreeNode::Leaf {
            schema,
            name,
            kind: LeafKind::Table | LeafKind::View,
            ..
        } => {
            let schema = schema.clone();
            let table = name.clone();
            let conn_name = find_conn_name_for(state, idx);
            state.current_schema = Some(schema.clone());

            let tab_id = state.open_or_focus_tab(TabKind::Table {
                conn_name,
                schema: schema.clone(),
                table: table.clone(),
            });

            Action::LoadTableData {
                tab_id,
                schema,
                table,
            }
        }
        TreeNode::Leaf {
            schema,
            name,
            kind: LeafKind::Package,
            ..
        } => {
            let schema = schema.clone();
            let pkg_name = name.clone();
            let conn_name = find_conn_name_for(state, idx);

            let tab_id = state.open_or_focus_tab(TabKind::Package {
                conn_name,
                schema: schema.clone(),
                name: pkg_name.clone(),
            });

            Action::LoadPackageContent {
                tab_id,
                schema,
                name: pkg_name,
            }
        }
        TreeNode::Leaf {
            schema,
            name,
            kind: LeafKind::Function,
            ..
        } => {
            let schema = schema.clone();
            let func_name = name.clone();
            let conn_name = find_conn_name_for(state, idx);

            let tab_id = state.open_or_focus_tab(TabKind::Function {
                conn_name,
                schema: schema.clone(),
                name: func_name.clone(),
            });

            Action::LoadSourceCode {
                tab_id,
                schema,
                name: func_name,
                obj_type: "FUNCTION".to_string(),
            }
        }
        TreeNode::Leaf {
            schema,
            name,
            kind: LeafKind::Procedure,
            ..
        } => {
            let schema = schema.clone();
            let proc_name = name.clone();
            let conn_name = find_conn_name_for(state, idx);

            let tab_id = state.open_or_focus_tab(TabKind::Procedure {
                conn_name,
                schema: schema.clone(),
                name: proc_name.clone(),
            });

            Action::LoadSourceCode {
                tab_id,
                schema,
                name: proc_name,
                obj_type: "PROCEDURE".to_string(),
            }
        }
        _ => {
            state.tree[idx].toggle_expand();
            Action::Render
        }
    }
}

fn insert_categories(state: &mut AppState, parent_idx: usize, schema: &str) {
    use crate::ui::state::CategoryKind;

    let categories = vec![
        ("Tables", CategoryKind::Tables),
        ("Views", CategoryKind::Views),
        ("Packages", CategoryKind::Packages),
        ("Procedures", CategoryKind::Procedures),
        ("Functions", CategoryKind::Functions),
    ];

    let insert_pos = parent_idx + 1;
    for (i, (label, kind)) in categories.into_iter().enumerate() {
        state.tree.insert(
            insert_pos + i,
            TreeNode::Category {
                label: label.to_string(),
                schema: schema.to_string(),
                kind,
                expanded: false,
            },
        );
    }
}

// --- Script Connection Picker ---

fn handle_script_conn_picker(state: &mut AppState, key: KeyEvent) -> Action {
    use crate::ui::state::PickerItem;

    let picker = match &mut state.script_conn_picker {
        Some(p) => p,
        None => {
            state.overlay = None;
            return Action::Render;
        }
    };
    let count = picker.visible_count();

    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            state.script_conn_picker = None;
            Action::Render
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if count > 0 {
                picker.cursor = (picker.cursor + 1).min(count - 1);
            }
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            picker.cursor = picker.cursor.saturating_sub(1);
            Action::Render
        }
        KeyCode::Enter | KeyCode::Char('l') => {
            let items = picker.visible_items();
            match items.get(picker.cursor) {
                Some(PickerItem::Active(name)) | Some(PickerItem::Other(name)) => {
                    let conn_name = name.clone();
                    state.overlay = None;
                    state.script_conn_picker = None;
                    Action::SetScriptConnection { conn_name }
                }
                Some(PickerItem::OthersHeader) => {
                    // Toggle expand/collapse
                    picker.others_expanded = !picker.others_expanded;
                    Action::Render
                }
                None => {
                    state.overlay = None;
                    state.script_conn_picker = None;
                    Action::Render
                }
            }
        }
        _ => Action::None,
    }
}

// --- Theme Picker ---

fn handle_theme_picker(state: &mut AppState, key: KeyEvent) -> Action {
    use crate::ui::theme::THEME_NAMES;

    let count = THEME_NAMES.len();
    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            Action::Render
        }
        KeyCode::Char('j') | KeyCode::Down => {
            state.theme_picker.cursor = (state.theme_picker.cursor + 1).min(count - 1);
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.theme_picker.cursor = state.theme_picker.cursor.saturating_sub(1);
            Action::Render
        }
        KeyCode::Enter => {
            let name = THEME_NAMES[state.theme_picker.cursor].to_string();
            state.overlay = None;
            Action::SetTheme { name }
        }
        _ => Action::None,
    }
}

// --- Bind Variables ---

/// Extract bind variable names (`:name` patterns) from a SQL query.
/// Skips string literals and comments. Returns unique names in order.
fn extract_bind_variables(query: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let bytes = query.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Skip string literals
        if bytes[i] == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                i += 1;
            }
            i += 1;
            continue;
        }
        // Skip line comments
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Detect :name (but not ::)
        if bytes[i] == b':'
            && i + 1 < bytes.len()
            && bytes[i + 1].is_ascii_alphabetic()
            && (i == 0 || bytes[i - 1] != b':')
        {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            let name = &query[start..end];
            if !name.is_empty() && seen.insert(name.to_string()) {
                vars.push(name.to_string());
            }
            i = end;
            continue;
        }
        i += 1;
    }

    vars
}

/// Check query for bind variables. If found, show prompt modal.
/// Otherwise, return the execute action directly.
fn maybe_prompt_bind_vars(
    state: &mut AppState,
    tab_id: TabId,
    query: String,
    start_line: usize,
    new_tab: bool,
) -> Action {
    let vars = extract_bind_variables(&query);
    if vars.is_empty() {
        if new_tab {
            Action::ExecuteQueryNewTab {
                tab_id,
                query,
                start_line,
            }
        } else {
            Action::ExecuteQuery {
                tab_id,
                query,
                start_line,
            }
        }
    } else {
        // Pre-fill with saved values from previous executions
        let saved = crate::ui::app::load_bind_variable_values();
        let variables = vars
            .into_iter()
            .map(|name| {
                let value = saved.get(&name).cloned().unwrap_or_default();
                (name, value)
            })
            .collect();

        state.bind_variables = Some(crate::ui::state::BindVariablesState {
            variables,
            selected_idx: 0,
            query,
            tab_id,
            start_line,
            new_tab,
        });
        state.overlay = Some(Overlay::BindVariables);
        Action::Render
    }
}

/// Handle key events in the bind variables prompt modal.
fn handle_bind_variables(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.bind_variables = None;
            state.overlay = None;
            Action::Render
        }
        KeyCode::Tab => {
            if let Some(ref mut bv) = state.bind_variables {
                bv.next_field();
            }
            Action::Render
        }
        KeyCode::BackTab => {
            if let Some(ref mut bv) = state.bind_variables {
                bv.prev_field();
            }
            Action::Render
        }
        KeyCode::Enter => {
            if let Some(bv) = state.bind_variables.take() {
                state.overlay = None;
                // Save values for future use
                crate::ui::app::save_bind_variable_values(&bv.variables);
                let final_query = bv.substituted_query();
                if bv.new_tab {
                    return Action::ExecuteQueryNewTab {
                        tab_id: bv.tab_id,
                        query: final_query,
                        start_line: bv.start_line,
                    };
                }
                return Action::ExecuteQuery {
                    tab_id: bv.tab_id,
                    query: final_query,
                    start_line: bv.start_line,
                };
            }
            Action::Render
        }
        KeyCode::Backspace => {
            if let Some(ref mut bv) = state.bind_variables {
                let idx = bv.selected_idx;
                bv.variables[idx].1.pop();
            }
            Action::Render
        }
        KeyCode::Char(c) => {
            if let Some(ref mut bv) = state.bind_variables {
                let idx = bv.selected_idx;
                bv.variables[idx].1.push(c);
            }
            Action::Render
        }
        _ => Action::None,
    }
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

/// Collect non-"Default" group names from the tree for persistence
fn persist_group_names(state: &AppState) -> Vec<String> {
    state
        .tree
        .iter()
        .filter_map(|n| {
            if let TreeNode::Group { name, .. } = n
                && name != "Default"
            {
                return Some(name.clone());
            }
            None
        })
        .collect()
}
