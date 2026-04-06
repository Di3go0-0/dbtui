use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::ui::state::{AppState, ExportField, ImportField, Overlay, TreeNode};
use crate::ui::tabs::TabId;

use super::Action;

// --- Confirm Delete Connection Overlay ---

pub(super) fn handle_confirm_delete_connection(
    state: &mut AppState,
    key: KeyEvent,
    name: String,
) -> Action {
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            state.overlay = None;
            Action::DeleteConnection { name }
        }
        KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => {
            state.overlay = None;
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Confirm Close Overlay ---

pub(super) fn handle_confirm_close(state: &mut AppState, key: KeyEvent) -> Action {
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

pub(super) fn handle_confirm_quit(state: &mut AppState, key: KeyEvent) -> Action {
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

pub(super) fn handle_save_script_name(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.scripts.save_name = None;
            state.overlay = None;
            Action::Render
        }
        KeyCode::Enter => {
            if let Some(name) = state.scripts.save_name.take() {
                state.overlay = None;
                if !name.is_empty() {
                    return Action::SaveScriptAs { name };
                }
            }
            Action::Render
        }
        KeyCode::Backspace => {
            if let Some(ref mut buf) = state.scripts.save_name {
                buf.pop();
            }
            Action::Render
        }
        KeyCode::Char(c) => {
            if let Some(ref mut buf) = state.scripts.save_name {
                buf.push(c);
            }
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Group Rename ---

pub(super) fn handle_group_rename(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.dialogs.group_renaming = None;
            state.dialogs.group_rename_buf.clear();
            Action::Render
        }
        KeyCode::Enter => {
            let new_name = state.dialogs.group_rename_buf.trim().to_string();
            if let Some(old_name) = state.dialogs.group_renaming.take() {
                state.dialogs.group_rename_buf.clear();
                if !new_name.is_empty() && new_name != old_name {
                    // Rename in tree
                    for node in &mut state.sidebar.tree {
                        if let TreeNode::Group { name, .. } = node
                            && *name == old_name
                        {
                            *name = new_name.clone();
                        }
                    }
                    // Rename in saved connections
                    for conn in &mut state.dialogs.saved_connections {
                        if conn.group == old_name {
                            conn.group = new_name.clone();
                        }
                    }
                    // Persist connections and groups
                    if let Ok(store) = crate::core::storage::ConnectionStore::new() {
                        let _ = store.save(&state.dialogs.saved_connections, "");
                        let _ = store.save_groups(&persist_group_names(state));
                    }
                    state.status_message =
                        format!("Group renamed: '{old_name}' \u{2192} '{new_name}'");
                }
            }
            Action::Render
        }
        KeyCode::Backspace => {
            state.dialogs.group_rename_buf.pop();
            Action::Render
        }
        KeyCode::Char(c) => {
            state.dialogs.group_rename_buf.push(c);
            Action::Render
        }
        _ => Action::None,
    }
}

pub(super) fn handle_group_create(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.dialogs.group_creating = false;
            state.dialogs.group_rename_buf.clear();
            Action::Render
        }
        KeyCode::Enter => {
            let name = state.dialogs.group_rename_buf.trim().to_string();
            state.dialogs.group_creating = false;
            state.dialogs.group_rename_buf.clear();
            if !name.is_empty() {
                // Check if group already exists
                let exists = state
                    .sidebar.tree
                    .iter()
                    .any(|n| matches!(n, TreeNode::Group { name: gn, .. } if gn == &name));
                if exists {
                    state.status_message = format!("Group '{name}' already exists");
                } else {
                    // Insert new empty group at the end of the tree
                    state.sidebar.tree.push(TreeNode::Group {
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
            state.dialogs.group_rename_buf.pop();
            Action::Render
        }
        KeyCode::Char(c) => {
            state.dialogs.group_rename_buf.push(c);
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Object Filter ---

pub(super) fn handle_object_filter(state: &mut AppState, key: KeyEvent) -> Action {
    if state.sidebar.object_filter.search_active {
        return handle_object_filter_search(state, key);
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            state.overlay = None;
            Action::SaveSchemaFilter
        }
        KeyCode::Char('j') | KeyCode::Down => {
            state.sidebar.object_filter.move_down();
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.sidebar.object_filter.move_up();
            Action::Render
        }
        KeyCode::Char('g') => {
            state.sidebar.object_filter.go_top();
            Action::Render
        }
        KeyCode::Char('G') => {
            state.sidebar.object_filter.go_bottom();
            Action::Render
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = state.sidebar.object_filter.visible_height / 2;
            let count = state.sidebar.object_filter.display_list().len();
            state.sidebar.object_filter.cursor =
                (state.sidebar.object_filter.cursor + half).min(count.saturating_sub(1));
            state.sidebar.object_filter.offset = state
                .sidebar.object_filter
                .cursor
                .saturating_sub(state.sidebar.object_filter.visible_height / 2);
            Action::Render
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = state.sidebar.object_filter.visible_height / 2;
            state.sidebar.object_filter.cursor = state.sidebar.object_filter.cursor.saturating_sub(half);
            state.sidebar.object_filter.offset = state
                .sidebar.object_filter
                .cursor
                .saturating_sub(state.sidebar.object_filter.visible_height / 2);
            Action::Render
        }
        KeyCode::Char(' ') => {
            state.sidebar.object_filter.toggle_at_cursor();
            Action::SaveSchemaFilter
        }
        KeyCode::Char('a') => {
            state.sidebar.object_filter.select_all();
            Action::SaveSchemaFilter
        }
        KeyCode::Char('/') => {
            state.sidebar.object_filter.search_active = true;
            state.sidebar.object_filter.search_query.clear();
            state.sidebar.object_filter.cursor = 0;
            state.sidebar.object_filter.offset = 0;
            Action::Render
        }
        KeyCode::Enter => {
            state.overlay = None;
            Action::SaveSchemaFilter
        }
        _ => Action::None,
    }
}

pub(super) fn handle_object_filter_search(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.sidebar.object_filter.search_active = false;
            state.sidebar.object_filter.search_query.clear();
            state.sidebar.object_filter.cursor = 0;
            state.sidebar.object_filter.offset = 0;
            Action::Render
        }
        KeyCode::Enter => {
            state.sidebar.object_filter.search_active = false;
            Action::Render
        }
        KeyCode::Backspace => {
            state.sidebar.object_filter.search_query.pop();
            state.sidebar.object_filter.cursor = 0;
            state.sidebar.object_filter.offset = 0;
            Action::Render
        }
        KeyCode::Char(c) => {
            state.sidebar.object_filter.search_query.push(c);
            state.sidebar.object_filter.cursor = 0;
            state.sidebar.object_filter.offset = 0;
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Connection Dialog ---

pub(super) fn handle_connection_dialog(state: &mut AppState, key: KeyEvent) -> Action {
    if state.dialogs.connection_form.show_saved_list {
        return handle_saved_connections_list(state, key);
    }

    if state.dialogs.connection_form.read_only {
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
            state.dialogs.connection_form.next_field();
            Action::Render
        }
        KeyCode::BackTab => {
            state.dialogs.connection_form.prev_field();
            Action::Render
        }
        KeyCode::Enter => {
            if state.dialogs.connection_form.name.is_empty() {
                state.dialogs.connection_form.error_message = "Name is required".to_string();
                return Action::Render;
            }
            if state.dialogs.connection_form.host.is_empty() {
                state.dialogs.connection_form.error_message = "Host is required".to_string();
                return Action::Render;
            }
            if state.dialogs.connection_form.username.is_empty() {
                state.dialogs.connection_form.error_message = "Username is required".to_string();
                return Action::Render;
            }
            state.dialogs.connection_form.error_message.clear();
            state.dialogs.connection_form.connecting = true;
            Action::Connect
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.dialogs.connection_form.password_visible = !state.dialogs.connection_form.password_visible;
            Action::Render
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.dialogs.connection_form.cycle_db_type();
            Action::Render
        }
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.dialogs.connection_form.cycle_group();
            Action::Render
        }
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Action::SaveConnection
        }
        KeyCode::Char(c) => {
            // Fields 1 (Type) and 7 (Group) are selectors, not text input
            if state.dialogs.connection_form.selected_field == 1
                || state.dialogs.connection_form.selected_field == 7
            {
                return Action::None;
            }
            state.dialogs.connection_form.active_field_mut().push(c);
            state.dialogs.connection_form.error_message.clear();
            Action::Render
        }
        KeyCode::Backspace => {
            if state.dialogs.connection_form.selected_field != 1
                && state.dialogs.connection_form.selected_field != 7
            {
                state.dialogs.connection_form.active_field_mut().pop();
            }
            Action::Render
        }
        _ => Action::None,
    }
}

pub(super) fn handle_saved_connections_list(state: &mut AppState, key: KeyEvent) -> Action {
    let count = state.dialogs.saved_connections.len();
    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            Action::Render
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if count > 0 {
                state.dialogs.connection_form.saved_cursor =
                    (state.dialogs.connection_form.saved_cursor + 1) % (count + 1);
            }
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if state.dialogs.connection_form.saved_cursor == 0 {
                state.dialogs.connection_form.saved_cursor = count;
            } else {
                state.dialogs.connection_form.saved_cursor -= 1;
            }
            Action::Render
        }
        KeyCode::Enter => {
            let cursor = state.dialogs.connection_form.saved_cursor;
            if cursor < count {
                let config = state.dialogs.saved_connections[cursor].clone();
                let groups = state.available_groups();
                state.dialogs.connection_form = crate::ui::state::ConnectionFormState::from_config(&config);
                state.dialogs.connection_form.group_options = groups;
                state.dialogs.connection_form.connecting = true;
                Action::Connect
            } else {
                state.dialogs.connection_form.show_saved_list = false;
                Action::Render
            }
        }
        KeyCode::Char('n') => {
            state.dialogs.connection_form.show_saved_list = false;
            Action::Render
        }
        KeyCode::Char('d') => {
            let cursor = state.dialogs.connection_form.saved_cursor;
            if cursor < count {
                let name = state.dialogs.saved_connections[cursor].name.clone();
                state.dialogs.saved_connections.remove(cursor);
                if let Ok(store) = crate::core::storage::ConnectionStore::new() {
                    let _ = store.save(&state.dialogs.saved_connections, "");
                }
                state.status_message = format!("Connection '{name}' deleted");
                if state.dialogs.connection_form.saved_cursor >= state.dialogs.saved_connections.len()
                    && state.dialogs.connection_form.saved_cursor > 0
                {
                    state.dialogs.connection_form.saved_cursor -= 1;
                }
                if state.dialogs.saved_connections.is_empty() {
                    state.dialogs.connection_form.show_saved_list = false;
                }
            }
            Action::Render
        }
        _ => Action::None,
    }
}

pub(super) fn handle_conn_menu(state: &mut AppState, key: KeyEvent) -> Action {
    use crate::ui::state::ConnMenuAction;

    let actions = ConnMenuAction::all();
    let count = actions.len();

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            state.overlay = None;
            Action::Render
        }
        KeyCode::Char('j') | KeyCode::Down => {
            state.dialogs.conn_menu.cursor = (state.dialogs.conn_menu.cursor + 1) % count;
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.dialogs.conn_menu.cursor = if state.dialogs.conn_menu.cursor == 0 {
                count - 1
            } else {
                state.dialogs.conn_menu.cursor - 1
            };
            Action::Render
        }
        KeyCode::Enter => {
            let selected = &actions[state.dialogs.conn_menu.cursor];
            let name = state.dialogs.conn_menu.conn_name.clone();
            state.overlay = None;

            match selected {
                ConnMenuAction::View => {
                    if let Some(config) = state.dialogs.saved_connections.iter().find(|c| c.name == name) {
                        let groups = state.available_groups();
                        let mut form = crate::ui::state::ConnectionFormState::from_config(config);
                        form.password = "********".to_string();
                        form.password_visible = false;
                        form.read_only = true;
                        form.group_options = groups;
                        state.dialogs.connection_form = form;
                        state.overlay = Some(Overlay::ConnectionDialog);
                    }
                    Action::Render
                }
                ConnMenuAction::Edit => {
                    if let Some(config) = state.dialogs.saved_connections.iter().find(|c| c.name == name) {
                        let groups = state.available_groups();
                        state.dialogs.connection_form =
                            crate::ui::state::ConnectionFormState::for_edit(config);
                        state.dialogs.connection_form.group_options = groups;
                        state.overlay = Some(Overlay::ConnectionDialog);
                    }
                    Action::Render
                }
                ConnMenuAction::Connect => Action::ConnectByName { name },
                ConnMenuAction::Disconnect => Action::DisconnectByName { name },
                ConnMenuAction::Restart => Action::ConnectByName { name },
                ConnMenuAction::Delete => {
                    state.overlay = Some(Overlay::ConfirmDeleteConnection { name });
                    Action::Render
                }
            }
        }
        _ => Action::None,
    }
}

