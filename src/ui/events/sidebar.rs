use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::core::models::DatabaseType;
use crate::ui::state::{AppState, LeafKind, Overlay, TreeNode};
use crate::ui::tabs::TabKind;

use super::overlays::{handle_group_create, handle_group_rename};
use super::Action;

pub(super) fn handle_filter_key(state: &mut AppState) -> Action {
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
            TreeNode::Empty => {}
            TreeNode::Leaf { schema, kind, .. } => {
                let base_key = match kind {
                    LeafKind::Table => format!("{schema}.Tables"),
                    LeafKind::View => format!("{schema}.Views"),
                    LeafKind::MaterializedView => format!("{schema}.MaterializedViews"),
                    LeafKind::Index => format!("{schema}.Indexes"),
                    LeafKind::Sequence => format!("{schema}.Sequences"),
                    LeafKind::Type => format!("{schema}.Types"),
                    LeafKind::Trigger => format!("{schema}.Triggers"),
                    LeafKind::Package => format!("{schema}.Packages"),
                    LeafKind::Procedure => format!("{schema}.Procedures"),
                    LeafKind::Function => format!("{schema}.Functions"),
                    LeafKind::Event => format!("{schema}.Events"),
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

// --- Sidebar Search ---

pub(super) fn handle_sidebar_search(state: &mut AppState, key: KeyEvent) -> Action {
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

pub(super) fn update_search_and_jump(state: &mut AppState) {
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

pub(super) fn handle_sidebar(state: &mut AppState, key: KeyEvent) -> Action {
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
        KeyCode::Char('h') | KeyCode::Left => {
            if let Some(idx) = state.selected_tree_index() {
                if state.tree[idx].is_expanded() {
                    // Collapse current node
                    state.tree[idx].toggle_expand();
                } else {
                    // Navigate to parent and collapse it
                    let child_depth = state.tree[idx].depth();
                    if child_depth > 0 {
                        let mut walk = idx;
                        while walk > 0 {
                            walk -= 1;
                            if state.tree[walk].depth() < child_depth {
                                // Found parent — collapse it and move cursor there
                                if state.tree[walk].is_expanded() {
                                    state.tree[walk].toggle_expand();
                                }
                                // Move cursor to parent in visible tree
                                let vis_info = {
                                    let visible = state.visible_tree();
                                    visible
                                        .iter()
                                        .position(|(vi, _)| *vi == walk)
                                        .map(|p| (p, visible.len()))
                                };
                                if let Some((vis_pos, vis_len)) = vis_info {
                                    state.tree_state.cursor = vis_pos;
                                    state.tree_state.adjust_scroll(vis_len);
                                }
                                break;
                            }
                        }
                    }
                }
            }
            Action::Render
        }
        KeyCode::Char('d') => {
            if state.tree_state.pending_d {
                state.tree_state.pending_d = false;
                if let Some(idx) = state.selected_tree_index() {
                    match &state.tree[idx] {
                        TreeNode::Connection { name, .. } => {
                            return Action::DeleteConnection { name: name.clone() };
                        }
                        TreeNode::Leaf {
                            name, schema, kind, ..
                        } if matches!(
                            kind,
                            LeafKind::Table | LeafKind::View | LeafKind::Package
                        ) =>
                        {
                            let obj_type = match kind {
                                LeafKind::Table => "TABLE",
                                LeafKind::View => "VIEW",
                                LeafKind::Package => "PACKAGE",
                                _ => unreachable!(),
                            };
                            let conn_name = find_conn_name_for(state, idx);
                            state.sidebar_pending_action =
                                Some(crate::ui::state::PendingObjectAction {
                                    schema: schema.clone(),
                                    name: name.clone(),
                                    obj_type: obj_type.to_string(),
                                    conn_name,
                                });
                            state.overlay = Some(Overlay::ConfirmDropObject);
                            return Action::Render;
                        }
                        _ => {}
                    }
                }
                Action::Render
            } else {
                state.tree_state.pending_d = true;
                Action::Render
            }
        }
        KeyCode::Char('r') => {
            // r → rename object or connection
            if let Some(idx) = state.selected_tree_index() {
                match &state.tree[idx] {
                    TreeNode::Connection { name, .. } => {
                        state.sidebar_rename_buf = name.clone();
                        state.sidebar_pending_action =
                            Some(crate::ui::state::PendingObjectAction {
                                schema: String::new(),
                                name: name.clone(),
                                obj_type: "CONNECTION".to_string(),
                                conn_name: name.clone(),
                            });
                        state.overlay = Some(Overlay::RenameObject);
                        return Action::Render;
                    }
                    TreeNode::Leaf {
                        name, schema, kind, ..
                    } if matches!(kind, LeafKind::Table | LeafKind::View) => {
                        let obj_type = match kind {
                            LeafKind::Table => "TABLE",
                            LeafKind::View => "VIEW",
                            _ => unreachable!(),
                        };
                        let conn_name = find_conn_name_for(state, idx);
                        state.sidebar_rename_buf = name.clone();
                        state.sidebar_pending_action =
                            Some(crate::ui::state::PendingObjectAction {
                                schema: schema.clone(),
                                name: name.clone(),
                                obj_type: obj_type.to_string(),
                                conn_name,
                            });
                        state.overlay = Some(Overlay::RenameObject);
                        return Action::Render;
                    }
                    _ => {}
                }
            }
            Action::Render
        }
        KeyCode::Char('y') => {
            // yy → yank connection for duplicate
            if let Some(idx) = state.selected_tree_index() {
                let mut walk = idx;
                loop {
                    if let TreeNode::Connection { name, .. } = &state.tree[walk] {
                        state.sidebar_yank_conn = Some(name.clone());
                        state.status_message = format!("Yanked connection: {name}");
                        break;
                    }
                    if walk == 0 {
                        break;
                    }
                    walk -= 1;
                }
            }
            Action::Render
        }
        KeyCode::Char('p') => {
            // p → paste (duplicate) yanked connection into current group
            if let Some(ref source) = state.sidebar_yank_conn.clone() {
                // Find group at cursor position by walking up
                let group = if let Some(idx) = state.selected_tree_index() {
                    let mut walk = idx;
                    loop {
                        if let TreeNode::Group { name, .. } = &state.tree[walk] {
                            break name.clone();
                        }
                        if walk == 0 {
                            break "Default".to_string();
                        }
                        walk -= 1;
                    }
                } else {
                    "Default".to_string()
                };
                return Action::DuplicateConnection {
                    source_name: source.clone(),
                    target_group: group,
                };
            }
            Action::Render
        }
        KeyCode::Char('o') | KeyCode::Char('i') => {
            // o/i → create new object or open connection dialog
            if let Some(idx) = state.selected_tree_index() {
                match &state.tree[idx] {
                    TreeNode::Connection { .. } | TreeNode::Group { .. } => {
                        state.connection_form = crate::ui::state::ConnectionFormState::new();
                        state.overlay = Some(Overlay::ConnectionDialog);
                        return Action::Render;
                    }
                    TreeNode::Category { schema, kind, .. } => {
                        let obj_type = match kind {
                            crate::ui::state::CategoryKind::Tables => "TABLE",
                            crate::ui::state::CategoryKind::Views => "VIEW",
                            crate::ui::state::CategoryKind::Packages => "PACKAGE",
                            _ => return Action::Render,
                        };
                        let conn_name = find_conn_name_for(state, idx);
                        return Action::CreateFromTemplate {
                            conn_name,
                            schema: schema.clone(),
                            obj_type: obj_type.to_string(),
                        };
                    }
                    TreeNode::Leaf { schema, kind, .. } => {
                        let obj_type = match kind {
                            LeafKind::Table => "TABLE",
                            LeafKind::View => "VIEW",
                            LeafKind::Package => "PACKAGE",
                            _ => return Action::Render,
                        };
                        let conn_name = find_conn_name_for(state, idx);
                        return Action::CreateFromTemplate {
                            conn_name,
                            schema: schema.clone(),
                            obj_type: obj_type.to_string(),
                        };
                    }
                    _ => {
                        state.connection_form = crate::ui::state::ConnectionFormState::new();
                        state.overlay = Some(Overlay::ConnectionDialog);
                        return Action::Render;
                    }
                }
            } else {
                state.connection_form = crate::ui::state::ConnectionFormState::new();
                state.overlay = Some(Overlay::ConnectionDialog);
            }
            Action::Render
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
pub(super) fn find_conn_name_for(state: &AppState, mut idx: usize) -> String {
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

pub(super) fn handle_tree_action(state: &mut AppState, idx: usize) -> Action {
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
            label,
            ..
        } if !expanded => {
            let schema = schema.clone();
            let label = label.clone();
            state.tree[idx].toggle_expand();
            Action::LoadChildren {
                schema,
                kind: label,
            }
        }
        TreeNode::Leaf {
            schema,
            name,
            kind: LeafKind::Table | LeafKind::View | LeafKind::MaterializedView,
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
        TreeNode::Leaf {
            schema, name, kind, ..
        } if matches!(kind, LeafKind::Index | LeafKind::Sequence | LeafKind::Event) => {
            let schema = schema.clone();
            let obj_name = name.clone();
            let conn_name = find_conn_name_for(state, idx);
            let obj_type = match kind {
                LeafKind::Index => "INDEX",
                LeafKind::Sequence => "SEQUENCE",
                LeafKind::Event => "EVENT",
                _ => unreachable!(),
            };

            let tab_id = state.open_or_focus_tab(TabKind::Function {
                conn_name,
                schema: schema.clone(),
                name: obj_name.clone(),
            });

            Action::LoadSourceCode {
                tab_id,
                schema,
                name: obj_name,
                obj_type: obj_type.to_string(),
            }
        }
        TreeNode::Leaf {
            schema,
            name,
            kind: LeafKind::Type,
            ..
        } => {
            let schema = schema.clone();
            let type_name = name.clone();
            let conn_name = find_conn_name_for(state, idx);

            let tab_id = state.open_or_focus_tab(TabKind::DbType {
                conn_name,
                schema: schema.clone(),
                name: type_name.clone(),
            });

            Action::LoadTypeInfo {
                tab_id,
                schema,
                name: type_name,
            }
        }
        TreeNode::Leaf {
            schema,
            name,
            kind: LeafKind::Trigger,
            ..
        } => {
            let schema = schema.clone();
            let trigger_name = name.clone();
            let conn_name = find_conn_name_for(state, idx);

            let tab_id = state.open_or_focus_tab(TabKind::Trigger {
                conn_name,
                schema: schema.clone(),
                name: trigger_name.clone(),
            });

            Action::LoadTriggerInfo {
                tab_id,
                schema,
                name: trigger_name,
            }
        }
        _ => {
            state.tree[idx].toggle_expand();
            Action::Render
        }
    }
}

pub(super) fn insert_categories(state: &mut AppState, parent_idx: usize, schema: &str) {
    use crate::ui::state::CategoryKind;

    let categories: Vec<(&str, CategoryKind)> = match state.db_type {
        Some(DatabaseType::Oracle) => vec![
            ("Tables", CategoryKind::Tables),
            ("Views", CategoryKind::Views),
            ("Materialized Views", CategoryKind::MaterializedViews),
            ("Indexes", CategoryKind::Indexes),
            ("Sequences", CategoryKind::Sequences),
            ("Types", CategoryKind::Types),
            ("Triggers", CategoryKind::Triggers),
            ("Packages", CategoryKind::Packages),
            ("Procedures", CategoryKind::Procedures),
            ("Functions", CategoryKind::Functions),
        ],
        Some(DatabaseType::MySQL) => vec![
            ("Tables", CategoryKind::Tables),
            ("Views", CategoryKind::Views),
            ("Indexes", CategoryKind::Indexes),
            ("Triggers", CategoryKind::Triggers),
            ("Events", CategoryKind::Events),
            ("Procedures", CategoryKind::Procedures),
            ("Functions", CategoryKind::Functions),
        ],
        Some(DatabaseType::PostgreSQL) | None => vec![
            ("Tables", CategoryKind::Tables),
            ("Views", CategoryKind::Views),
            ("Materialized Views", CategoryKind::MaterializedViews),
            ("Indexes", CategoryKind::Indexes),
            ("Sequences", CategoryKind::Sequences),
            ("Triggers", CategoryKind::Triggers),
            ("Procedures", CategoryKind::Procedures),
            ("Functions", CategoryKind::Functions),
        ],
    };

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
