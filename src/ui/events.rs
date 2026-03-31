use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;

use crate::ui::state::{AppState, CenterTab, LeafKind, Mode, Overlay, Panel, TreeNode};

pub enum Action {
    Quit,
    Render,
    None,
    LoadSchemas { conn_name: String },
    SaveSchemaFilter,
    LoadChildren { schema: String, kind: String },
    LoadTableData { schema: String, table: String },
    LoadColumns { schema: String, table: String },
    LoadPackageContent { schema: String, name: String },
    ExecuteQuery(String),
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
            _ => Action::None,
        };
    }

    // Handle sidebar search mode
    if state.tree_state.search_active {
        return handle_sidebar_search(state, key);
    }

    // Global keys (Normal mode only)
    if state.mode == Mode::Normal {
        match key.code {
            KeyCode::Char('q') => {
                if state.show_editor && state.active_panel == Panel::QueryEditor {
                    state.show_editor = false;
                    state.active_panel = Panel::Sidebar;
                    return Action::Render;
                }
                return Action::Quit;
            }
            KeyCode::Char('?') => {
                state.overlay = Some(Overlay::Help);
                return Action::Render;
            }
            KeyCode::Char('a') => {
                state.overlay = Some(Overlay::ConnectionDialog);
                state.connection_form = crate::ui::state::ConnectionFormState::new();
                return Action::Render;
            }
            KeyCode::Char('e') => {
                state.show_editor = !state.show_editor;
                if state.show_editor {
                    state.active_panel = Panel::QueryEditor;
                } else {
                    state.active_panel = Panel::Sidebar;
                }
                return Action::Render;
            }
            KeyCode::Char('F') => {
                // Open context-sensitive filter based on current tree position
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
                            // Go up to parent category
                            let cat_key = match kind {
                                LeafKind::Table => format!("{schema}.Tables"),
                                LeafKind::View => format!("{schema}.Views"),
                                LeafKind::Package => format!("{schema}.Packages"),
                                LeafKind::Procedure => format!("{schema}.Procedures"),
                                LeafKind::Function => format!("{schema}.Functions"),
                            };
                            // Find the category node
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
                    // Default: schema filter
                    let schemas = state.all_schema_names();
                    if !schemas.is_empty() {
                        state.object_filter.open_for("schemas", schemas);
                        state.overlay = Some(Overlay::ObjectFilter);
                    }
                }
                return Action::Render;
            }
            _ => {}
        }
    }

    // Panel switching with Ctrl
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => {
                state.active_panel = Panel::Sidebar;
                return Action::Render;
            }
            KeyCode::Char('l') | KeyCode::Right => {
                state.active_panel = Panel::DataGrid;
                return Action::Render;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if state.show_editor {
                    state.active_panel = Panel::QueryEditor;
                }
                return Action::Render;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.active_panel = Panel::DataGrid;
                return Action::Render;
            }
            _ => {}
        }
    }

    match state.active_panel {
        Panel::Sidebar => handle_sidebar(state, key),
        Panel::DataGrid => handle_data_grid(state, key),
        Panel::Properties => handle_properties(state, key),
        Panel::PackageView => handle_package_view(state, key),
        Panel::QueryEditor => handle_editor(state, key),
    }
}

// --- Schema Filter ---

