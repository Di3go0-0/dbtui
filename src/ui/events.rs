use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;

use crate::ui::state::{AppState, Focus, LeafKind, Mode, Overlay, TreeNode};
use crate::ui::tabs::{SubView, TabId, TabKind};
use crate::ui::vim::EditorAction;

pub enum Action {
    Quit,
    Render,
    None,
    LoadSchemas { conn_name: String },
    SaveSchemaFilter,
    LoadChildren { schema: String, kind: String },
    LoadTableData { tab_id: TabId, schema: String, table: String },
    LoadColumns { tab_id: TabId, schema: String, table: String },
    LoadPackageContent { tab_id: TabId, schema: String, name: String },
    ExecuteQuery { tab_id: TabId, query: String },
    LoadSourceCode { tab_id: TabId, schema: String, name: String, obj_type: String },
    LoadTableDDL { tab_id: TabId, schema: String, table: String },
    OpenNewScript,
    CloseTab,
    SaveScript,
    ConfirmCloseYes,
    ConfirmCloseNo,
    Connect,
    ConnectByName { name: String },
    DisconnectByName { name: String },
    SaveConnection,
    DeleteConnection { name: String },
    EditConnection { name: String },
}

pub fn poll_event(timeout: Duration) -> Option<KeyEvent> {
    if event::poll(timeout).ok()? {
        if let Event::Key(key) = event::read().ok()? {
            return Some(key);
        }
    }
    None
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Action {
    // Handle overlays first
    if let Some(overlay) = &state.overlay {
        return match overlay {
            Overlay::ConnectionDialog => handle_connection_dialog(state, key),
            Overlay::Help => handle_help_overlay(state, key),
            Overlay::ObjectFilter => handle_object_filter(state, key),
            Overlay::ConnectionMenu => handle_conn_menu(state, key),
            Overlay::ConfirmClose => handle_confirm_close(state, key),
            _ => Action::None,
        };
    }

    // Handle sidebar search mode
    if state.tree_state.search_active {
        return handle_sidebar_search(state, key);
    }

    // Global keys (Normal mode only, when no editor is in insert/visual)
    let in_editor_special_mode = if state.focus == Focus::TabContent {
        if let Some(tab) = state.active_tab() {
            if let Some(editor) = tab.active_editor() {
                !matches!(editor.mode, crate::ui::vim::VimMode::Normal)
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    if state.mode == Mode::Normal && !in_editor_special_mode {
        match key.code {
            KeyCode::Char('q') => {
                return Action::Quit;
            }
            KeyCode::Char('?') => {
                state.overlay = Some(Overlay::Help);
                return Action::Render;
            }
            KeyCode::Char('a') if state.focus == Focus::Sidebar => {
                state.overlay = Some(Overlay::ConnectionDialog);
                state.connection_form = crate::ui::state::ConnectionFormState::new();
                return Action::Render;
            }
            KeyCode::Char('n') => {
                return Action::OpenNewScript;
            }
            KeyCode::Char('F') => {
                return handle_filter_key(state);
            }
            KeyCode::Char(']') => {
                // Next tab
                if !state.tabs.is_empty() {
                    state.active_tab_idx = (state.active_tab_idx + 1) % state.tabs.len();
                    state.focus = Focus::TabContent;
                }
                return Action::Render;
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
                return Action::Render;
            }
            KeyCode::Char('}') => {
                // Next sub-view
                if let Some(tab) = state.active_tab_mut() {
                    tab.next_sub_view();
                }
                return Action::Render;
            }
            KeyCode::Char('{') => {
                // Previous sub-view
                if let Some(tab) = state.active_tab_mut() {
                    tab.prev_sub_view();
                }
                return Action::Render;
            }
            _ => {}
        }
    }

    // Focus switching with Ctrl
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => {
                state.focus = Focus::Sidebar;
                return Action::Render;
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if !state.tabs.is_empty() {
                    state.focus = Focus::TabContent;
                }
                return Action::Render;
            }
            _ => {}
        }
    }

    match state.focus {
        Focus::Sidebar => handle_sidebar(state, key),
        Focus::TabContent => handle_tab_content(state, key),
    }
}

// --- Tab Content Dispatch ---

fn handle_tab_content(state: &mut AppState, key: KeyEvent) -> Action {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return Action::None;
    }

    let sub_view = state.tabs[tab_idx].active_sub_view.clone();

    // For non-editor views, handle leader+bd at this level
    let is_non_editor_view = matches!(
        sub_view,
        Some(SubView::TableData) | Some(SubView::TableProperties)
            | Some(SubView::PackageFunctions) | Some(SubView::PackageProcedures)
    );
    if is_non_editor_view {
        if let Some(action) = handle_leader_bd(state, key) {
            return action;
        }
    }

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
            // Script / Function / Procedure - has an editor
            handle_tab_editor(state, key)
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

    if let Some(editor) = tab.active_editor_mut() {
        match editor.handle_key(key) {
            EditorAction::Handled => Action::Render,
            EditorAction::Unhandled(_) => Action::None,
            EditorAction::ExecuteQuery(query) => Action::ExecuteQuery { tab_id, query },
            EditorAction::CloseBuffer => Action::CloseTab,
            EditorAction::SaveBuffer => Action::SaveScript,
        }
    } else {
        Action::None
    }
}

fn handle_tab_data_grid(state: &mut AppState, key: KeyEvent) -> Action {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return Action::None;
    }

    let tab = &mut state.tabs[tab_idx];
    let row_count = tab.query_result.as_ref().map(|r| r.rows.len()).unwrap_or(0);
    let vh = tab.grid_visible_height.max(1);

    match key.code {
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
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = vh / 2;
            tab.grid_selected_row =
                (tab.grid_selected_row + half).min(row_count.saturating_sub(1));
            tab.grid_scroll_row = tab.grid_selected_row.saturating_sub(vh / 2);
            Action::Render
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = vh / 2;
            tab.grid_selected_row = tab.grid_selected_row.saturating_sub(half);
            tab.grid_scroll_row = tab.grid_selected_row.saturating_sub(vh / 2);
            Action::Render
        }
        KeyCode::Char('g') => {
            tab.grid_selected_row = 0;
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
        _ => Action::None,
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
        _ => {
            // Handle leader+bd for non-editor views
            if let Some(action) = handle_leader_bd(state, key) {
                return action;
            }
            Action::None
        }
    }
}

