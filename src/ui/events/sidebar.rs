use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::core::models::DatabaseType;
use crate::keybindings::Context;
use crate::ui::state::{AppState, LeafKind, Overlay, TreeNode};
use crate::ui::tabs::TabKind;

use super::Action;
use super::overlays::{handle_conn_rename, handle_group_create, handle_group_rename};

pub(super) fn handle_filter_key(state: &mut AppState) -> Action {
    if let Some(idx) = state.selected_tree_index() {
        // Prefix filter keys with connection name so each connection has independent filters
        let conn_prefix = state.connection_for_tree_idx(idx).unwrap_or("").to_string();

        match &state.sidebar.tree[idx] {
            TreeNode::Group { .. } => {}
            TreeNode::Connection { .. } | TreeNode::Schema { .. } => {
                let schemas = state.schema_names_for_conn(&conn_prefix);
                if !schemas.is_empty() {
                    let key = format!("{conn_prefix}::schemas");
                    state.sidebar.object_filter.open_for(&key, schemas);
                    state.overlay = Some(Overlay::ObjectFilter);
                }
            }
            TreeNode::Category { schema, kind, .. } => {
                let base_key = kind.filter_key(schema);
                let key = format!("{conn_prefix}::{base_key}");
                let items = state.leaves_under_category(idx);
                if !items.is_empty() {
                    state.sidebar.object_filter.open_for(&key, items);
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
                    if matches!(&state.sidebar.tree[walk], TreeNode::Category { .. }) {
                        let items = state.leaves_under_category(walk);
                        if !items.is_empty() {
                            state.sidebar.object_filter.open_for(&cat_key, items);
                            state.overlay = Some(Overlay::ObjectFilter);
                        }
                        break;
                    }
                }
            }
        }
    } else if !state.sidebar.tree.is_empty() {
        let schemas = state.all_schema_names();
        if !schemas.is_empty() {
            state.sidebar.object_filter.open_for("schemas", schemas);
            state.overlay = Some(Overlay::ObjectFilter);
        }
    }
    Action::Render
}

// --- Sidebar Search ---

pub(super) fn handle_sidebar_search(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.sidebar.tree_state.search_active = false;
            state.sidebar.tree_state.search_query.clear();
            state.sidebar.tree_state.search_matches.clear();
            Action::Render
        }
        KeyCode::Enter => {
            state.sidebar.tree_state.search_active = false;
            Action::Render
        }
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let count = state.visible_tree().len();
            state.sidebar.tree_state.next_match(count);
            Action::Render
        }
        KeyCode::Backspace => {
            state.sidebar.tree_state.search_query.pop();
            update_search_and_jump(state);
            Action::Render
        }
        KeyCode::Char(c) => {
            state.sidebar.tree_state.search_query.push(c);
            update_search_and_jump(state);
            Action::Render
        }
        _ => Action::None,
    }
}

pub(super) fn update_search_and_jump(state: &mut AppState) {
    let query = state.sidebar.tree_state.search_query.to_lowercase();
    let visible = state.visible_tree();
    let mut matches = Vec::new();
    for (vis_idx, (_, node, _)) in visible.iter().enumerate() {
        if !query.is_empty() && node.display_name().to_lowercase().contains(&query) {
            matches.push(vis_idx);
        }
    }
    let count = visible.len();
    drop(visible);

    state.sidebar.tree_state.search_matches = matches;
    state.sidebar.tree_state.search_match_idx = 0;
    if let Some(&first) = state.sidebar.tree_state.search_matches.first() {
        state.sidebar.tree_state.cursor = first;
        state.sidebar.tree_state.center_scroll(count);
    }
}

// --- Sidebar (Neovim-like) ---