pub(super) fn handle_group_menu(state: &mut AppState, key: KeyEvent) -> Action {
    use crate::ui::state::GroupMenuAction;

    let actions = GroupMenuAction::all();
    let count = actions.len();

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            state.overlay = None;
            Action::Render
        }
        KeyCode::Char('j') | KeyCode::Down => {
            state.dialogs.group_menu.cursor = (state.dialogs.group_menu.cursor + 1) % count;
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.dialogs.group_menu.cursor = if state.dialogs.group_menu.cursor == 0 {
                count - 1
            } else {
                state.dialogs.group_menu.cursor - 1
            };
            Action::Render
        }
        KeyCode::Enter => {
            let selected_idx = state.dialogs.group_menu.cursor;
            let group_name = state.dialogs.group_menu.group_name.clone();
            let is_empty = state.dialogs.group_menu.is_empty;
            state.overlay = None;

            match &actions[selected_idx] {
                GroupMenuAction::Rename => {
                    state.dialogs.group_renaming = Some(group_name.clone());
                    state.dialogs.group_rename_buf = group_name;
                    Action::Render
                }
                GroupMenuAction::Delete => {
                    if !is_empty {
                        state.status_message = "Cannot delete group with connections".to_string();
                        return Action::Render;
                    }
                    // Remove the empty group node from tree
                    state.sidebar.tree.retain(
                        |n| !matches!(n, TreeNode::Group { name, .. } if name == &group_name),
                    );
                    state.status_message = format!("Group '{group_name}' deleted");
                    // Persist groups
                    if let Ok(store) = crate::core::storage::ConnectionStore::new() {
                        let _ = store.save_groups(&persist_group_names(state));
                    }
                    Action::Render
                }
                GroupMenuAction::NewGroup => {
                    state.dialogs.group_creating = true;
                    state.dialogs.group_rename_buf.clear();
                    Action::Render
                }
            }
        }
        _ => Action::None,
    }
}