fn handle_object_filter(state: &mut AppState, key: KeyEvent) -> Action {
    // Search mode inside schema filter
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
            // Keep search results visible but exit search input mode
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
            // Jump to current match and exit search
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
    // Saved connections list mode
    if state.connection_form.show_saved_list {
        return handle_saved_connections_list(state, key);
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
            // Save connection without connecting
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
                    (state.connection_form.saved_cursor + 1) % (count + 1); // +1 for "New" option
            }
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if state.connection_form.saved_cursor == 0 {
                state.connection_form.saved_cursor = count; // wrap to "New"
            } else {
                state.connection_form.saved_cursor -= 1;
            }
            Action::Render
        }
        KeyCode::Enter => {
            let cursor = state.connection_form.saved_cursor;
            if cursor < count {
                // Load selected connection into form
                let config = state.saved_connections[cursor].clone();
                state.connection_form =
                    crate::ui::state::ConnectionFormState::from_config(&config);
                // Connect immediately
                state.connection_form.connecting = true;
                Action::Connect
            } else {
                // "New connection" selected
                state.connection_form.show_saved_list = false;
                Action::Render
            }
        }
        KeyCode::Char('n') => {
            // New connection shortcut
            state.connection_form.show_saved_list = false;
            Action::Render
        }
        KeyCode::Char('d') => {
            // Delete saved connection
            let cursor = state.connection_form.saved_cursor;
            if cursor < count {
                let name = state.saved_connections[cursor].name.clone();
                state.saved_connections.remove(cursor);
                // Persist deletion
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
                    // Open form in read-only mode (password masked)
                    if let Some(config) = state
                        .saved_connections
                        .iter()
                        .find(|c| c.name == name)
                    {
                        let mut form =
                            crate::ui::state::ConnectionFormState::from_config(config);
                        form.password = "********".to_string();
                        form.password_visible = false;
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
                            crate::ui::state::ConnectionFormState::from_config(config);
                        state.overlay = Some(Overlay::ConnectionDialog);
                    }
                    Action::Render
                }
                ConnMenuAction::Connect => Action::ConnectByName { name },
                ConnMenuAction::Disconnect => Action::DisconnectByName { name },
                ConnMenuAction::Restart => {
                    // Disconnect + reconnect
                    Action::ConnectByName { name } // app.rs will handle disconnect first
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
    // Collect search data without borrowing tree_state
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
            // Open connection menu if on a Connection node
            if let Some(idx) = state.selected_tree_index() {
                // Walk up to find Connection
                let mut walk = idx;
                loop {
                    if let TreeNode::Connection { name, .. } = &state.tree[walk] {
                        let conn_name = name.clone();
                        state.conn_menu.conn_name = conn_name;
                        state.conn_menu.cursor = 0;
                        state.conn_menu.is_connected = state.connected
                            && state.connection_name.as_deref() == Some(name);
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
        KeyCode::Tab => {
            cycle_tab(state);
            Action::Render
        }
        _ => Action::None,
    }
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
            state.current_schema = Some(schema.clone());
            state.active_panel = Panel::DataGrid;
            state.active_tab = CenterTab::Data;
            Action::LoadTableData { schema, table }
        }
        TreeNode::Leaf {
            schema,
            name,
            kind: LeafKind::Package,
            ..
        } => {
            let schema = schema.clone();
            let pkg_name = name.clone();
            state.active_panel = Panel::PackageView;
            state.active_tab = CenterTab::Declaration;
            Action::LoadPackageContent {
                schema,
                name: pkg_name,
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

// --- Data Grid ---

fn handle_data_grid(state: &mut AppState, key: KeyEvent) -> Action {
    let row_count = state
        .query_result
        .as_ref()
        .map(|r| r.rows.len())
        .unwrap_or(0);
    let vh = state.grid_visible_height.max(1);

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if state.grid_selected_row + 1 < row_count {
                state.grid_selected_row += 1;
                if state.grid_selected_row >= state.grid_scroll_row + vh {
                    state.grid_scroll_row = state.grid_selected_row - vh + 1;
                }
            }
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if state.grid_selected_row > 0 {
                state.grid_selected_row -= 1;
                if state.grid_selected_row < state.grid_scroll_row {
                    state.grid_scroll_row = state.grid_selected_row;
                }
            }
            Action::Render
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = vh / 2;
            state.grid_selected_row =
                (state.grid_selected_row + half).min(row_count.saturating_sub(1));
            state.grid_scroll_row = state.grid_selected_row.saturating_sub(vh / 2);
            Action::Render
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = vh / 2;
            state.grid_selected_row = state.grid_selected_row.saturating_sub(half);
            state.grid_scroll_row = state.grid_selected_row.saturating_sub(vh / 2);
            Action::Render
        }
        KeyCode::Char('g') => {
            state.grid_selected_row = 0;
            state.grid_scroll_row = 0;
            Action::Render
        }
        KeyCode::Char('G') => {
            if row_count > 0 {
                state.grid_selected_row = row_count - 1;
                state.grid_scroll_row = row_count.saturating_sub(vh);
            }
            Action::Render
        }
        KeyCode::Tab => {
            cycle_tab(state);
            Action::Render
        }
        _ => Action::None,
    }
}

fn handle_properties(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Tab => {
            cycle_tab(state);
            Action::Render
        }
        _ => Action::None,
    }
}

fn handle_package_view(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Tab => {
            state.active_tab = match state.active_tab {
                CenterTab::Declaration => CenterTab::Body,
                CenterTab::Body => CenterTab::Declaration,
                _ => CenterTab::Declaration,
            };
            Action::Render
        }
        _ => Action::None,
    }
}

// --- Query Editor ---

fn handle_editor(state: &mut AppState, key: KeyEvent) -> Action {
    match state.mode {
        Mode::Normal => match key.code {
            KeyCode::Char('i') => {
                state.mode = Mode::Insert;
                Action::Render
            }
            KeyCode::Char('a') => {
                state.mode = Mode::Insert;
                state.editor_cursor_col += 1;
                Action::Render
            }
            KeyCode::Char('o') => {
                state.mode = Mode::Insert;
                state.editor_content.push('\n');
                state.editor_cursor_row += 1;
                state.editor_cursor_col = 0;
                Action::Render
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.editor_content.clear();
                state.editor_cursor_row = 0;
                state.editor_cursor_col = 0;
                Action::Render
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let query = state.editor_content.clone();
                if !query.trim().is_empty() {
                    state.status_message = "Executing query...".to_string();
                    Action::ExecuteQuery(query)
                } else {
                    Action::None
                }
            }
            _ => Action::None,
        },
        Mode::Insert => match key.code {
            KeyCode::Esc => {
                state.mode = Mode::Normal;
                Action::Render
            }
            KeyCode::Char(c) => {
                let lines: Vec<&str> = state.editor_content.split('\n').collect();
                let row = state.editor_cursor_row.min(lines.len().saturating_sub(1));
                let col = state
                    .editor_cursor_col
                    .min(lines.get(row).map(|l| l.len()).unwrap_or(0));
                let mut byte_pos = 0;
                for (i, line) in lines.iter().enumerate() {
                    if i == row {
                        byte_pos += col;
                        break;
                    }
                    byte_pos += line.len() + 1;
                }
                byte_pos = byte_pos.min(state.editor_content.len());
                state.editor_content.insert(byte_pos, c);
                state.editor_cursor_col += 1;
                Action::Render
            }
            KeyCode::Backspace => {
                let lines: Vec<&str> = state.editor_content.split('\n').collect();
                let row = state.editor_cursor_row.min(lines.len().saturating_sub(1));
                let col = state
                    .editor_cursor_col
                    .min(lines.get(row).map(|l| l.len()).unwrap_or(0));
                if col > 0 {
                    let mut byte_pos = 0;
                    for (i, line) in lines.iter().enumerate() {
                        if i == row {
                            byte_pos += col - 1;
                            break;
                        }
                        byte_pos += line.len() + 1;
                    }
                    if byte_pos < state.editor_content.len() {
                        state.editor_content.remove(byte_pos);
                        state.editor_cursor_col -= 1;
                    }
                } else if row > 0 {
                    let prev_line_len = lines[row - 1].len();
                    let mut byte_pos = 0;
                    for (i, line) in lines.iter().enumerate() {
                        if i == row {
                            break;
                        }
                        byte_pos += line.len() + 1;
                    }
                    if byte_pos > 0 {
                        state.editor_content.remove(byte_pos - 1);
                        state.editor_cursor_row -= 1;
                        state.editor_cursor_col = prev_line_len;
                    }
                }
                Action::Render
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.mode = Mode::Normal;
                let query = state.editor_content.clone();
                if !query.trim().is_empty() {
                    state.status_message = "Executing query...".to_string();
                    Action::ExecuteQuery(query)
                } else {
                    Action::None
                }
            }
            KeyCode::Enter => {
                let lines: Vec<&str> = state.editor_content.split('\n').collect();
                let row = state.editor_cursor_row.min(lines.len().saturating_sub(1));
                let col = state
                    .editor_cursor_col
                    .min(lines.get(row).map(|l| l.len()).unwrap_or(0));
                let mut byte_pos = 0;
                for (i, line) in lines.iter().enumerate() {
                    if i == row {
                        byte_pos += col;
                        break;
                    }
                    byte_pos += line.len() + 1;
                }
                byte_pos = byte_pos.min(state.editor_content.len());
                state.editor_content.insert(byte_pos, '\n');
                state.editor_cursor_row += 1;
                state.editor_cursor_col = 0;
                Action::Render
            }
            _ => Action::None,
        },
    }
}

fn cycle_tab(state: &mut AppState) {
    state.active_tab = match state.active_tab {
        CenterTab::Data => CenterTab::Properties,
        CenterTab::Properties => CenterTab::Data,
        CenterTab::Declaration => CenterTab::Body,
        CenterTab::Body => CenterTab::Declaration,
        CenterTab::DDL => CenterTab::Data,
    };
    state.active_panel = match state.active_tab {
        CenterTab::Data => Panel::DataGrid,
        CenterTab::Properties => Panel::Properties,
        CenterTab::Declaration | CenterTab::Body => Panel::PackageView,
        CenterTab::DDL => Panel::DataGrid,
    };
}
