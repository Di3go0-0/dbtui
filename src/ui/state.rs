use std::collections::{HashMap, HashSet};

use crate::core::models::*;
use crate::ui::tabs::{TabId, TabKind, WorkspaceTab};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    ScriptsPanel,
    TabContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    ConnectionDialog,
    ObjectFilter,
    ConnectionMenu,
    GroupMenu,
    Help,
    ConfirmClose,
    ConfirmQuit,
    SaveScriptName,
    ScriptConnection,
    ThemePicker,
    BindVariables,
    SaveGridChanges,
    ConfirmDropObject,
    RenameObject,
    ConfirmCompile,
    ExportDialog,
    ImportDialog,
}

/// Info about an object pending drop/rename
#[derive(Debug, Clone)]
pub struct PendingObjectAction {
    pub schema: String,
    pub name: String,
    pub obj_type: String, // "TABLE", "VIEW", "PACKAGE"
    pub conn_name: String,
}

pub struct GroupMenuState {
    pub group_name: String,
    pub cursor: usize,
    pub is_empty: bool, // true if the group has no connections
}

// ---------------------------------------------------------------------------
// Export / Import dialog state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportField {
    Path,
    IncludeCredentials,
    ShowPassword,
    Password,
    Confirm,
}

#[derive(Debug, Clone)]
pub struct ExportDialogState {
    pub path: String,
    pub include_credentials: bool,
    pub show_password: bool,
    pub password: String,
    pub confirm: String,
    pub focused: ExportField,
    pub error: Option<String>,
    pub path_completions: Vec<String>,
    pub completion_idx: usize,
}

impl ExportDialogState {
    pub fn new() -> Self {
        // Default path: ~/dbtui_export_{date}.dbx
        let date = {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let days = secs / 86400;
            // Simple date approximation (good enough for filename)
            let y = 1970 + (days / 365);
            let rem = days % 365;
            let m = rem / 30 + 1;
            let d = rem % 30 + 1;
            format!("{y}-{m:02}-{d:02}")
        };
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Self {
            path: format!("{home}/dbtui_export_{date}.dbx"),
            include_credentials: true,
            show_password: false,
            password: String::new(),
            confirm: String::new(),
            focused: ExportField::Path,
            error: None,
            path_completions: Vec::new(),
            completion_idx: 0,
        }
    }

    pub fn complete_path(&mut self) {
        complete_path_field(
            &mut self.path,
            &mut self.path_completions,
            &mut self.completion_idx,
        );
    }

    pub fn reset_completions(&mut self) {
        self.path_completions.clear();
        self.completion_idx = 0;
    }

    pub fn next_field(&mut self) {
        self.focused = match self.focused {
            ExportField::Path => ExportField::IncludeCredentials,
            ExportField::IncludeCredentials => ExportField::ShowPassword,
            ExportField::ShowPassword => ExportField::Password,
            ExportField::Password => ExportField::Confirm,
            ExportField::Confirm => ExportField::Path,
        };
    }