pub(super) fn handle_help_overlay(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
            state.overlay = None;
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Script Connection Picker ---

pub(super) fn handle_script_conn_picker(state: &mut AppState, key: KeyEvent) -> Action {
    use crate::ui::state::PickerItem;

    let picker = match &mut state.dialogs.script_conn_picker {
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
            state.dialogs.script_conn_picker = None;
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
                    state.dialogs.script_conn_picker = None;
                    Action::SetScriptConnection { conn_name }
                }
                Some(PickerItem::OthersHeader) => {
                    // Toggle expand/collapse
                    picker.others_expanded = !picker.others_expanded;
                    Action::Render
                }
                None => {
                    state.overlay = None;
                    state.dialogs.script_conn_picker = None;
                    Action::Render
                }
            }
        }
        _ => Action::None,
    }
}

// --- Theme Picker ---

pub(super) fn handle_theme_picker(state: &mut AppState, key: KeyEvent) -> Action {
    use crate::ui::theme::THEME_NAMES;

    let count = THEME_NAMES.len();
    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            Action::Render
        }
        KeyCode::Char('j') | KeyCode::Down => {
            state.dialogs.theme_picker.cursor = (state.dialogs.theme_picker.cursor + 1).min(count - 1);
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.dialogs.theme_picker.cursor = state.dialogs.theme_picker.cursor.saturating_sub(1);
            Action::Render
        }
        KeyCode::Enter => {
            let name = THEME_NAMES[state.dialogs.theme_picker.cursor].to_string();
            state.overlay = None;
            Action::SetTheme { name }
        }
        _ => Action::None,
    }
}

