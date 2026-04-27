mod connection;
mod dialogs;
mod scripts;
mod tree;

pub use connection::*;
pub use dialogs::*;
pub use scripts::*;
pub use tree::*;

use std::collections::HashMap;

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

/// A group of tabs displayed together in one half of a vertical split.
#[derive(Debug, Clone)]
pub struct TabGroup {
    pub tab_ids: Vec<TabId>,
    pub active_idx: usize,
}

impl TabGroup {
    pub fn new(tab_ids: Vec<TabId>, active_idx: usize) -> Self {
        Self {
            tab_ids,
            active_idx,
        }
    }

    pub fn active_tab_id(&self) -> Option<TabId> {
        self.tab_ids.get(self.active_idx).copied()
    }
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
    ConfirmDeleteConnection { name: String },
    ConfirmDropObject,
    RenameObject,
    ConfirmCompile,
    ExportDialog,
    ImportDialog,
}

// --- Oil Floating Navigator State ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OilPane {
    Explorer,
    Scripts,
}

pub struct OilState {
    pub pane: OilPane,
    pub previous_focus: Focus,
}

impl OilState {
    pub fn new(previous_focus: Focus) -> Self {
        Self {
            pane: OilPane::Explorer,
            previous_focus,
        }
    }
}

// --- Leader State ---

pub struct LeaderState {
    pub pending: bool,
    pub b_pending: bool,
    pub w_pending: bool,
    pub s_pending: bool,
    pub f_pending: bool,
    pub q_pending: bool,
    pub leader_pending: bool,
    pub pressed_at: Option<std::time::Instant>,
    pub help_visible: bool,
}

impl LeaderState {
    pub fn new() -> Self {
        Self {
            pending: false,
            b_pending: false,
            w_pending: false,
            s_pending: false,
            f_pending: false,
            q_pending: false,
            leader_pending: false,
            pressed_at: None,
            help_visible: false,
        }
    }

    /// Reset all leader key state
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.pending = false;
        self.b_pending = false;
        self.w_pending = false;
        self.s_pending = false;
        self.f_pending = false;
        self.q_pending = false;
        self.leader_pending = false;
        self.pressed_at = None;
        self.help_visible = false;
    }
}

// --- Engine State ---

pub struct EngineState {
    /// Completion popup
    pub completion: Option<crate::ui::completion::CompletionState>,
    /// Diagnostics
    pub diagnostics: Vec<crate::ui::diagnostics::Diagnostic>,
    /// Last time diagnostics were re-run. Used for the in-insert-mode
    /// debounce so we don't re-parse on every keystroke.
    pub last_diagnostic_run: Option<std::time::Instant>,
    /// Column metadata cache for CMP (key: "SCHEMA.TABLE" uppercase)
    pub column_cache: HashMap<String, Vec<Column>>,
    /// SQL engine metadata indexes, keyed by connection name
    pub metadata_indexes: HashMap<String, crate::sql_engine::metadata::MetadataIndex>,
    /// Diagnostic hover popup: (row, message) shown with K key
    pub diagnostic_hover: Option<(usize, String)>,
    /// Diagnostic list panel visible
    pub diagnostic_list_visible: bool,
    /// Diagnostic list cursor position
    pub diagnostic_list_cursor: usize,
    /// Generation counter for server diagnostics. Incremented each time a
    /// server compile-check is dispatched; stale results (from an earlier
    /// generation) are silently dropped.
    pub server_diag_generation: u64,
    /// Last time a server diagnostic request was dispatched. Used for
    /// debouncing so rapid Insert→Normal transitions don't hammer the DB.
    pub last_server_diag_dispatch: Option<std::time::Instant>,
    /// Pending server diagnostic request: (sql, conn_name). Set by the editor
    /// event handler and consumed by the app main loop, which spawns the async
    /// task. This sidesteps the single-action return limitation.
    pub pending_server_diag: Option<(String, String)>,
    /// Cache: last analyzed block (lines, cursor_row, cursor_col) → SemanticContext.
    /// Avoids re-parsing when the block and cursor haven't changed.
    pub analysis_cache: Option<AnalysisCache>,
}