    pub fn prev_field(&mut self) {
        self.focused = match self.focused {
            ExportField::Path => ExportField::Confirm,
            ExportField::IncludeCredentials => ExportField::Path,
            ExportField::ShowPassword => ExportField::IncludeCredentials,
            ExportField::Password => ExportField::ShowPassword,
            ExportField::Confirm => ExportField::Password,
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportField {
    Path,
    ShowPassword,
    Password,
}

#[derive(Debug, Clone)]
pub struct ImportDialogState {
    pub path: String,
    pub show_password: bool,
    pub password: String,
    pub focused: ImportField,
    pub error: Option<String>,
    pub path_completions: Vec<String>,
    pub completion_idx: usize,
}

impl ImportDialogState {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Self {
            path: format!("{home}/"),
            show_password: false,
            password: String::new(),
            focused: ImportField::Path,
            error: None,
            path_completions: Vec::new(),
            completion_idx: 0,
        }
    }

    pub fn next_field(&mut self) {
        self.focused = match self.focused {
            ImportField::Path => ImportField::ShowPassword,
            ImportField::ShowPassword => ImportField::Password,
            ImportField::Password => ImportField::Path,
        };
    }

    pub fn complete_path(&mut self) {
        complete_path_field(
            &mut self.path,
            &mut self.path_completions,
            &mut self.completion_idx,
        );
    }

    pub fn reset_completions(&mut self) {
        self.path_completions.clear();
        self.completion_idx = 0;
    }
}

/// Shared path completion logic for export/import dialogs.
/// Scans the filesystem and cycles through matches on repeated Tab.
fn complete_path_field(path: &mut String, completions: &mut Vec<String>, idx: &mut usize) {
    let p = std::path::Path::new(path.as_str());

    let (dir, prefix) = if path.ends_with('/') {
        (std::path::PathBuf::from(path.as_str()), String::new())
    } else {
        let parent = p.parent().unwrap_or(std::path::Path::new("/"));
        let file_prefix = p
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        (parent.to_path_buf(), file_prefix)
    };

    // If we already have completions, cycle through them
    if !completions.is_empty() {
        *idx = (*idx + 1) % completions.len();
        *path = completions[*idx].clone();
        return;
    }

    // Scan directory
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let prefix_lower = prefix.to_lowercase();
    let mut matches: Vec<String> = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !prefix.is_empty() && !name.to_lowercase().starts_with(&prefix_lower) {
            continue;
        }
        if name.starts_with('.') {
            continue;
        }
        let full = dir.join(&name);
        let display = if full.is_dir() {
            format!("{}/", full.display())
        } else {
            full.display().to_string()
        };
        matches.push(display);
    }

    matches.sort();

    if matches.is_empty() {
        return;
    }

    if matches.len() == 1 {
        *path = matches[0].clone();
        completions.clear();
        *idx = 0;
        return;
    }

    *path = matches[0].clone();
    *completions = matches;
    *idx = 0;
}

pub enum GroupMenuAction {
    Rename,
    Delete,
    NewGroup,
}

impl GroupMenuAction {
    pub fn all() -> Vec<Self> {
        vec![Self::Rename, Self::Delete, Self::NewGroup]
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Rename => "Rename group",
            Self::Delete => "Delete group",
            Self::NewGroup => "New group",
        }
    }