// --- Bind Variables ---

/// Extract bind variable names (`:name` patterns) from a SQL query.
/// Skips string literals and comments. Returns unique names in order.
pub(super) fn extract_bind_variables(query: &str) -> Vec<String> {
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
pub(super) fn maybe_prompt_bind_vars(
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

        state.dialogs.bind_variables = Some(crate::ui::state::BindVariablesState {
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
pub(super) fn handle_bind_variables(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.dialogs.bind_variables = None;
            state.overlay = None;
            Action::Render
        }
        KeyCode::Tab => {
            if let Some(ref mut bv) = state.dialogs.bind_variables {
                bv.next_field();
            }
            Action::Render
        }
        KeyCode::BackTab => {
            if let Some(ref mut bv) = state.dialogs.bind_variables {
                bv.prev_field();
            }
            Action::Render
        }
        KeyCode::Enter => {
            if let Some(bv) = state.dialogs.bind_variables.take() {
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
            if let Some(ref mut bv) = state.dialogs.bind_variables {
                let idx = bv.selected_idx;
                bv.variables[idx].1.pop();
            }
            Action::Render
        }
        KeyCode::Char(c) => {
            if let Some(ref mut bv) = state.dialogs.bind_variables {
                let idx = bv.selected_idx;
                bv.variables[idx].1.push(c);
            }
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Export Dialog ---

pub(super) fn handle_export_dialog(state: &mut AppState, key: KeyEvent) -> Action {
    let dialog = match state.dialogs.export_dialog.as_mut() {
        Some(d) => d,
        None => {
            state.overlay = None;
            return Action::Render;
        }
    };
    dialog.error = None;

    match key.code {
        KeyCode::Esc => {
            state.dialogs.export_dialog = None;
            state.overlay = None;
            Action::Render
        }
        KeyCode::Tab => {
            if dialog.focused == ExportField::Path
                && !dialog.path.is_empty()
                && !dialog.path.ends_with(' ')
            {
                dialog.complete_path();
            } else {
                dialog.next_field();
            }
            Action::Render
        }
        KeyCode::Down => {
            dialog.next_field();
            Action::Render
        }
        KeyCode::BackTab | KeyCode::Up => {
            dialog.prev_field();
            Action::Render
        }
        KeyCode::Enter => {
            // Validate
            let d = state.dialogs.export_dialog.as_ref().unwrap();
            if d.password.is_empty() {
                state.dialogs.export_dialog.as_mut().unwrap().error =
                    Some("Password is required".to_string());
                return Action::Render;
            }
            if d.password != d.confirm {
                state.dialogs.export_dialog.as_mut().unwrap().error =
                    Some("Passwords do not match".to_string());
                return Action::Render;
            }
            if d.path.is_empty() {
                state.dialogs.export_dialog.as_mut().unwrap().error = Some("Path is required".to_string());
                return Action::Render;
            }
            state.overlay = None;
            Action::ExportBundle
        }
        KeyCode::Char(' ')
            if dialog.focused == ExportField::IncludeCredentials
                || dialog.focused == ExportField::ShowPassword =>
        {
            if dialog.focused == ExportField::IncludeCredentials {
                dialog.include_credentials = !dialog.include_credentials;
            } else {
                dialog.show_password = !dialog.show_password;
            }
            Action::Render
        }
        KeyCode::Char(c) => {
            match dialog.focused {
                ExportField::Path => {
                    dialog.path.push(c);
                    dialog.reset_completions();
                }
                ExportField::Password => dialog.password.push(c),
                ExportField::Confirm => dialog.confirm.push(c),
                ExportField::ShowPassword => {
                    if c == 'y' || c == 'Y' {
                        dialog.show_password = true;
                    } else if c == 'n' || c == 'N' {
                        dialog.show_password = false;
                    }
                }
                ExportField::IncludeCredentials => {
                    if c == 'y' || c == 'Y' {
                        dialog.include_credentials = true;
                    } else if c == 'n' || c == 'N' {
                        dialog.include_credentials = false;
                    }
                }
            }
            Action::Render
        }
        KeyCode::Backspace => {
            match dialog.focused {
                ExportField::Path => {
                    dialog.path.pop();
                    dialog.reset_completions();
                }
                ExportField::Password => {
                    dialog.password.pop();
                }
                ExportField::Confirm => {
                    dialog.confirm.pop();
                }
                ExportField::IncludeCredentials | ExportField::ShowPassword => {}
            }
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Import Dialog ---

pub(super) fn handle_import_dialog(state: &mut AppState, key: KeyEvent) -> Action {
    let dialog = match state.dialogs.import_dialog.as_mut() {
        Some(d) => d,
        None => {
            state.overlay = None;
            return Action::Render;
        }
    };
    dialog.error = None;

    match key.code {
        KeyCode::Esc => {
            state.dialogs.import_dialog = None;
            state.overlay = None;
            Action::Render
        }
        KeyCode::Tab => {
            if dialog.focused == ImportField::Path
                && !dialog.path.is_empty()
                && !dialog.path.ends_with(' ')
            {
                dialog.complete_path();
            } else {
                dialog.next_field();
            }
            Action::Render
        }
        KeyCode::Down => {
            dialog.next_field();
            Action::Render
        }
        KeyCode::BackTab | KeyCode::Up => {
            dialog.next_field();
            Action::Render
        }
        KeyCode::Enter => {
            let d = state.dialogs.import_dialog.as_ref().unwrap();
            if d.path.is_empty() {
                state.dialogs.import_dialog.as_mut().unwrap().error =
                    Some("File path is required".to_string());
                return Action::Render;
            }
            if d.password.is_empty() {
                state.dialogs.import_dialog.as_mut().unwrap().error =
                    Some("Password is required".to_string());
                return Action::Render;
            }
            state.overlay = None;
            Action::ImportBundle
        }
        KeyCode::Char(' ') if dialog.focused == ImportField::ShowPassword => {
            dialog.show_password = !dialog.show_password;
            Action::Render
        }
        KeyCode::Char(c) => {
            match dialog.focused {
                ImportField::Path => {
                    dialog.path.push(c);
                    dialog.reset_completions();
                }
                ImportField::Password => dialog.password.push(c),
                ImportField::ShowPassword => {
                    if c == 'y' || c == 'Y' {
                        dialog.show_password = true;
                    } else if c == 'n' || c == 'N' {
                        dialog.show_password = false;
                    }
                }
            }
            Action::Render
        }
        KeyCode::Backspace => {
            match dialog.focused {
                ImportField::Path => {
                    dialog.path.pop();
                    dialog.reset_completions();
                }
                ImportField::Password => {
                    dialog.password.pop();
                }
                ImportField::ShowPassword => {}
            }
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Helper ---

pub(super) fn persist_group_names(state: &AppState) -> Vec<String> {
    state
        .sidebar.tree
        .iter()
        .filter_map(|n| {
            if let TreeNode::Group { name, .. } = n {
                return Some(name.clone());
            }
            None
        })
        .collect()
}
