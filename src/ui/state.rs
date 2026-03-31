use std::collections::{HashMap, HashSet};

use crate::core::models::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Panel {
    Sidebar,
    DataGrid,
    Properties,
    PackageView,
    QueryEditor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    ConnectionDialog,
    ObjectFilter,
    ConnectionMenu,
    ScriptsBrowser,
    Help,
    MasterPassword,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnMenuAction {
    View,
    Edit,
    Connect,
    Disconnect,
    Restart,
    Delete,
}

impl ConnMenuAction {
    pub fn all() -> Vec<Self> {
        vec![
            Self::View,
            Self::Edit,
            Self::Connect,
            Self::Disconnect,
            Self::Restart,
            Self::Delete,
        ]
    }

    pub fn label(&self) -> &str {
        match self {
            Self::View => "View connection info",
            Self::Edit => "Edit connection",
            Self::Connect => "Connect",
            Self::Disconnect => "Disconnect",
            Self::Restart => "Restart connection",
            Self::Delete => "Delete connection",
        }
    }

    pub fn icon(&self) -> &str {
        match self {
            Self::View => "👁",
            Self::Edit => "✎",
            Self::Connect => "●",
            Self::Disconnect => "○",
            Self::Restart => "↻",
            Self::Delete => "✗",
        }
    }
}

pub struct ConnMenuState {
    pub conn_name: String,
    pub cursor: usize,
    pub is_connected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CenterTab {
    Data,
    Properties,
    DDL,
    Declaration,
    Body,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnStatus {
    Disconnected,
    Connecting,
    Connected,
    Failed,
}

#[derive(Debug, Clone)]
pub enum TreeNode {
    Connection {
        name: String,
        expanded: bool,
        status: ConnStatus,
    },
    Schema {
        name: String,
        expanded: bool,
    },
    Category {
        label: String,
        schema: String,
        kind: CategoryKind,
        expanded: bool,
    },
    Leaf {
        name: String,
        schema: String,
        kind: LeafKind,
        valid: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CategoryKind {
    Tables,
    Views,
    Packages,
    Procedures,
    Functions,
}

impl CategoryKind {
    pub fn filter_key(&self, schema: &str) -> String {
        format!("{}.{:?}", schema, self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeafKind {
    Table,
    View,
    Package,
    Procedure,
    Function,
}

impl TreeNode {
    pub fn display_name(&self) -> &str {
        match self {
            TreeNode::Connection { name, .. } => name,
            TreeNode::Schema { name, .. } => name,
            TreeNode::Category { label, .. } => label,
            TreeNode::Leaf { name, .. } => name,
        }
    }

    pub fn is_expanded(&self) -> bool {
        match self {
            TreeNode::Connection { expanded, .. }
            | TreeNode::Schema { expanded, .. }
            | TreeNode::Category { expanded, .. } => *expanded,
            TreeNode::Leaf { .. } => false,
        }
    }

    pub fn toggle_expand(&mut self) {
        match self {
            TreeNode::Connection { expanded, .. }
            | TreeNode::Schema { expanded, .. }
            | TreeNode::Category { expanded, .. } => *expanded = !*expanded,
            TreeNode::Leaf { .. } => {}
        }
    }

    pub fn depth(&self) -> usize {
        match self {
            TreeNode::Connection { .. } => 0,
            TreeNode::Schema { .. } => 1,
            TreeNode::Category { .. } => 2,
            TreeNode::Leaf { .. } => 3,
        }
    }
}

// --- Sidebar Tree State (Neovim-like scroll) ---

pub const SCROLLOFF: usize = 2;

pub struct TreeState {
    pub cursor: usize,
    pub offset: usize,
    pub visible_height: usize,
    pub search_active: bool,
    pub search_query: String,
    pub search_matches: Vec<usize>,
    pub search_match_idx: usize,
    pub pending_d: bool,
}

impl TreeState {
    pub fn new() -> Self {
        Self {
            cursor: 0,
            offset: 0,
            visible_height: 20,
            search_active: false,
            search_query: String::new(),
            search_matches: vec![],
            search_match_idx: 0,
            pending_d: false,
        }
    }

    pub fn move_down(&mut self, visible_count: usize) {
        if self.cursor + 1 < visible_count {
            self.cursor += 1;
            self.adjust_scroll(visible_count);
        }
    }

    pub fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.adjust_scroll_up();
        }
    }

    pub fn half_page_down(&mut self, visible_count: usize) {
        let half = self.visible_height / 2;
        self.cursor = (self.cursor + half).min(visible_count.saturating_sub(1));
        self.center_scroll(visible_count);
    }

    pub fn half_page_up(&mut self, visible_count: usize) {
        let half = self.visible_height / 2;
        self.cursor = self.cursor.saturating_sub(half);
        self.center_scroll(visible_count);
    }

    pub fn go_top(&mut self) {
        self.cursor = 0;
        self.offset = 0;
    }

    pub fn go_bottom(&mut self, visible_count: usize) {
        if visible_count > 0 {
            self.cursor = visible_count - 1;
            self.offset = visible_count.saturating_sub(self.visible_height);
        }
    }

    fn adjust_scroll(&mut self, visible_count: usize) {
        let vh = self.visible_height;
        if self.cursor + SCROLLOFF >= self.offset + vh {
            self.offset = (self.cursor + SCROLLOFF + 1).saturating_sub(vh);
        }
        let max_offset = visible_count.saturating_sub(vh);
        if self.offset > max_offset {
            self.offset = max_offset;
        }
    }

    fn adjust_scroll_up(&mut self) {
        if self.cursor < self.offset + SCROLLOFF {
            self.offset = self.cursor.saturating_sub(SCROLLOFF);
        }
    }

    pub fn center_scroll(&mut self, visible_count: usize) {
        let vh = self.visible_height;
        self.offset = self.cursor.saturating_sub(vh / 2);
        let max_offset = visible_count.saturating_sub(vh);
        if self.offset > max_offset {
            self.offset = max_offset;
        }
    }

    pub fn next_match(&mut self, visible_count: usize) {
        if self.search_matches.is_empty() {
            return;
        }
        self.search_match_idx = (self.search_match_idx + 1) % self.search_matches.len();
        self.cursor = self.search_matches[self.search_match_idx];
        self.center_scroll(visible_count);
    }
}

impl Default for TreeState {
    fn default() -> Self {
        Self::new()
    }
}

// --- Connection Form State ---

pub struct ConnectionFormState {
    pub name: String,
    pub db_type_idx: usize,
    pub host: String,
    pub port: String,
    pub username: String,
    pub password: String,
    pub database: String,
    pub selected_field: usize,
    pub error_message: String,
    pub password_visible: bool,
    pub connecting: bool,
    pub show_saved_list: bool,
    pub saved_cursor: usize,
    pub editing_name: Option<String>,
    pub read_only: bool,
}

impl ConnectionFormState {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            db_type_idx: 0,
            host: "localhost".to_string(),
            port: "5432".to_string(),
            username: String::new(),
            password: String::new(),
            database: String::new(),
            selected_field: 0,
            error_message: String::new(),
            password_visible: false,
            connecting: false,
            show_saved_list: false,
            saved_cursor: 0,
            editing_name: None,
            read_only: false,
        }
    }

    pub fn db_type_label(&self) -> &str {
        match self.db_type_idx {
            0 => "PostgreSQL",
            1 => "MySQL",
            2 => "Oracle",
            _ => "PostgreSQL",
        }
    }

    pub fn cycle_db_type(&mut self) {
        self.db_type_idx = (self.db_type_idx + 1) % 3;
        self.port = match self.db_type_idx {
            0 => "5432".to_string(),
            1 => "3306".to_string(),
            2 => "1521".to_string(),
            _ => "5432".to_string(),
        };
    }

    pub fn to_connection_config(&self) -> ConnectionConfig {
        let db_type = match self.db_type_idx {
            1 => DatabaseType::MySQL,
            2 => DatabaseType::Oracle,
            _ => DatabaseType::PostgreSQL,
        };
        ConnectionConfig {
            name: self.name.clone(),
            db_type,
            host: self.host.clone(),
            port: self.port.parse().unwrap_or(5432),
            username: self.username.clone(),
            password: self.password.clone(),
            database: if self.database.is_empty() {
                None
            } else {
                Some(self.database.clone())
            },
        }
    }

    pub fn from_config(config: &ConnectionConfig) -> Self {
        let db_type_idx = match config.db_type {
            DatabaseType::PostgreSQL => 0,
            DatabaseType::MySQL => 1,
            DatabaseType::Oracle => 2,
        };
        Self {
            name: config.name.clone(),
            db_type_idx,
            host: config.host.clone(),
            port: config.port.to_string(),
            username: config.username.clone(),
            password: config.password.clone(),
            database: config.database.clone().unwrap_or_default(),
            selected_field: 0,
            error_message: String::new(),
            password_visible: false,
            connecting: false,
            show_saved_list: false,
            saved_cursor: 0,
            editing_name: None,
            read_only: false,
        }
    }

    pub fn for_edit(config: &ConnectionConfig) -> Self {
        let mut form = Self::from_config(config);
        form.editing_name = Some(config.name.clone());
        form
    }

    pub fn active_field_mut(&mut self) -> &mut String {
        match self.selected_field {
            0 => &mut self.name,
            1 => &mut self.name,
            2 => &mut self.host,
            3 => &mut self.port,
            4 => &mut self.username,
            5 => &mut self.password,
            6 => &mut self.database,
            _ => &mut self.name,
        }
    }

    pub fn next_field(&mut self) {
        self.selected_field = (self.selected_field + 1) % 7;
    }

    pub fn prev_field(&mut self) {
        self.selected_field = if self.selected_field == 0 { 6 } else { self.selected_field - 1 };
    }
}

impl Default for ConnectionFormState {
    fn default() -> Self {
        Self::new()
    }
}

// --- Generic Object Filter (works for schemas, tables, views, etc.) ---

pub struct ObjectFilterState {
    /// Key: filter context (e.g. "schemas", "SCOTT.Tables")
    /// Value: set of enabled names. Empty = show all.
    pub filters: HashMap<String, HashSet<String>>,
    /// Current filter context being edited
    pub current_key: String,
    /// All items available for current filter
    pub all_items: Vec<String>,
    pub cursor: usize,
    pub offset: usize,
    pub visible_height: usize,
    pub search_active: bool,
    pub search_query: String,
}

impl ObjectFilterState {
    pub fn new() -> Self {
        Self {
            filters: HashMap::new(),
            current_key: String::new(),
            all_items: vec![],
            cursor: 0,
            offset: 0,
            visible_height: 15,
            search_active: false,
            search_query: String::new(),
        }
    }

    pub fn is_enabled(&self, key: &str, name: &str) -> bool {
        match self.filters.get(key) {
            None => true,        // No filter = show all
            Some(set) if set.is_empty() => true,
            Some(set) => set.contains(name),
        }
    }

    pub fn has_filter(&self, key: &str) -> bool {
        self.filters.get(key).is_some_and(|s| !s.is_empty())
    }

    pub fn enabled_count(&self, key: &str, total: usize) -> (usize, usize) {
        match self.filters.get(key) {
            None => (total, total),
            Some(set) if set.is_empty() => (total, total),
            Some(set) => (set.len(), total),
        }
    }

    /// Prepare filter for a specific context
    pub fn open_for(&mut self, key: &str, items: Vec<String>) {
        self.current_key = key.to_string();
        self.all_items = items;
        self.cursor = 0;
        self.offset = 0;
        self.search_active = false;
        self.search_query.clear();
    }

    pub fn display_list(&self) -> Vec<(usize, &str)> {
        if self.search_query.is_empty() {
            self.all_items
                .iter()
                .enumerate()
                .map(|(i, s)| (i, s.as_str()))
                .collect()
        } else {
            let q = self.search_query.to_lowercase();
            self.all_items
                .iter()
                .enumerate()
                .filter(|(_, s)| s.to_lowercase().contains(&q))
                .map(|(i, s)| (i, s.as_str()))
                .collect()
        }
    }

    pub fn toggle_at_cursor(&mut self) {
        let display = self.display_list();
        if let Some((real_idx, _)) = display.get(self.cursor) {
            let name = self.all_items[*real_idx].clone();
            let set = self.filters.entry(self.current_key.clone()).or_default();
            if set.contains(&name) {
                set.remove(&name);
            } else {
                set.insert(name);
            }
        }
    }

    pub fn is_item_enabled(&self, name: &str) -> bool {
        self.is_enabled(&self.current_key, name)
    }

    pub fn select_all(&mut self) {
        self.filters.remove(&self.current_key);
    }

    pub fn filter_info(&self, key: &str) -> Option<(usize, usize)> {
        if let Some(set) = self.filters.get(key) {
            if !set.is_empty() {
                return Some((set.len(), self.all_items.len()));
            }
        }
        None
    }

    pub fn move_down(&mut self) {
        let count = self.display_list().len();
        if count > 0 && self.cursor + 1 < count {
            self.cursor += 1;
            let vh = self.visible_height;
            if self.cursor + SCROLLOFF >= self.offset + vh {
                self.offset = (self.cursor + SCROLLOFF + 1).saturating_sub(vh);
            }
        }
    }

    pub fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            if self.cursor < self.offset + SCROLLOFF {
                self.offset = self.cursor.saturating_sub(SCROLLOFF);
            }
        }
    }

    pub fn go_top(&mut self) {
        self.cursor = 0;
        self.offset = 0;
    }

    pub fn go_bottom(&mut self) {
        let count = self.display_list().len();
        if count > 0 {
            self.cursor = count - 1;
            self.offset = count.saturating_sub(self.visible_height);
        }
    }
}

impl Default for ObjectFilterState {
    fn default() -> Self {
        Self::new()
    }
}

// --- App State ---

pub struct AppState {
    pub mode: Mode,
    pub active_panel: Panel,
    pub active_tab: CenterTab,
    pub overlay: Option<Overlay>,

    pub connection_name: Option<String>,
    pub db_type: Option<DatabaseType>,
    pub current_schema: Option<String>,
    pub connected: bool,

    pub tree: Vec<TreeNode>,
    pub tree_state: TreeState,

    /// Generic filter for any tree level
    pub object_filter: ObjectFilterState,

    pub query_result: Option<QueryResult>,
    pub grid_scroll_row: usize,
    pub grid_scroll_col: usize,
    pub grid_selected_row: usize,
    pub grid_selected_col: usize,

    pub columns: Vec<Column>,
    pub package_content: Option<PackageContent>,

    pub editor_content: String,
    pub editor_cursor_row: usize,
    pub editor_cursor_col: usize,

    pub status_message: String,
    pub loading: bool,

    pub connection_form: ConnectionFormState,
    pub conn_menu: ConnMenuState,
    pub saved_connections: Vec<ConnectionConfig>,

    pub show_editor: bool,
    pub grid_visible_height: usize,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            mode: Mode::Normal,
            active_panel: Panel::Sidebar,
            active_tab: CenterTab::Data,
            overlay: None,
            connection_name: None,
            db_type: None,
            current_schema: None,
            connected: false,
            tree: vec![],
            tree_state: TreeState::new(),
            object_filter: ObjectFilterState::new(),
            query_result: None,
            grid_scroll_row: 0,
            grid_scroll_col: 0,
            grid_selected_row: 0,
            grid_selected_col: 0,
            columns: vec![],
            package_content: None,
            editor_content: String::new(),
            editor_cursor_row: 0,
            editor_cursor_col: 0,
            status_message: "Ready - press 'a' to add connection, '?' for help".to_string(),
            loading: false,
            connection_form: ConnectionFormState::new(),
            conn_menu: ConnMenuState {
                conn_name: String::new(),
                cursor: 0,
                is_connected: false,
            },
            saved_connections: vec![],
            show_editor: false,
            grid_visible_height: 20,
        }
    }

    /// Get visible tree nodes, filtered at ALL levels
    pub fn visible_tree(&self) -> Vec<(usize, &TreeNode)> {
        let mut visible = Vec::new();
        let mut i = 0;
        while i < self.tree.len() {
            let node = &self.tree[i];

            // Filter schemas
            if let TreeNode::Schema { name, .. } = node {
                if !self.object_filter.is_enabled("schemas", name) {
                    let d = node.depth();
                    i += 1;
                    while i < self.tree.len() && self.tree[i].depth() > d {
                        i += 1;
                    }
                    continue;
                }
            }

            // Filter leaves (tables, views, etc.)
            if let TreeNode::Leaf {
                name,
                schema,
                kind,
                ..
            } = node
            {
                let cat_key = match kind {
                    LeafKind::Table => format!("{schema}.Tables"),
                    LeafKind::View => format!("{schema}.Views"),
                    LeafKind::Package => format!("{schema}.Packages"),
                    LeafKind::Procedure => format!("{schema}.Procedures"),
                    LeafKind::Function => format!("{schema}.Functions"),
                };
                if !self.object_filter.is_enabled(&cat_key, name) {
                    i += 1;
                    continue;
                }
            }

            visible.push((i, node));

            if !node.is_expanded() {
                let d = node.depth();
                i += 1;
                while i < self.tree.len() && self.tree[i].depth() > d {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        visible
    }

    /// Get filter hint for a node if a filter is active at that level
    pub fn filter_hint_for(&self, node: &TreeNode) -> Option<String> {
        match node {
            TreeNode::Connection { expanded: true, .. } => {
                if self.object_filter.has_filter("schemas") {
                    let total = self.all_schema_names().len();
                    let enabled = self.object_filter.filters.get("schemas")
                        .map(|s| s.len()).unwrap_or(total);
                    Some(format!("... ({enabled}/{total} schemas shown)"))
                } else {
                    None
                }
            }
            TreeNode::Category { expanded: true, schema, kind, .. } => {
                let key = kind.filter_key(schema);
                if self.object_filter.has_filter(&key) {
                    let total_in_tree = self.leaves_under_category_count(&key);
                    let enabled = self.object_filter.filters.get(&key)
                        .map(|s| s.len()).unwrap_or(total_in_tree);
                    Some(format!("... ({enabled}/{total_in_tree} shown)"))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn leaves_under_category_count(&self, filter_key: &str) -> usize {
        // Count how many leaves exist for this category in the tree
        // The filter_key format is "SCHEMA.CategoryKind"
        let parts: Vec<&str> = filter_key.splitn(2, '.').collect();
        if parts.len() != 2 { return 0; }
        let (schema, kind_str) = (parts[0], parts[1]);

        self.tree.iter().filter(|n| {
            if let TreeNode::Leaf { schema: s, kind, .. } = n {
                let k = format!("{:?}", kind);
                // LeafKind::Table -> "Table", CategoryKind is "Tables"
                s == schema && kind_str.starts_with(&k)
            } else {
                false
            }
        }).count()
    }

    pub fn selected_tree_index(&self) -> Option<usize> {
        let visible = self.visible_tree();
        visible.get(self.tree_state.cursor).map(|(idx, _)| *idx)
    }

    /// Get all leaf names under a category for filter purposes
    pub fn leaves_under_category(&self, cat_idx: usize) -> Vec<String> {
        let mut items = vec![];
        let cat_depth = self.tree[cat_idx].depth();
        let mut i = cat_idx + 1;
        while i < self.tree.len() && self.tree[i].depth() > cat_depth {
            if let TreeNode::Leaf { name, .. } = &self.tree[i] {
                items.push(name.clone());
            }
            i += 1;
        }
        items
    }

    /// Get all schema names in the tree
    pub fn all_schema_names(&self) -> Vec<String> {
        self.tree
            .iter()
            .filter_map(|n| {
                if let TreeNode::Schema { name, .. } = n {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Count filtered items for a given filter key, compared to items in tree
    pub fn filter_hint(&self, key: &str, total_in_tree: usize) -> Option<String> {
        if let Some(set) = self.object_filter.filters.get(key) {
            if !set.is_empty() && set.len() < total_in_tree {
                return Some(format!("... ({}/{} filtered)", set.len(), total_in_tree));
            }
        }
        None
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