/// Cached result of semantic analysis for a query block.
pub struct AnalysisCache {
    pub block_lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub context: crate::sql_engine::context::SemanticContext,
}

impl EngineState {
    pub fn new() -> Self {
        Self {
            completion: None,
            diagnostics: vec![],
            last_diagnostic_run: None,
            column_cache: HashMap::new(),
            metadata_indexes: HashMap::new(),
            diagnostic_hover: None,
            diagnostic_list_visible: false,
            diagnostic_list_cursor: 0,
            server_diag_generation: 0,
            last_server_diag_dispatch: None,
            pending_server_diag: None,
            analysis_cache: None,
        }
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
    /// Tab groups (vertical split). None = single view, Some = two groups.
    /// groups[0] = left, groups[1] = right.
    pub groups: Option<[TabGroup; 2]>,
    /// Which group is focused (0 or 1). Only meaningful when groups is Some.
    pub active_group: usize,
    /// Transient: which group is currently being rendered (set during render loop).
    /// Used by render_tab_bar to differentiate the focused group's styling.
    pub rendering_group: Option<usize>,

    pub conn: ConnectionState,

    pub sidebar: SidebarState,
    pub sidebar_visible: bool,
    pub oil: Option<OilState>,

    pub status_message: String,
    pub loading: bool,
    pub loading_since: Option<std::time::Instant>,
    pub pending_d: bool,
    /// True once the primary schema's tables have been loaded (diagnostics safe to run)
    pub metadata_ready: bool,

    pub compile_confirmed: bool,

    pub dialogs: DialogState,

    pub leader: LeaderState,

    pub scripts: ScriptsState,

    pub engine: EngineState,

    /// Resolved keybindings (defaults merged with user overrides from
    /// ~/.config/dbtui/keybindings.toml). The event handlers query this
    /// via `bindings.matches(Context::X, "action", &key)` and the help
    /// screens read it via `bindings.keys_for(...)`. The handler
    /// migration is incremental; until every handler reads from this
    /// field the dead-code warning would be misleading.
    #[allow(dead_code)]
    pub bindings: crate::keybindings::KeyBindings,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            mode: Mode::Normal,
            focus: Focus::TabContent,
            overlay: None,
            tabs: vec![],
            active_tab_idx: 0,
            next_tab_id: 1,
            groups: None,
            active_group: 0,
            rendering_group: None,
            conn: ConnectionState::new(),
            sidebar: SidebarState::new(),
            sidebar_visible: false,
            oil: None,
            status_message: "Ready - press 'a' to add connection, '?' for help".to_string(),
            loading: false,
            loading_since: None,
            pending_d: false,
            metadata_ready: false,
            compile_confirmed: false,
            dialogs: DialogState::new(),
            leader: LeaderState::new(),
            scripts: ScriptsState::new(),
            engine: EngineState::new(),
            bindings: crate::keybindings::KeyBindings::defaults(),
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

    /// Get the focused group's tab IDs (or all tab IDs if no split)
    #[allow(dead_code)]
    pub fn focused_group_tab_ids(&self) -> Vec<TabId> {
        match &self.groups {
            Some(groups) => groups[self.active_group].tab_ids.clone(),
            None => self.tabs.iter().map(|t| t.id).collect(),
        }
    }

    /// Get the focused group's active tab ID
    pub fn focused_tab_id(&self) -> Option<TabId> {
        match &self.groups {
            Some(groups) => groups[self.active_group].active_tab_id(),
            None => self.tabs.get(self.active_tab_idx).map(|t| t.id),
        }
    }

    /// Create a vertical split with an empty right group. The current tabs all
    /// stay in group 0; group 1 starts empty and becomes focused. Used by oil
    /// when opening with Ctrl+S so the new object lands in a fresh group.
    pub fn create_empty_split(&mut self) {
        if self.groups.is_some() {
            // Already split — just switch focus to the right group
            self.active_group = 1;
            return;
        }
        let all_ids: Vec<TabId> = self.tabs.iter().map(|t| t.id).collect();
        let g0 = TabGroup::new(all_ids, self.active_tab_idx);
        let g1 = TabGroup::new(Vec::new(), 0);
        self.groups = Some([g0, g1]);
        self.active_group = 1;
    }

    /// Sync `active_tab_idx` to point at the focused group's active tab.
    /// No-op when there's no split.
    pub fn sync_active_tab_idx(&mut self) {
        if let Some(focused_id) = self.focused_tab_id()
            && let Some(idx) = self.tabs.iter().position(|t| t.id == focused_id)
        {
            self.active_tab_idx = idx;
        }
    }

    /// Find a tab by TabId
    pub fn find_tab(&self, id: TabId) -> Option<&WorkspaceTab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    /// Find a tab by TabId mutably
    /// Collect available group names from tree (for the group selector in connection form)
    pub fn available_groups(&self) -> Vec<String> {
        let mut groups = Vec::new();
        for node in &self.sidebar.tree {
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
        // When split is active, only consider tabs already in the focused group
        // for deduplication. This way each group is independent — opening the same
        // object from a different group creates a new instance.
        let candidate_tab = if let Some(groups) = &self.groups {
            let focused_ids = &groups[self.active_group].tab_ids;
            self.tabs
                .iter()
                .position(|t| focused_ids.contains(&t.id) && t.kind.same_object(&kind))
        } else {
            self.tabs.iter().position(|t| t.kind.same_object(&kind))
        };

        if let Some(idx) = candidate_tab {
            let existing_id = self.tabs[idx].id;
            self.active_tab_idx = idx;
            self.focus = Focus::TabContent;

            // Sync group's active_idx to point at the existing tab
            if let Some(groups) = self.groups.as_mut()
                && let Some(pos) = groups[self.active_group]
                    .tab_ids
                    .iter()
                    .position(|id| *id == existing_id)
            {
                groups[self.active_group].active_idx = pos;
            }

            return existing_id;
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

        // Append to focused group when split is active
        if let Some(groups) = self.groups.as_mut() {
            groups[self.active_group].tab_ids.push(id);
            groups[self.active_group].active_idx = groups[self.active_group].tab_ids.len() - 1;
        }

        id
    }

    /// Close the active tab
    pub fn close_active_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }

        // Determine which TabId to close
        let closing_id = match &self.groups {
            Some(groups) => match groups[self.active_group].active_tab_id() {
                Some(id) => id,
                None => return,
            },
            None => self.tabs[self.active_tab_idx].id,
        };

        // If split is active, remove from focused group only.
        // Tab is removed from state.tabs only if no group still references it.
        if let Some(groups) = self.groups.as_mut() {
            let g = &mut groups[self.active_group];
            if let Some(pos) = g.tab_ids.iter().position(|id| *id == closing_id) {
                g.tab_ids.remove(pos);
                if g.active_idx >= g.tab_ids.len() && !g.tab_ids.is_empty() {
                    g.active_idx = g.tab_ids.len() - 1;
                }
            }

            // Check if other group still references this tab
            let other = 1 - self.active_group;
            let still_referenced = groups[other].tab_ids.contains(&closing_id);

            // If focused group is now empty, destroy split and merge other group
            let focused_empty = groups[self.active_group].tab_ids.is_empty();
            if focused_empty {
                let surviving = groups[other].clone();
                self.groups = None;
                self.active_group = 0;
                // Reorder state.tabs to match surviving group order
                self.reorder_tabs_to_group(&surviving);
                // Remove the closed tab if not referenced anywhere
                if !still_referenced
                    && let Some(idx) = self.tabs.iter().position(|t| t.id == closing_id)
                {
                    self.tabs.remove(idx);
                }
                self.sync_active_tab_idx();
                if self.tabs.is_empty() {
                    self.active_tab_idx = 0;
                    self.focus = Focus::Sidebar;
                }
            } else {
                if !still_referenced
                    && let Some(idx) = self.tabs.iter().position(|t| t.id == closing_id)
                {
                    self.tabs.remove(idx);
                }
                self.sync_active_tab_idx();
            }
            return;
        }

        // No split: existing flat behavior
        if let Some(idx) = self.tabs.iter().position(|t| t.id == closing_id) {
            self.tabs.remove(idx);
        }
        if self.tabs.is_empty() {
            self.active_tab_idx = 0;
            self.focus = Focus::Sidebar;
        } else if self.active_tab_idx >= self.tabs.len() {
            self.active_tab_idx = self.tabs.len() - 1;
        }
    }

    /// Reorder state.tabs to match the order in the given group, removing tabs not in the group.
    /// Used when destroying a split — the surviving group becomes the new flat order.
    fn reorder_tabs_to_group(&mut self, group: &TabGroup) {
        let mut new_tabs: Vec<WorkspaceTab> = Vec::with_capacity(group.tab_ids.len());
        for id in &group.tab_ids {
            if let Some(pos) = self.tabs.iter().position(|t| t.id == *id) {
                new_tabs.push(self.tabs.remove(pos));
            }
        }
        // Append any remaining tabs (shouldn't happen normally, but safe)
        new_tabs.append(&mut self.tabs);
        self.tabs = new_tabs;
        self.active_tab_idx = group.active_idx.min(self.tabs.len().saturating_sub(1));
    }

    /// Get visible tree nodes, filtered at ALL levels
    pub fn visible_tree(&self) -> Vec<(usize, &TreeNode, &str)> {
        let mut visible = Vec::with_capacity(self.sidebar.tree.len());
        let mut i = 0;
        let mut current_conn: &str = "";
        // Reusable buffer for filter keys to avoid per-node allocations
        let mut key_buf = String::with_capacity(64);

        while i < self.sidebar.tree.len() {
            let node = &self.sidebar.tree[i];

            if let TreeNode::Connection { name, .. } = node {
                current_conn = name;
            }

            // Filter schemas
            if let TreeNode::Schema { name, .. } = node {
                key_buf.clear();
                key_buf.push_str(current_conn);
                key_buf.push_str("::schemas");
                if !self.sidebar.object_filter.is_enabled(&key_buf, name) {
                    let d = node.depth();
                    i += 1;
                    while i < self.sidebar.tree.len() && self.sidebar.tree[i].depth() > d {
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
                if !self.sidebar.object_filter.is_enabled(&key_buf, name) {
                    i += 1;
                    continue;
                }
            }

            visible.push((i, node, current_conn));

            if !node.is_expanded() {
                let d = node.depth();
                i += 1;
                while i < self.sidebar.tree.len() && self.sidebar.tree[i].depth() > d {
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
                if self.sidebar.object_filter.has_filter(&key) {
                    let total = self.schema_names_for_conn(conn_name).len();
                    let enabled = self
                        .sidebar
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
                if self.sidebar.object_filter.has_filter(&key) {
                    let total_in_tree = self.leaves_under_category_count(&base_key);
                    let enabled = self
                        .sidebar
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

        self.sidebar
            .tree
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
        visible
            .get(self.sidebar.tree_state.cursor)
            .map(|(idx, _, _)| *idx)
    }

    /// Walk backwards from a tree index to find its parent Connection name
    pub fn connection_for_tree_idx(&self, idx: usize) -> Option<&str> {
        let mut i = idx;
        loop {
            if let TreeNode::Connection { name, .. } = &self.sidebar.tree[i] {
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
        let cat_depth = self.sidebar.tree[cat_idx].depth();
        let mut i = cat_idx + 1;
        while i < self.sidebar.tree.len() && self.sidebar.tree[i].depth() > cat_depth {
            if let TreeNode::Leaf { name, .. } = &self.sidebar.tree[i] {
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
        for node in &self.sidebar.tree {
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
        self.sidebar
            .tree
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
        if let Some(set) = self.sidebar.object_filter.filters.get(key)
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