pub(super) fn handle_sidebar(state: &mut AppState, key: KeyEvent) -> Action {
    // Group rename mode
    if state.dialogs.group_renaming.is_some() {
        return handle_group_rename(state, key);
    }
    // Group create mode
    if state.dialogs.group_creating {
        return handle_group_create(state, key);
    }
    // Connection inline rename mode
    if state.dialogs.conn_renaming.is_some() {
        return handle_conn_rename(state, key);
    }

    let visible_count = state.visible_tree().len();
    if visible_count == 0 {
        return Action::None;
    }

    // Half-page scroll (configurable).
    if state
        .bindings
        .matches(Context::Sidebar, "half_page_down", &key)
    {
        state.sidebar.tree_state.half_page_down(visible_count);
        return Action::Render;
    }
    if state
        .bindings
        .matches(Context::Sidebar, "half_page_up", &key)
    {
        state.sidebar.tree_state.half_page_up(visible_count);
        return Action::Render;
    }

    // Reset pending_d if the key isn't the delete key (so `d<something>`
    // doesn't get stuck in pending state).
    if !state
        .bindings
        .matches(Context::Sidebar, "delete_pending", &key)
    {
        state.sidebar.tree_state.pending_d = false;
    }

    // Configurable simple scroll/movement/expand bindings. These cover the
    // Sidebar context defaults; users can rebind them in keybindings.toml.
    let b = &state.bindings;
    if b.matches(Context::Sidebar, "scroll_down", &key) {
        state.sidebar.tree_state.move_down(visible_count);
        return Action::Render;
    }
    if b.matches(Context::Sidebar, "scroll_up", &key) {
        state.sidebar.tree_state.move_up();
        return Action::Render;
    }
    if b.matches(Context::Sidebar, "scroll_top", &key) {
        state.sidebar.tree_state.go_top();
        return Action::Render;
    }
    if b.matches(Context::Sidebar, "scroll_bottom", &key) {
        state.sidebar.tree_state.go_bottom(visible_count);
        return Action::Render;
    }
    if b.matches(Context::Sidebar, "expand_or_open", &key) {
        return if let Some(idx) = state.selected_tree_index() {
            handle_tree_action(state, idx)
        } else {
            Action::None
        };
    }
    if b.matches(Context::Sidebar, "start_search", &key) {
        state.sidebar.tree_state.search_active = true;
        state.sidebar.tree_state.search_query.clear();
        state.sidebar.tree_state.search_matches.clear();
        return Action::Render;
    }

    if state
        .bindings
        .matches(Context::Sidebar, "collapse_or_parent", &key)
    {
        if let Some(idx) = state.selected_tree_index() {
            if state.sidebar.tree[idx].is_expanded() {
                // Collapse current node
                state.sidebar.tree[idx].toggle_expand();
            } else {
                // Navigate to parent and collapse it
                let child_depth = state.sidebar.tree[idx].depth();
                if child_depth > 0 {
                    let mut walk = idx;
                    while walk > 0 {
                        walk -= 1;
                        if state.sidebar.tree[walk].depth() < child_depth {
                            if state.sidebar.tree[walk].is_expanded() {
                                state.sidebar.tree[walk].toggle_expand();
                            }
                            let vis_info = {
                                let visible = state.visible_tree();
                                visible
                                    .iter()
                                    .position(|(vi, _, _)| *vi == walk)
                                    .map(|p| (p, visible.len()))
                            };
                            if let Some((vis_pos, vis_len)) = vis_info {
                                state.sidebar.tree_state.cursor = vis_pos;
                                state.sidebar.tree_state.adjust_scroll(vis_len);
                            }
                            break;
                        }
                    }
                }
            }
        }
        return Action::Render;
    }

    match key.code {
        _ if state
            .bindings
            .matches(Context::Sidebar, "delete_pending", &key) =>
        {
            if state.sidebar.tree_state.pending_d {
                state.sidebar.tree_state.pending_d = false;
                if let Some(idx) = state.selected_tree_index() {
                    match &state.sidebar.tree[idx] {
                        TreeNode::Connection { name, .. } => {
                            state.overlay =
                                Some(Overlay::ConfirmDeleteConnection { name: name.clone() });
                            return Action::Render;
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
                            state.sidebar.pending_action =
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
                state.sidebar.tree_state.pending_d = true;
                Action::Render
            }
        }
        _ if state
            .bindings
            .matches(Context::Sidebar, "rename_or_refresh", &key) =>
        {
            // r → context-aware:
            //   - on a Group (collection) → inline rename
            //   - on a Connection → inline rename (oil-style, no modal)
            //   - on a Leaf (Table/View) → rename modal
            //   - on a Category → reload the children of that category
            //   - on a Schema → reload all expanded categories under it
            if let Some(idx) = state.selected_tree_index() {
                match &state.sidebar.tree[idx] {
                    TreeNode::Group { name, .. } => {
                        state.dialogs.group_renaming = Some(name.clone());
                        state.dialogs.group_rename_buf = name.clone();
                        return Action::Render;
                    }
                    TreeNode::Connection { name, .. } => {
                        state.dialogs.conn_renaming = Some(name.clone());
                        state.dialogs.conn_rename_buf = name.clone();
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
                        state.sidebar.rename_buf = name.clone();
                        state.sidebar.pending_action =
                            Some(crate::ui::state::PendingObjectAction {
                                schema: schema.clone(),
                                name: name.clone(),
                                obj_type: obj_type.to_string(),
                                conn_name,
                            });
                        state.overlay = Some(Overlay::RenameObject);
                        return Action::Render;
                    }
                    TreeNode::Category { schema, label, .. } => {
                        let schema = schema.clone();
                        let label = label.clone();
                        // Drop the children of this category so they get re-fetched
                        let depth = state.sidebar.tree[idx].depth();
                        let mut end = idx + 1;
                        while end < state.sidebar.tree.len()
                            && state.sidebar.tree[end].depth() > depth
                        {
                            end += 1;
                        }
                        if end > idx + 1 {
                            state.sidebar.tree.drain(idx + 1..end);
                        }
                        state.status_message = format!("Refreshing {label}...");
                        return Action::LoadChildren { schema, kind: label };
                    }
                    TreeNode::Schema { name: schema_name, .. } => {
                        // Reload every expanded category under this schema
                        let schema = schema_name.clone();
                        let depth = state.sidebar.tree[idx].depth();
                        let mut categories: Vec<String> = Vec::new();
                        let mut i = idx + 1;
                        while i < state.sidebar.tree.len()
                            && state.sidebar.tree[i].depth() > depth
                        {
                            if let TreeNode::Category { label, expanded, .. } =
                                &state.sidebar.tree[i]
                                && *expanded
                            {
                                categories.push(label.clone());
                            }
                            i += 1;
                        }
                        // Drop and re-load each expanded category
                        for label in &categories {
                            // Find this category fresh because the tree mutates between iters
                            if let Some(cat_idx) =
                                state.sidebar.tree.iter().position(|n| matches!(n,
                                    TreeNode::Category { schema: s, label: l, .. }
                                        if s == &schema && l == label))
                            {
                                let cdepth = state.sidebar.tree[cat_idx].depth();
                                let mut cend = cat_idx + 1;
                                while cend < state.sidebar.tree.len()
                                    && state.sidebar.tree[cend].depth() > cdepth
                                {
                                    cend += 1;
                                }
                                if cend > cat_idx + 1 {
                                    state.sidebar.tree.drain(cat_idx + 1..cend);
                                }
                            }
                        }
                        state.status_message =
                            format!("Refreshing schema {schema}...");
                        return Action::RefreshSchema {
                            schema,
                            kinds: categories,
                        };
                    }
                    _ => {}
                }
            }
            Action::Render
        }
        _ if state
            .bindings
            .matches(Context::Sidebar, "yank_pending", &key) =>
        {
            // yy → yank connection for duplicate
            if let Some(idx) = state.selected_tree_index() {
                let mut walk = idx;
                loop {
                    if let TreeNode::Connection { name, .. } = &state.sidebar.tree[walk] {
                        state.sidebar.yank_conn = Some(name.clone());
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
        _ if state.bindings.matches(Context::Sidebar, "paste", &key) => {
            // p → paste (duplicate) yanked connection into current group
            if let Some(ref source) = state.sidebar.yank_conn.clone() {
                // Find group at cursor position by walking up
                let group = if let Some(idx) = state.selected_tree_index() {
                    let mut walk = idx;
                    loop {
                        if let TreeNode::Group { name, .. } = &state.sidebar.tree[walk] {
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
        _ if state
            .bindings
            .matches(Context::Sidebar, "create_new", &key) =>
        {
            // o/i → context-aware (oil-style):
            //   - on a COLLAPSED Group → start inline create-new-collection
            //   - on an EXPANDED Group → open connection dialog (creates inside)
            //   - on a Connection → open connection dialog
            //   - on a Category/Leaf → CREATE FROM TEMPLATE
            if let Some(idx) = state.selected_tree_index() {
                match &state.sidebar.tree[idx] {
                    TreeNode::Group { expanded, .. } => {
                        if *expanded {
                            // Inside an expanded group → create connection here
                            let group_name = if let TreeNode::Group { name, .. } =
                                &state.sidebar.tree[idx]
                            {
                                name.clone()
                            } else {
                                "Default".to_string()
                            };
                            state.dialogs.connection_form =
                                crate::ui::state::ConnectionFormState::new();
                            state.dialogs.connection_form.group = group_name;
                            state.dialogs.connection_form.group_options =
                                state.available_groups();
                            state.overlay = Some(Overlay::ConnectionDialog);
                        } else {
                            // Collapsed group → create a new collection inline
                            state.dialogs.group_creating = true;
                            state.dialogs.group_rename_buf.clear();
                        }
                        return Action::Render;
                    }
                    TreeNode::Connection { .. } => {
                        state.dialogs.connection_form =
                            crate::ui::state::ConnectionFormState::new();
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
                        state.dialogs.connection_form =
                            crate::ui::state::ConnectionFormState::new();
                        state.overlay = Some(Overlay::ConnectionDialog);
                        return Action::Render;
                    }
                }
            } else {
                state.dialogs.connection_form = crate::ui::state::ConnectionFormState::new();
                state.overlay = Some(Overlay::ConnectionDialog);
            }
            Action::Render
        }
        _ if state
            .bindings
            .matches(Context::Sidebar, "group_menu", &key) =>
        {
            if let Some(idx) = state.selected_tree_index() {
                // If on a Group node, open group menu
                if let TreeNode::Group { name, .. } = &state.sidebar.tree[idx] {
                    let group_name = name.clone();
                    let has_children = idx + 1 < state.sidebar.tree.len()
                        && state.sidebar.tree[idx + 1].depth() > state.sidebar.tree[idx].depth();
                    state.dialogs.group_menu.group_name = group_name;
                    state.dialogs.group_menu.cursor = 0;
                    state.dialogs.group_menu.is_empty = !has_children;
                    state.overlay = Some(Overlay::GroupMenu);
                    return Action::Render;
                }
                let mut walk = idx;
                loop {
                    if let TreeNode::Connection { name, status, .. } = &state.sidebar.tree[walk] {
                        let conn_name = name.clone();
                        state.dialogs.conn_menu.conn_name = conn_name;
                        state.dialogs.conn_menu.cursor = 0;
                        state.dialogs.conn_menu.is_connected =
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
        _ => Action::None,
    }
}

/// Find the connection name for a tree node by walking up to the Connection node
pub(super) fn find_conn_name_for(state: &AppState, mut idx: usize) -> String {
    loop {
        if let TreeNode::Connection { name, .. } = &state.sidebar.tree[idx] {
            return name.clone();
        }
        if idx == 0 {
            break;
        }
        idx -= 1;
    }
    state.conn.name.clone().unwrap_or_default()
}

pub(super) fn handle_tree_action(state: &mut AppState, idx: usize) -> Action {
    if idx >= state.sidebar.tree.len() {
        return Action::None;
    }

    let node = &state.sidebar.tree[idx];
    match node {
        TreeNode::Connection { expanded, name, .. } if !expanded => {
            let conn_name = name.clone();
            state.sidebar.tree[idx].toggle_expand();
            Action::LoadSchemas { conn_name }
        }
        TreeNode::Schema { expanded, name, .. } if !expanded => {
            let schema = name.clone();
            state.sidebar.tree[idx].toggle_expand();
            let has_children = idx + 1 < state.sidebar.tree.len()
                && state.sidebar.tree[idx + 1].depth() > state.sidebar.tree[idx].depth();
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
            state.sidebar.tree[idx].toggle_expand();
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
            state.conn.current_schema = Some(schema.clone());

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
            state.sidebar.tree[idx].toggle_expand();
            Action::Render
        }
    }
}

pub(super) fn insert_categories(state: &mut AppState, parent_idx: usize, schema: &str) {
    use crate::ui::state::CategoryKind;

    let categories: Vec<(&str, CategoryKind)> = match state.conn.db_type {
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
        state.sidebar.tree.insert(
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