    pub fn icon(&self) -> &str {
        match self {
            Self::Rename => "✎",
            Self::Delete => "✗",
            Self::NewGroup => "+",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptNode {
    Collection {
        name: String,
        expanded: bool,
    },
    Script {
        name: String,
        collection: Option<String>,
        file_path: String,
    },
}

pub enum ScriptsMode {
    Normal,
    Insert { buf: String },
    Rename { buf: String, original_path: String },
    ConfirmDelete { path: String },
    PendingD,
    PendingY,
}

pub struct ThemePickerState {
    pub cursor: usize,
}

/// State for the bind variables prompt before query execution.
pub struct BindVariablesState {
    /// Variable names and their values: (name, value)
    pub variables: Vec<(String, String)>,
    /// Currently selected variable index
    pub selected_idx: usize,
    /// The query to execute after substitution
    pub query: String,
    /// Tab ID for execution
    pub tab_id: crate::ui::tabs::TabId,
    /// Start line in editor (for error reporting)
    pub start_line: usize,
    /// Whether to open in a new result tab
    pub new_tab: bool,
}

impl BindVariablesState {
    pub fn next_field(&mut self) {
        if !self.variables.is_empty() {
            self.selected_idx = (self.selected_idx + 1) % self.variables.len();
        }
    }

    pub fn prev_field(&mut self) {
        if !self.variables.is_empty() {
            self.selected_idx = if self.selected_idx == 0 {
                self.variables.len() - 1
            } else {
                self.selected_idx - 1
            };
        }
    }

    /// Build the final query with bind variables replaced by their values.
    pub fn substituted_query(&self) -> String {
        let mut result = self.query.clone();
        for (name, value) in &self.variables {
            let placeholder = format!(":{name}");
            result = result.replace(&placeholder, value);
        }
        result
    }
}

/// State for the script connection picker overlay.
/// Two sections: active connections at top, "Others" collapsible group below.
pub struct ScriptConnPicker {
    pub active: Vec<String>, // Connected (ready to use)
    pub others: Vec<String>, // Saved but not connected
    pub others_expanded: bool,
    pub cursor: usize, // Index into the visible items list
}

impl ScriptConnPicker {
    pub fn new(active: Vec<String>, others: Vec<String>) -> Self {
        Self {
            active,
            others,
            others_expanded: false,
            cursor: 0,
        }
    }

    /// Build the flat visible list for rendering and navigation
    pub fn visible_items(&self) -> Vec<PickerItem> {
        let mut items = Vec::new();
        for name in &self.active {
            items.push(PickerItem::Active(name.clone()));
        }
        if !self.others.is_empty() {
            items.push(PickerItem::OthersHeader);
            if self.others_expanded {
                for name in &self.others {
                    items.push(PickerItem::Other(name.clone()));
                }
            }
        }
        items
    }

    pub fn visible_count(&self) -> usize {
        self.visible_items().len()
    }
}

#[derive(Clone)]
pub enum PickerItem {
    Active(String),
    OthersHeader,
    Other(String),
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
pub enum ConnStatus {
    Disconnected,
    Connecting,
    Connected,
    Failed,
}

#[derive(Debug, Clone)]
pub enum TreeNode {
    Group {
        name: String,
        expanded: bool,
    },
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
        privilege: crate::core::models::ObjectPrivilege,
    },
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CategoryKind {
    Tables,
    Views,
    MaterializedViews,
    Indexes,
    Sequences,
    Types,
    Triggers,
    Packages,
    Procedures,
    Functions,
    Events,
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
    MaterializedView,
    Index,
    Sequence,
    Type,
    Trigger,
    Package,
    Procedure,
    Function,
    Event,
}

impl TreeNode {
    pub fn display_name(&self) -> &str {
        match self {
            TreeNode::Group { name, .. } => name,
            TreeNode::Connection { name, .. } => name,
            TreeNode::Schema { name, .. } => name,
            TreeNode::Category { label, .. } => label,
            TreeNode::Leaf { name, .. } => name,
            TreeNode::Empty => "(empty)",
        }
    }

    pub fn is_expanded(&self) -> bool {
        match self {
            TreeNode::Group { expanded, .. }
            | TreeNode::Connection { expanded, .. }
            | TreeNode::Schema { expanded, .. }
            | TreeNode::Category { expanded, .. } => *expanded,
            TreeNode::Leaf { .. } | TreeNode::Empty => false,
        }
    }

    pub fn toggle_expand(&mut self) {
        match self {
            TreeNode::Group { expanded, .. }
            | TreeNode::Connection { expanded, .. }
            | TreeNode::Schema { expanded, .. }
            | TreeNode::Category { expanded, .. } => *expanded = !*expanded,
            TreeNode::Leaf { .. } | TreeNode::Empty => {}
        }
    }

    pub fn depth(&self) -> usize {
        match self {
            TreeNode::Group { .. } => 0,
            TreeNode::Connection { .. } => 1,
            TreeNode::Schema { .. } => 2,
            TreeNode::Category { .. } => 3,
            TreeNode::Leaf { .. } | TreeNode::Empty => 4,
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

    pub fn adjust_scroll(&mut self, visible_count: usize) {
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
    pub group: String,
    pub group_options: Vec<String>, // available groups for cycling
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
            group: "Default".to_string(),
            group_options: vec!["Default".to_string()],
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
            group: if self.group.is_empty() {
                "Default".to_string()
            } else {
                self.group.clone()
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
            group: config.group.clone(),
            group_options: vec!["Default".to_string()],
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
            1 => &mut self.name, // db_type (handled separately via Ctrl+T)
            2 => &mut self.host,
            3 => &mut self.port,
            4 => &mut self.username,
            5 => &mut self.password,
            6 => &mut self.database,
            7 => &mut self.group, // group (handled separately via Ctrl+G)
            _ => &mut self.name,
        }
    }

    /// Cycle through available groups (Ctrl+G)
    pub fn cycle_group(&mut self) {
        if self.group_options.is_empty() {
            return;
        }
        let current_idx = self
            .group_options
            .iter()
            .position(|g| g == &self.group)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % self.group_options.len();
        self.group = self.group_options[next_idx].clone();
    }

    pub fn next_field(&mut self) {
        self.selected_field = (self.selected_field + 1) % 8;
    }

    pub fn prev_field(&mut self) {
        self.selected_field = if self.selected_field == 0 {
            7
        } else {
            self.selected_field - 1
        };
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
            None => true, // No filter = show all
            Some(set) if set.is_empty() => true,
            Some(set) => set.contains(name),
        }
    }

    pub fn has_filter(&self, key: &str) -> bool {
        self.filters.get(key).is_some_and(|s| !s.is_empty())
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub fn filter_info(&self, key: &str) -> Option<(usize, usize)> {
        if let Some(set) = self.filters.get(key)
            && !set.is_empty()
        {
            return Some((set.len(), self.all_items.len()));
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
    pub focus: Focus,
    pub overlay: Option<Overlay>,

    // Tab workspace
    pub tabs: Vec<WorkspaceTab>,
    pub active_tab_idx: usize,
    pub next_tab_id: u64,

    pub connection_name: Option<String>,
    pub db_type: Option<DatabaseType>,
    pub current_schema: Option<String>,
    pub connected: bool,

    pub tree: Vec<TreeNode>,
    pub tree_state: TreeState,

    /// Generic filter for any tree level
    pub object_filter: ObjectFilterState,

    pub status_message: String,
    pub loading: bool,
    pub loading_since: Option<std::time::Instant>,
    pub pending_d: bool,
    /// True once the primary schema's tables have been loaded (diagnostics safe to run)
    pub metadata_ready: bool,

    pub compile_confirmed: bool,

    // Sidebar object actions
    pub sidebar_rename_buf: String,
    pub sidebar_yank_conn: Option<String>, // yanked connection name for paste/duplicate
    pub sidebar_pending_action: Option<PendingObjectAction>,

    pub connection_form: ConnectionFormState,
    pub conn_menu: ConnMenuState,
    pub script_conn_picker: Option<ScriptConnPicker>,
    pub theme_picker: ThemePickerState,
    pub saved_connections: Vec<ConnectionConfig>,

    // Connection group state
    pub group_menu: GroupMenuState,
    pub group_renaming: Option<String>, // Some(original_name) when renaming a group
    pub group_rename_buf: String,
    pub group_creating: bool, // true when creating a new group

    // Global leader key state (works from any panel)
    pub leader_pending: bool,
    pub leader_b_pending: bool,
    pub leader_w_pending: bool,
    pub leader_s_pending: bool,
    pub leader_leader_pending: bool,
    pub leader_pressed_at: Option<std::time::Instant>,
    pub leader_help_visible: bool,

    // Scripts panel state (Oil-style)
    pub scripts_tree: Vec<ScriptNode>,
    pub scripts_cursor: usize,
    pub scripts_offset: usize,
    pub scripts_mode: ScriptsMode,
    pub scripts_yank: Option<String>,
    pub scripts_save_name: Option<String>,

    // Completion popup
    pub completion: Option<crate::ui::completion::CompletionState>,

    // Diagnostics (LCP)
    pub diagnostics: Vec<crate::ui::diagnostics::Diagnostic>,

    // Column metadata cache for CMP (key: "SCHEMA.TABLE" uppercase)
    pub column_cache: HashMap<String, Vec<Column>>,

    // SQL engine metadata indexes, keyed by connection name
    pub metadata_indexes: HashMap<String, crate::sql_engine::metadata::MetadataIndex>,

    // Bind variables prompt state
    pub bind_variables: Option<BindVariablesState>,

    // Export/Import dialog state
    pub export_dialog: Option<ExportDialogState>,
    pub import_dialog: Option<ImportDialogState>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            mode: Mode::Normal,
            focus: Focus::Sidebar,
            overlay: None,
            tabs: vec![],
            active_tab_idx: 0,
            next_tab_id: 1,
            connection_name: None,
            db_type: None,
            current_schema: None,
            connected: false,
            tree: vec![],
            tree_state: TreeState::new(),
            object_filter: ObjectFilterState::new(),
            status_message: "Ready - press 'a' to add connection, '?' for help".to_string(),
            loading: false,
            loading_since: None,
            pending_d: false,
            metadata_ready: false,
            compile_confirmed: false,
            sidebar_rename_buf: String::new(),
            sidebar_yank_conn: None,
            sidebar_pending_action: None,
            connection_form: ConnectionFormState::new(),
            conn_menu: ConnMenuState {
                conn_name: String::new(),
                cursor: 0,
                is_connected: false,
            },
            script_conn_picker: None,
            theme_picker: ThemePickerState { cursor: 0 },
            saved_connections: vec![],
            group_menu: GroupMenuState {
                group_name: String::new(),
                cursor: 0,
                is_empty: false,
            },
            group_renaming: None,
            group_rename_buf: String::new(),
            group_creating: false,
            leader_pending: false,
            leader_b_pending: false,
            leader_w_pending: false,
            leader_s_pending: false,
            leader_leader_pending: false,
            leader_pressed_at: None,
            leader_help_visible: false,
            scripts_tree: vec![],
            scripts_cursor: 0,
            scripts_offset: 0,
            scripts_mode: ScriptsMode::Normal,
            scripts_yank: None,
            scripts_save_name: None,
            completion: None,
            diagnostics: vec![],
            column_cache: HashMap::new(),
            metadata_indexes: HashMap::new(),
            bind_variables: None,
            export_dialog: None,
            import_dialog: None,
        }
    }

    /// Get the active workspace tab
    pub fn active_tab(&self) -> Option<&WorkspaceTab> {
        self.tabs.get(self.active_tab_idx)
    }

    /// Get the active workspace tab mutably
    pub fn active_tab_mut(&mut self) -> Option<&mut WorkspaceTab> {
        self.tabs.get_mut(self.active_tab_idx)
    }

    /// Find a tab by TabId
    pub fn find_tab(&self, id: TabId) -> Option<&WorkspaceTab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    /// Find a tab by TabId mutably
    /// Collect available group names from tree (for the group selector in connection form)
    pub fn available_groups(&self) -> Vec<String> {
        let mut groups = Vec::new();
        for node in &self.tree {
            if let TreeNode::Group { name, .. } = node
                && !groups.contains(name)
            {
                groups.push(name.clone());
            }
        }
        // If no groups exist, provide "Default" as fallback
        if groups.is_empty() {
            groups.push("Default".to_string());
        }
        groups
    }

    pub fn visible_scripts(&self) -> Vec<(usize, &ScriptNode)> {
        let mut visible = Vec::new();
        let mut i = 0;
        while i < self.scripts_tree.len() {
            let node = &self.scripts_tree[i];
            visible.push((i, node));
            if let ScriptNode::Collection {
                name,
                expanded: false,
            } = node
            {
                // Skip only scripts that belong to this collection
                i += 1;
                while i < self.scripts_tree.len()
                    && let ScriptNode::Script {
                        collection: Some(c),
                        ..
                    } = &self.scripts_tree[i]
                    && c == name
                {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        visible
    }

    #[allow(dead_code)]
    pub fn selected_script_node(&self) -> Option<&ScriptNode> {
        let visible = self.visible_scripts();
        visible.get(self.scripts_cursor).map(|(_, node)| *node)
    }

    #[allow(dead_code)]
    pub fn current_collection(&self) -> Option<String> {
        let visible = self.visible_scripts();
        if let Some((_, node)) = visible.get(self.scripts_cursor) {
            match node {
                ScriptNode::Collection { name, .. } => Some(name.clone()),
                ScriptNode::Script { collection, .. } => collection.clone(),
            }
        } else {
            None
        }
    }

    pub fn find_tab_mut(&mut self, id: TabId) -> Option<&mut WorkspaceTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// Allocate a new unique TabId
    pub fn alloc_tab_id(&mut self) -> TabId {
        let id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        id
    }

    /// Open a tab or focus an existing one with the same object
    pub fn open_or_focus_tab(&mut self, kind: TabKind) -> TabId {
        // Check for existing tab with same object
        if let Some(idx) = self.tabs.iter().position(|t| t.kind.same_object(&kind)) {
            self.active_tab_idx = idx;
            self.focus = Focus::TabContent;

            return self.tabs[idx].id;
        }

        let id = self.alloc_tab_id();
        let tab = match &kind {
            TabKind::Script {
                file_path,
                name,
                conn_name,
            } => WorkspaceTab::new_script(id, name.clone(), file_path.clone(), conn_name.clone()),
            TabKind::Table {
                conn_name,
                schema,
                table,
            } => WorkspaceTab::new_table(id, conn_name.clone(), schema.clone(), table.clone()),
            TabKind::Package {
                conn_name,
                schema,
                name,
            } => WorkspaceTab::new_package(id, conn_name.clone(), schema.clone(), name.clone()),
            TabKind::Function {
                conn_name,
                schema,
                name,
            } => WorkspaceTab::new_function(id, conn_name.clone(), schema.clone(), name.clone()),
            TabKind::Procedure {
                conn_name,
                schema,
                name,
            } => WorkspaceTab::new_procedure(id, conn_name.clone(), schema.clone(), name.clone()),
            TabKind::DbType {
                conn_name,
                schema,
                name,
            } => WorkspaceTab::new_db_type(id, conn_name.clone(), schema.clone(), name.clone()),
            TabKind::Trigger {
                conn_name,
                schema,
                name,
            } => WorkspaceTab::new_trigger(id, conn_name.clone(), schema.clone(), name.clone()),
        };
        self.tabs.push(tab);
        self.active_tab_idx = self.tabs.len() - 1;
        self.focus = Focus::TabContent;

        id
    }

    /// Close the active tab
    pub fn close_active_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        self.tabs.remove(self.active_tab_idx);

        if self.tabs.is_empty() {
            self.active_tab_idx = 0;
            self.focus = Focus::Sidebar;
        } else if self.active_tab_idx >= self.tabs.len() {
            self.active_tab_idx = self.tabs.len() - 1;
        }
    }

    /// Get visible tree nodes, filtered at ALL levels
    pub fn visible_tree(&self) -> Vec<(usize, &TreeNode)> {
        let mut visible = Vec::with_capacity(self.tree.len());
        let mut i = 0;
        let mut current_conn: &str = "";
        // Reusable buffer for filter keys to avoid per-node allocations
        let mut key_buf = String::with_capacity(64);

        while i < self.tree.len() {
            let node = &self.tree[i];

            if let TreeNode::Connection { name, .. } = node {
                current_conn = name;
            }

            // Filter schemas
            if let TreeNode::Schema { name, .. } = node {
                key_buf.clear();
                key_buf.push_str(current_conn);
                key_buf.push_str("::schemas");
                if !self.object_filter.is_enabled(&key_buf, name) {
                    let d = node.depth();
                    i += 1;
                    while i < self.tree.len() && self.tree[i].depth() > d {
                        i += 1;
                    }
                    continue;
                }
            }

            // Filter leaves
            if let TreeNode::Leaf {
                name, schema, kind, ..
            } = node
            {
                let cat_suffix = match kind {
                    LeafKind::Table => "Tables",
                    LeafKind::View => "Views",
                    LeafKind::MaterializedView => "MaterializedViews",
                    LeafKind::Index => "Indexes",
                    LeafKind::Sequence => "Sequences",
                    LeafKind::Type => "Types",
                    LeafKind::Trigger => "Triggers",
                    LeafKind::Package => "Packages",
                    LeafKind::Procedure => "Procedures",
                    LeafKind::Function => "Functions",
                    LeafKind::Event => "Events",
                };
                key_buf.clear();
                key_buf.push_str(current_conn);
                key_buf.push_str("::");
                key_buf.push_str(schema);
                key_buf.push('.');
                key_buf.push_str(cat_suffix);
                if !self.object_filter.is_enabled(&key_buf, name) {
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

    /// Get filter hint for a node if a filter is active at that level.
    /// `conn_name` scopes the filter to the owning connection.
    pub fn filter_hint_for(&self, node: &TreeNode, conn_name: &str) -> Option<String> {
        match node {
            TreeNode::Connection { expanded: true, .. } => {
                let key = format!("{conn_name}::schemas");
                if self.object_filter.has_filter(&key) {
                    let total = self.schema_names_for_conn(conn_name).len();
                    let enabled = self
                        .object_filter
                        .filters
                        .get(&key)
                        .map(|s| s.len())
                        .unwrap_or(total);
                    Some(format!("... ({enabled}/{total} schemas shown)"))
                } else {
                    None
                }
            }
            TreeNode::Category {
                expanded: true,
                schema,
                kind,
                ..
            } => {
                let base_key = kind.filter_key(schema);
                let key = format!("{conn_name}::{base_key}");
                if self.object_filter.has_filter(&key) {
                    let total_in_tree = self.leaves_under_category_count(&base_key);
                    let enabled = self
                        .object_filter
                        .filters
                        .get(&key)
                        .map(|s| s.len())
                        .unwrap_or(total_in_tree);
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
        if parts.len() != 2 {
            return 0;
        }
        let (schema, kind_str) = (parts[0], parts[1]);

        self.tree
            .iter()
            .filter(|n| {
                if let TreeNode::Leaf {
                    schema: s, kind, ..
                } = n
                {
                    let k = format!("{:?}", kind);
                    // LeafKind::Table -> "Table", CategoryKind is "Tables"
                    s == schema && kind_str.starts_with(&k)
                } else {
                    false
                }
            })
            .count()
    }

    pub fn selected_tree_index(&self) -> Option<usize> {
        let visible = self.visible_tree();
        visible.get(self.tree_state.cursor).map(|(idx, _)| *idx)
    }

    /// Walk backwards from a tree index to find its parent Connection name
    pub fn connection_for_tree_idx(&self, idx: usize) -> Option<&str> {
        let mut i = idx;
        loop {
            if let TreeNode::Connection { name, .. } = &self.tree[i] {
                return Some(name.as_str());
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
        None
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
    /// Get schema names scoped to a specific connection
    pub fn schema_names_for_conn(&self, conn_name: &str) -> Vec<String> {
        let mut in_target = false;
        let mut schemas = Vec::new();
        for node in &self.tree {
            match node {
                TreeNode::Connection { name, .. } => {
                    in_target = name == conn_name;
                }
                TreeNode::Schema { name, .. } if in_target => {
                    schemas.push(name.clone());
                }
                _ => {}
            }
        }
        schemas
    }

    /// Get all schema names across all connections (legacy helper)
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
    #[allow(dead_code)]
    pub fn filter_hint(&self, key: &str, total_in_tree: usize) -> Option<String> {
        if let Some(set) = self.object_filter.filters.get(key)
            && !set.is_empty()
            && set.len() < total_in_tree
        {
            return Some(format!("... ({}/{} filtered)", set.len(), total_in_tree));
        }
        None
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
