use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::collections::HashMap;
use std::time::Duration;

use crate::keybindings::Context;
use crate::ui::state::{AppState, Focus, Mode, Overlay, TreeNode};
use crate::ui::tabs::{SubView, TabId, TabKind, WorkspaceTab};
use vimltui::GutterSign;

pub(crate) mod editor;
mod grid;
mod leader;
mod oil;
pub(crate) mod overlays;
mod scripts;
mod sidebar;
use editor::*;
use grid::*;
use leader::*;
use overlays::*;
use scripts::*;
use sidebar::*;

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
    /// Reload every category in the given schema's expanded categories at once.
    RefreshSchema {
        schema: String,
        kinds: Vec<String>,
    },
    /// Fetch a package's declaration, parse out its function/procedure
    /// names, and stash them in the connection's MetadataIndex so completion
    /// can suggest `pkg.foo` from anywhere in the editor without the user
    /// having to open the package in a tab first.
    LoadPackageMembers {
        schema: String,
        package: String,
    },
    /// Fetch the pseudo-columns exposed by a PL/SQL function used in
    /// `TABLE(fn())` — their shape comes from the function's return type
    /// (an Oracle object type). Cached in the per-connection MetadataIndex.
    LoadFunctionReturnColumns {
        schema: Option<String>,
        package: Option<String>,
        function: String,
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
    #[allow(dead_code)]
    OpenNewScript,
    OpenScript {
        name: String,
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
    CreateSplit,
    CloseGroup,
    MoveTabToOther,
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
    CacheSchemaObjects {
        schema: String,
    },
    ScriptOp {
        op: ScriptOperation,
    },
    ReloadTableData,
    SaveGridChanges,
    LoadTableDDL {
        tab_id: TabId,
        schema: String,
        table: String,
    },
    LoadTypeInfo {
        tab_id: TabId,
        schema: String,
        name: String,
    },
    LoadTriggerInfo {
        tab_id: TabId,
        schema: String,
        name: String,
    },
    DropObject {
        conn_name: String,
        schema: String,
        name: String,
        obj_type: String,
    },
    RenameObject {
        conn_name: String,
        schema: String,
        old_name: String,
        new_name: String,
        obj_type: String,
    },
    CreateFromTemplate {
        conn_name: String,
        schema: String,
        obj_type: String,
    },
    DuplicateConnection {
        source_name: String,
        target_group: String,
    },
    ExportBundle,
    ImportBundle,
}

pub enum ScriptOperation {
    Create {
        name: String,
        in_collection: Option<String>,
    },
    Delete {
        path: String,
    },
    DeleteCollection {
        name: String,
    },
    Rename {
        old_path: String,
        new_name: String,
    },
    RenameCollection {
        old_name: String,
        new_name: String,
    },
    Move {
        from: String,
        to_collection: Option<String>,
    },
}

pub enum InputEvent {
    Key(KeyEvent),
    Paste(String),
}

pub fn poll_event(timeout: Duration) -> Option<InputEvent> {
    if event::poll(timeout).ok()? {
        match event::read().ok()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                return Some(InputEvent::Key(key));
            }
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
        && !state.sidebar.tree_state.search_active
        && let Some(action) = handle_global_leader(state, key)
    {
        return action;
    }

    // Ctrl+S: save script or compile source to DB. Skip when the floating
    // oil navigator is open — there Ctrl+S means "open in vertical split",
    // which oil's handler needs to receive.
    if key.code == KeyCode::Char('s')
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && state.overlay.is_none()
        && state.oil.is_none()
        && state.focus == Focus::TabContent
        && let Some(tab) = state.active_tab()
    {
        match &tab.kind {
            TabKind::Script { .. } => return Action::SaveScript,
            TabKind::Package { .. } | TabKind::Function { .. } | TabKind::Procedure { .. } => {
                return Action::CompileToDb { tab_id: tab.id };
            }
            _ => {}
        }
    }

    // Handle overlays first
    if let Some(overlay) = &state.overlay {
        return match overlay {
            Overlay::ConnectionDialog => handle_connection_dialog(state, key),
            Overlay::Help => handle_help_overlay(state, key),
            Overlay::ObjectFilter => handle_object_filter(state, key),
            Overlay::ConnectionMenu => handle_conn_menu(state, key),
            Overlay::GroupMenu => handle_group_menu(state, key),
            Overlay::ConfirmDeleteConnection { name } => {
                handle_confirm_delete_connection(state, key, name.clone())
            }
            Overlay::ConfirmClose => handle_confirm_close(state, key),
            Overlay::ConfirmQuit => handle_confirm_quit(state, key),
            Overlay::SaveScriptName => handle_save_script_name(state, key),
            Overlay::ScriptConnection => handle_script_conn_picker(state, key),
            Overlay::ThemePicker => handle_theme_picker(state, key),
            Overlay::BindVariables => handle_bind_variables(state, key),
            Overlay::SaveGridChanges => handle_save_grid_confirm(state, key),
            Overlay::RenameObject => match key.code {
                KeyCode::Enter => {
                    let new_name = state.sidebar.rename_buf.trim().to_string();
                    state.overlay = None;
                    if let Some(action) = state.sidebar.pending_action.take() {
                        if new_name.is_empty() || new_name == action.name {
                            state.sidebar.rename_buf.clear();
                            state.status_message = "Rename cancelled".to_string();
                            Action::Render
                        } else {
                            state.sidebar.rename_buf.clear();
                            Action::RenameObject {
                                conn_name: action.conn_name,
                                schema: action.schema,
                                old_name: action.name,
                                new_name,
                                obj_type: action.obj_type,
                            }
                        }
                    } else {
                        Action::Render
                    }
                }
                KeyCode::Esc => {
                    state.overlay = None;
                    state.sidebar.pending_action = None;
                    state.sidebar.rename_buf.clear();
                    Action::Render
                }
                KeyCode::Char(c) => {
                    state.sidebar.rename_buf.push(c);
                    Action::Render
                }
                KeyCode::Backspace => {
                    state.sidebar.rename_buf.pop();
                    Action::Render
                }
                _ => Action::Render,
            },
            Overlay::ConfirmDropObject => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    state.overlay = None;
                    if let Some(action) = state.sidebar.pending_action.take() {
                        Action::DropObject {
                            conn_name: action.conn_name,
                            schema: action.schema,
                            name: action.name,
                            obj_type: action.obj_type,
                        }
                    } else {
                        Action::Render
                    }
                }
                _ => {
                    state.overlay = None;
                    state.sidebar.pending_action = None;
                    state.status_message = "Drop cancelled".to_string();
                    Action::Render
                }
            },
            Overlay::ExportDialog => handle_export_dialog(state, key),
            Overlay::ImportDialog => handle_import_dialog(state, key),
            Overlay::ConfirmCompile => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    state.overlay = None;
                    state.compile_confirmed = true;
                    if let Some(tab) = state.active_tab() {
                        let tab_id = tab.id;
                        Action::CompileToDb { tab_id }
                    } else {
                        Action::Render
                    }
                }
                _ => {
                    state.overlay = None;
                    state.status_message = "Compile cancelled".to_string();
                    Action::Render
                }
            },
        };
    }

    // Handle oil floating navigator (above normal focus, below overlays)
    if state.oil.is_some() {
        return oil::handle_oil(state, key);
    }

    // Handle sidebar search mode
    if state.sidebar.tree_state.search_active {
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

    // (Numeric panel jump aliases 1/2/3/4 — with or without Ctrl — were
    // removed: they collided with Vim count prefixes (e.g. `d3j`) and the
    // spatial navigation via Ctrl+h/j/k/l covers the same use case.)

    // Configurable-binding dispatch (global context). Checked before the
    // fallthrough match so users can rebind these keys via keybindings.toml.
    if state.bindings.matches(Context::Global, "help", &key) {
        state.overlay = Some(Overlay::Help);
        return Some(Action::Render);
    }
    if state.focus == Focus::Sidebar
        && state.bindings.matches(Context::Global, "add_connection", &key)
    {
        let groups = state.available_groups();
        let current_group = state
            .selected_tree_index()
            .and_then(|idx| {
                let mut i = idx;
                loop {
                    if let TreeNode::Group { name, .. } = &state.sidebar.tree[i] {
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
        state.dialogs.connection_form = crate::ui::state::ConnectionFormState::new();
        state.dialogs.connection_form.group = current_group;
        state.dialogs.connection_form.group_options = groups;
        state.overlay = Some(Overlay::ConnectionDialog);
        return Some(Action::Render);
    }
    if state
        .bindings
        .matches(Context::Global, "filter_objects", &key)
    {
        return Some(handle_filter_key(state));
    }
    if state
        .bindings
        .matches(Context::Global, "toggle_oil_navigator", &key)
    {
        if state.oil.is_some() {
            let prev = state.oil.take().map(|o| o.previous_focus);
            if let Some(f) = prev {
                state.focus = f;
            }
        } else {
            state.oil = Some(crate::ui::state::OilState::new(state.focus));
        }
        return Some(Action::Render);
    }
    if state.bindings.matches(Context::Global, "next_tab", &key) {
        if let Some(groups) = state.groups.as_mut() {
            let g = &mut groups[state.active_group];
            if !g.tab_ids.is_empty() {
                g.active_idx = (g.active_idx + 1) % g.tab_ids.len();
                state.focus = Focus::TabContent;
                state.sync_active_tab_idx();
            }
        } else if !state.tabs.is_empty() {
            state.active_tab_idx = (state.active_tab_idx + 1) % state.tabs.len();
            state.focus = Focus::TabContent;
        }
        return Some(Action::Render);
    }
    if state.bindings.matches(Context::Global, "prev_tab", &key) {
        if let Some(groups) = state.groups.as_mut() {
            let g = &mut groups[state.active_group];
            if !g.tab_ids.is_empty() {
                g.active_idx = if g.active_idx == 0 {
                    g.tab_ids.len() - 1
                } else {
                    g.active_idx - 1
                };
                state.focus = Focus::TabContent;
                state.sync_active_tab_idx();
            }
        } else if !state.tabs.is_empty() {
            state.active_tab_idx = if state.active_tab_idx == 0 {
                state.tabs.len() - 1
            } else {
                state.active_tab_idx - 1
            };
            state.focus = Focus::TabContent;
        }
        return Some(Action::Render);
    }
    if state
        .bindings
        .matches(Context::Global, "next_sub_view", &key)
    {
        if let Some(tab) = state.active_tab_mut() {
            let is_script_with_results = matches!(tab.kind, TabKind::Script { .. })
                && tab.result_tabs.len() > 1;
            if is_script_with_results {
                grid::sync_grid_to_result_tab(tab);
                tab.active_result_idx = (tab.active_result_idx + 1) % tab.result_tabs.len();
            } else {
                tab.next_sub_view();
                tab.sync_grid_for_subview();
            }
        }
        return maybe_load_ddl(state);
    }
    if state
        .bindings
        .matches(Context::Global, "prev_sub_view", &key)
    {
        if let Some(tab) = state.active_tab_mut() {
            let is_script_with_results = matches!(tab.kind, TabKind::Script { .. })
                && tab.result_tabs.len() > 1;
            if is_script_with_results {
                grid::sync_grid_to_result_tab(tab);
                tab.active_result_idx = if tab.active_result_idx == 0 {
                    tab.result_tabs.len() - 1
                } else {
                    tab.active_result_idx - 1
                };
            } else {
                tab.prev_sub_view();
                tab.sync_grid_for_subview();
            }
        }
        return maybe_load_ddl(state);
    }

    None
}

/// If the active sub-view just switched to TableDDL and the editor is empty,
/// return a LoadTableDDL action so the DDL gets fetched.
fn maybe_load_ddl(state: &AppState) -> Option<Action> {
    let tab = state.active_tab()?;
    if tab.active_sub_view.as_ref() != Some(&SubView::TableDDL) {
        return Some(Action::Render);
    }
    // Already loaded
    if let Some(editor) = &tab.ddl_editor
        && !editor.content().is_empty()
    {
        return Some(Action::Render);
    }
    if let TabKind::Table { schema, table, .. } = &tab.kind {
        Some(Action::LoadTableDDL {
            tab_id: tab.id,
            schema: schema.clone(),
            table: table.clone(),
        })
    } else {
        Some(Action::Render)
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
    if in_editor_special_mode {
        return None;
    }

    use crate::ui::tabs::SubFocus;

    // Resolve which directional action (if any) the key binds to. Using the
    // configurable bindings here means users can remap Ctrl+hjkl to anything.
    let dir = if state.bindings.matches(Context::Global, "navigate_left", &key) {
        Some('h')
    } else if state
        .bindings
        .matches(Context::Global, "navigate_right", &key)
    {
        Some('l')
    } else if state
        .bindings
        .matches(Context::Global, "navigate_down", &key)
    {
        Some('j')
    } else if state.bindings.matches(Context::Global, "navigate_up", &key) {
        Some('k')
    } else {
        None
    };
    let dir = dir?;

    let sub = state
        .active_tab()
        .map(|t| t.sub_focus)
        .unwrap_or(SubFocus::Editor);
    let has_tabs = !state.tabs.is_empty();

    // Group navigation: when split is active and focus is TabContent, Ctrl+h/l
    // switches between groups before any other transition.
    let is_split = state.groups.is_some();

    match dir {
        'h' => {
            // Within-tab nav has priority: QueryView → Results
            if state.focus == Focus::TabContent && sub == SubFocus::QueryView {
                if let Some(tab) = state.active_tab_mut() {
                    tab.sub_focus = SubFocus::Results;
                }
                return Some(Action::Render);
            }
            // Group nav: from group 1 → group 0
            if is_split && state.focus == Focus::TabContent && state.active_group == 1 {
                state.active_group = 0;
                state.sync_active_tab_idx();
                return Some(Action::Render);
            }
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
        'l' => {
            // Within-tab nav has priority: Results → QueryView (if query pane exists)
            if state.focus == Focus::TabContent && sub == SubFocus::Results {
                let has_query = state.active_tab().is_some_and(|t| {
                    let idx = t.active_result_idx;
                    (idx < t.result_tabs.len() && t.result_tabs[idx].query_editor.is_some())
                        || t.grid_query_editor.is_some()
                });
                if has_query {
                    if let Some(tab) = state.active_tab_mut() {
                        tab.sub_focus = SubFocus::QueryView;
                    }
                    return Some(Action::Render);
                }
            }
            // Group nav: from group 0 → group 1
            if is_split && state.focus == Focus::TabContent && state.active_group == 0 {
                state.active_group = 1;
                state.sync_active_tab_idx();
                return Some(Action::Render);
            }
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
                        (idx < t.result_tabs.len() && t.result_tabs[idx].query_editor.is_some())
                            || t.grid_query_editor.is_some()
                    });
                    if has_query && let Some(tab) = state.active_tab_mut() {
                        tab.sub_focus = SubFocus::QueryView;
                    }
                }
                _ => {}
            }
            Some(Action::Render)
        }
        'j' => {
            match (state.focus, sub) {
                // Explorer -> Scripts panel
                (Focus::Sidebar, _) => state.focus = Focus::ScriptsPanel,
                // Script/Grid -> Error/Results
                (Focus::TabContent, SubFocus::Editor) => {
                    let has_error_pane = state
                        .active_tab()
                        .is_some_and(|t| t.grid_error_editor.is_some());
                    let has_bottom = state
                        .active_tab()
                        .is_some_and(|t| !t.result_tabs.is_empty() || t.query_result.is_some());
                    if has_error_pane {
                        if let Some(tab) = state.active_tab_mut() {
                            tab.sub_focus = SubFocus::Results;
                            tab.grid_focused = false;
                        }
                    } else if has_bottom && let Some(tab) = state.active_tab_mut() {
                        tab.sub_focus = SubFocus::Results;
                        tab.grid_focused = true;
                    }
                }
                _ => {}
            }
            Some(Action::Render)
        }
        'k' => {
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
        Some(SubView::TableData) => {
            let tab = &state.tabs[tab_idx];
            let has_error = tab.grid_error_editor.is_some();
            let sub = tab.sub_focus;

            // When error panes exist, route by sub_focus
            if has_error {
                use crate::ui::tabs::SubFocus;
                match sub {
                    SubFocus::Results => {
                        return handle_table_error_editor(state, key, false);
                    }
                    SubFocus::QueryView => {
                        return handle_table_error_editor(state, key, true);
                    }
                    SubFocus::Editor => {}
                }
            }

            handle_tab_data_grid(state, key)
        }
        Some(SubView::TableProperties) => handle_tab_data_grid(state, key),
        Some(SubView::TableDDL) => handle_tab_editor(state, key),
        Some(SubView::PackageBody) | Some(SubView::PackageDeclaration) => {
            let tab = &state.tabs[tab_idx];
            let has_error = tab.grid_error_editor.is_some();
            let sub = tab.sub_focus;
            if has_error {
                use crate::ui::tabs::SubFocus;
                match sub {
                    SubFocus::Results => {
                        return handle_table_error_editor(state, key, false);
                    }
                    SubFocus::QueryView => {
                        return handle_table_error_editor(state, key, true);
                    }
                    SubFocus::Editor => {}
                }
            }
            handle_tab_editor(state, key)
        }
        Some(SubView::PackageFunctions) | Some(SubView::PackageProcedures) => {
            handle_tab_package_list(state, key)
        }
        Some(SubView::TypeAttributes)
        | Some(SubView::TypeMethods)
        | Some(SubView::TriggerColumns) => handle_tab_data_grid(state, key),
        Some(SubView::TypeDeclaration)
        | Some(SubView::TypeBody)
        | Some(SubView::TriggerDeclaration) => handle_tab_editor(state, key),
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

// --- Scripts Panel (Oil-style) ---

/// Compute diff signs comparing original content against current editor lines.
/// Uses LCS (Longest Common Subsequence) to correctly detect insertions,
/// deletions, and modifications — not just positional comparison.
pub(super) fn compute_diff_signs(original: &str, current: &[String]) -> HashMap<usize, GutterSign> {
    let orig: Vec<&str> = original.lines().collect();
    let cur: Vec<&str> = current.iter().map(|s| s.as_str()).collect();
    let mut signs = HashMap::new();

    if orig.len() == cur.len()
        && orig
            .iter()
            .zip(cur.iter())
            .all(|(a, b)| a.trim_end() == b.trim_end())
    {
        return signs;
    }

    let n = orig.len();
    let m = cur.len();

    // Compare lines with trailing-whitespace tolerance
    // (editors may add/remove trailing spaces on empty lines)
    let lines_eq = |a: &str, b: &str| -> bool { a.trim_end() == b.trim_end() };

    // Build LCS table
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            dp[i][j] = if lines_eq(orig[i - 1], cur[j - 1]) {
                dp[i - 1][j - 1] + 1
            } else {
                dp[i - 1][j].max(dp[i][j - 1])
            };
        }
    }

    // Backtrack to classify each line.
    // On ties (dp[i-1][j] == dp[i][j-1]), prefer the direction that keeps
    // lines closer to their original position — reduces false diffs on
    // duplicate lines (empty lines, repeated patterns).
    let mut i = n;
    let mut j = m;
    let mut cur_matched = vec![false; m];
    let mut orig_matched = vec![false; n];

    while i > 0 && j > 0 {
        if lines_eq(orig[i - 1], cur[j - 1]) {
            cur_matched[j - 1] = true;
            orig_matched[i - 1] = true;
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] > dp[i][j - 1] {
            i -= 1;
        } else if dp[i][j - 1] > dp[i - 1][j] {
            j -= 1;
        } else {
            // Tie: prefer skipping the side farther from the diagonal
            // to keep matches positionally aligned
            if i > j {
                i -= 1;
            } else {
                j -= 1;
            }
        }
    }

    // Post-process: ensure unmatched counts explain the line count difference.
    // If lines were added (m > n), we need at least (m - n) unmatched cur lines.
    // If lines were deleted (n > m), we need at least (n - m) unmatched orig lines.
    // When the LCS over-matches trivial lines (empty), unmatch the most displaced ones.
    {
        let unmatched_cur_count = cur_matched.iter().filter(|&&m| !m).count();
        let unmatched_orig_count = orig_matched.iter().filter(|&&m| !m).count();
        let need_cur_unmatched = m.saturating_sub(n);
        let need_orig_unmatched = n.saturating_sub(m);

        // Need more unmatched cur lines (additions not detected)
        if unmatched_cur_count < need_cur_unmatched {
            let deficit = need_cur_unmatched - unmatched_cur_count;
            // Find matched trivial cur lines sorted by displacement (most displaced first)
            let mut candidates: Vec<(usize, usize, usize)> = Vec::new(); // (cur_idx, orig_idx, displacement)
            let (mut oi, mut ci) = (0, 0);
            while oi < n && ci < m {
                if orig_matched[oi] && cur_matched[ci] && lines_eq(orig[oi], cur[ci]) {
                    if cur[ci].trim().is_empty() && oi != ci {
                        candidates.push((ci, oi, oi.abs_diff(ci)));
                    }
                    oi += 1;
                    ci += 1;
                } else if !orig_matched[oi] {
                    oi += 1;
                } else {
                    ci += 1;
                }
            }
            candidates.sort_by(|a, b| b.2.cmp(&a.2));
            for (ci, oi, _) in candidates.into_iter().take(deficit) {
                cur_matched[ci] = false;
                orig_matched[oi] = false;
            }
        }

        // Need more unmatched orig lines (deletions not detected)
        if unmatched_orig_count < need_orig_unmatched {
            let deficit = need_orig_unmatched - unmatched_orig_count;
            let mut candidates: Vec<(usize, usize, usize)> = Vec::new();
            let (mut oi, mut ci) = (0, 0);
            while oi < n && ci < m {
                if orig_matched[oi] && cur_matched[ci] && lines_eq(orig[oi], cur[ci]) {
                    if orig[oi].trim().is_empty() && oi != ci {
                        candidates.push((oi, ci, oi.abs_diff(ci)));
                    }
                    oi += 1;
                    ci += 1;
                } else if !orig_matched[oi] {
                    oi += 1;
                } else {
                    ci += 1;
                }
            }
            candidates.sort_by(|a, b| b.2.cmp(&a.2));
            for (oi, ci, _) in candidates.into_iter().take(deficit) {
                orig_matched[oi] = false;
                cur_matched[ci] = false;
            }
        }

        // Same line count but content differs: if LCS matched everything
        // (0 unmatched on both sides), trivial lines absorbed a delete+add.
        // Unmatch the most displaced trivial pairs.
        let unmatched_cur_count = cur_matched.iter().filter(|&&m| !m).count();
        let unmatched_orig_count = orig_matched.iter().filter(|&&m| !m).count();
        if n == m && unmatched_cur_count == 0 && unmatched_orig_count == 0 {
            let mut candidates: Vec<(usize, usize, usize)> = Vec::new();
            let (mut oi, mut ci) = (0, 0);
            while oi < n && ci < m {
                if orig_matched[oi] && cur_matched[ci] && lines_eq(orig[oi], cur[ci]) {
                    if orig[oi].trim().is_empty() && oi != ci {
                        candidates.push((oi, ci, oi.abs_diff(ci)));
                    }
                    oi += 1;
                    ci += 1;
                } else if !orig_matched[oi] {
                    oi += 1;
                } else {
                    ci += 1;
                }
            }
            if !candidates.is_empty() {
                candidates.sort_by(|a, b| b.2.cmp(&a.2));
                // Unmatch one pair to reveal the hidden delete+add
                let (oi, ci, _) = candidates[0];
                orig_matched[oi] = false;
                cur_matched[ci] = false;
            }
        }
    }

    // Collect unmatched line indices
    let unmatched_orig: Vec<usize> = orig_matched
        .iter()
        .enumerate()
        .filter(|(_, m)| !*m)
        .map(|(i, _)| i)
        .collect();
    let unmatched_cur: Vec<usize> = cur_matched
        .iter()
        .enumerate()
        .filter(|(_, m)| !*m)
        .map(|(i, _)| i)
        .collect();

    // Pair unmatched orig lines with unmatched current lines by string similarity.
    // Each unmatched orig line finds the most similar unclaimed current line
    // → Modified. Leftovers → Added / Deleted.
    let mut claimed_cur = vec![false; unmatched_cur.len()];

    for &oi in &unmatched_orig {
        let orig_line = orig[oi];
        let mut best: Option<(usize, usize)> = None; // (index into unmatched_cur, common_chars)
        for (k, &ci) in unmatched_cur.iter().enumerate() {
            if claimed_cur[k] {
                continue;
            }
            // Count common prefix + suffix chars as a simple similarity metric
            let cur_line = cur[ci];
            let common_prefix = orig_line
                .chars()
                .zip(cur_line.chars())
                .take_while(|(a, b)| a == b)
                .count();
            let common_suffix = orig_line
                .chars()
                .rev()
                .zip(cur_line.chars().rev())
                .take_while(|(a, b)| a == b)
                .count();
            let similarity = common_prefix + common_suffix;
            // Require at least some similarity (>30% of shorter line) to pair as Modified
            let min_len = orig_line.len().min(cur_line.len()).max(1);
            if similarity * 3 > min_len && (best.is_none() || similarity > best.unwrap().1) {
                best = Some((k, similarity));
            }
        }
        if let Some((k, _)) = best {
            claimed_cur[k] = true;
            signs.insert(unmatched_cur[k], GutterSign::Modified);
        }
    }

    // Unclaimed current lines → Added
    for (k, &ci) in unmatched_cur.iter().enumerate() {
        if !claimed_cur[k] {
            signs.insert(ci, GutterSign::Added);
        }
    }

    // Unpaired orig lines (more deletes than inserts) → Deleted indicators
    let has_unpaired_orig = unmatched_orig.len() > unmatched_cur.len();
    if has_unpaired_orig {
        // Build orig→cur position map for matched lines
        let mut orig_to_cur: Vec<Option<usize>> = vec![None; n];
        let (mut oi2, mut ci2) = (0, 0);
        while oi2 < n && ci2 < m {
            if orig_matched[oi2] && cur_matched[ci2] && lines_eq(orig[oi2], cur[ci2]) {
                orig_to_cur[oi2] = Some(ci2);
                oi2 += 1;
                ci2 += 1;
            } else if !orig_matched[oi2] {
                oi2 += 1;
            } else {
                ci2 += 1;
            }
        }

        // Count how many orig lines were actually paired
        let paired_count = claimed_cur.iter().filter(|&&c| c).count();
        // The unpaired orig lines are those beyond what we could pair
        // Find them: orig lines that didn't get a cur partner
        let mut paired_orig_count = 0;
        for &oi in &unmatched_orig {
            if paired_orig_count < paired_count {
                paired_orig_count += 1;
                continue;
            }
            let cur_pos = orig_to_cur[oi..].iter().find_map(|c| *c);
            if let Some(pos) = cur_pos {
                if pos > 0 {
                    signs.entry(pos - 1).or_insert(GutterSign::DeletedBelow);
                } else {
                    signs.entry(0).or_insert(GutterSign::DeletedAbove);
                }
            } else if !cur.is_empty() {
                signs
                    .entry(cur.len() - 1)
                    .or_insert(GutterSign::DeletedBelow);
            }
        }
    }

    signs
}