// --- Leader key for non-editor views ---

fn handle_leader_bd(state: &mut AppState, key: KeyEvent) -> Option<Action> {
    if state.leader_b_pending {
        state.leader_b_pending = false;
        state.leader_pending = false;
        if let KeyCode::Char('d') = key.code {
            return Some(Action::CloseTab);
        }
        return Some(Action::Render);
    }
    if state.leader_pending {
        state.leader_pending = false;
        if let KeyCode::Char('b') = key.code {
            state.leader_b_pending = true;
            return Some(Action::Render);
        }
        return Some(Action::Render);
    }
    if let KeyCode::Char(c) = key.code {
        if c == crate::ui::vim::LEADER_KEY {
            state.leader_pending = true;
            return Some(Action::Render);
        }
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

// --- Filter Key ---

fn handle_filter_key(state: &mut AppState) -> Action {
    if let Some(idx) = state.selected_tree_index() {
        match &state.tree[idx] {
            TreeNode::Connection { .. } | TreeNode::Schema { .. } => {
                let schemas = state.all_schema_names();
                if !schemas.is_empty() {
                    state.object_filter.open_for("schemas", schemas);
                    state.overlay = Some(Overlay::ObjectFilter);
                }
            }
            TreeNode::Category { schema, kind, .. } => {
                let key = kind.filter_key(schema);
                let items = state.leaves_under_category(idx);
                if !items.is_empty() {
                    state.object_filter.open_for(&key, items);
                    state.overlay = Some(Overlay::ObjectFilter);
                }
            }
            TreeNode::Leaf { schema, kind, .. } => {
                let cat_key = match kind {
                    LeafKind::Table => format!("{schema}.Tables"),
                    LeafKind::View => format!("{schema}.Views"),
                    LeafKind::Package => format!("{schema}.Packages"),
                    LeafKind::Procedure => format!("{schema}.Procedures"),
                    LeafKind::Function => format!("{schema}.Functions"),
                };
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
            state.object_filter.offset =
                state.object_filter.cursor.saturating_sub(state.object_filter.visible_height / 2);
            Action::Render
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = state.object_filter.visible_height / 2;
            state.object_filter.cursor = state.object_filter.cursor.saturating_sub(half);
            state.object_filter.offset =
                state.object_filter.cursor.saturating_sub(state.object_filter.visible_height / 2);
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
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Action::SaveConnection
        }
        KeyCode::Char(c) => {
            if state.connection_form.selected_field == 1 {
                return Action::None;
            }
            state.connection_form.active_field_mut().push(c);
            state.connection_form.error_message.clear();
            Action::Render
        }
        KeyCode::Backspace => {
            if state.connection_form.selected_field != 1 {
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
                state.connection_form =
                    crate::ui::state::ConnectionFormState::from_config(&config);
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
                    if let Some(config) = state
                        .saved_connections
                        .iter()
                        .find(|c| c.name == name)
                    {
                        let mut form =
                            crate::ui::state::ConnectionFormState::from_config(config);
                        form.password = "********".to_string();
                        form.password_visible = false;
                        form.read_only = true;
                        state.connection_form = form;
                        state.overlay = Some(Overlay::ConnectionDialog);
                    }
                    Action::Render
                }
                ConnMenuAction::Edit => {
                    if let Some(config) = state
                        .saved_connections
                        .iter()
                        .find(|c| c.name == name)
                    {
                        state.connection_form =
                            crate::ui::state::ConnectionFormState::for_edit(config);
                        state.overlay = Some(Overlay::ConnectionDialog);
                    }
                    Action::Render
                }
                ConnMenuAction::Connect => Action::ConnectByName { name },
                ConnMenuAction::Disconnect => Action::DisconnectByName { name },
                ConnMenuAction::Restart => {
                    Action::ConnectByName { name }
                }
                ConnMenuAction::Delete => Action::DeleteConnection { name },
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
            if let Some(idx) = state.selected_tree_index() {
                if state.tree[idx].is_expanded() {
                    state.tree[idx].toggle_expand();
                }
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
                            return Action::DeleteConnection {
                                name: name.clone(),
                            };
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
            let has_children = idx + 1 < state.tree.len()
                && state.tree[idx + 1].depth() > state.tree[idx].depth();
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

            Action::LoadTableData { tab_id, schema, table }
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
            if state.tree[idx].is_expanded() {
                state.tree[idx].toggle_expand();
            }
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
