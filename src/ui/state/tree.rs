use std::collections::{HashMap, HashSet};

use super::connection::PendingObjectAction;

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
    /// A database, for engines that put a catalog level above schemas.
    /// Only SQL Server produces these today; the other drivers report no
    /// catalogs and their trees keep the Connection → Schema shape.
    Catalog {
        name: String,
        expanded: bool,
    },
    Schema {
        name: String,
        /// Owning database, when the connection has a catalog level. `None`
        /// keeps this node one row shallower, directly under the connection.
        catalog: Option<String>,
        expanded: bool,
    },
    Category {
        label: String,
        schema: String,
        catalog: Option<String>,
        kind: CategoryKind,
        expanded: bool,
    },
    Leaf {
        name: String,
        schema: String,
        catalog: Option<String>,
        kind: LeafKind,
        valid: bool,
        privilege: crate::core::models::ObjectPrivilege,
    },
    /// Placeholder shown when a node has no children.
    ///
    /// It carries its own depth because it can hang off a category, a schema
    /// or a connection, and every subtree scan in the sidebar walks by
    /// comparing depths — a placeholder shallower than its parent would cut
    /// those scans short and strand the nodes below it.
    Empty {
        depth: usize,
    },
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
            TreeNode::Catalog { name, .. } => name,
            TreeNode::Schema { name, .. } => name,
            TreeNode::Category { label, .. } => label,
            TreeNode::Leaf { name, .. } => name,
            TreeNode::Empty { .. } => "(empty)",
        }
    }

    pub fn is_expanded(&self) -> bool {
        match self {
            TreeNode::Group { expanded, .. }
            | TreeNode::Connection { expanded, .. }
            | TreeNode::Catalog { expanded, .. }
            | TreeNode::Schema { expanded, .. }
            | TreeNode::Category { expanded, .. } => *expanded,
            TreeNode::Leaf { .. } | TreeNode::Empty { .. } => false,
        }
    }

    pub fn toggle_expand(&mut self) {
        match self {
            TreeNode::Group { expanded, .. }
            | TreeNode::Connection { expanded, .. }
            | TreeNode::Catalog { expanded, .. }
            | TreeNode::Schema { expanded, .. }
            | TreeNode::Category { expanded, .. } => *expanded = !*expanded,
            TreeNode::Leaf { .. } | TreeNode::Empty { .. } => {}
        }
    }

    /// Indent level. Nodes under a catalog sit one row deeper, so the depth
    /// of everything below the connection depends on whether a catalog owns it.
    pub fn depth(&self) -> usize {
        let catalog_offset = |catalog: &Option<String>| usize::from(catalog.is_some());
        match self {
            TreeNode::Group { .. } => 0,
            TreeNode::Connection { .. } => 1,
            TreeNode::Catalog { .. } => 2,
            TreeNode::Schema { catalog, .. } => 2 + catalog_offset(catalog),
            TreeNode::Category { catalog, .. } => 3 + catalog_offset(catalog),
            TreeNode::Leaf { catalog, .. } => 4 + catalog_offset(catalog),
            TreeNode::Empty { depth } => *depth,
        }
    }

    /// The database this node belongs to, if the connection has a catalog level.
    pub fn catalog(&self) -> Option<&str> {
        match self {
            TreeNode::Catalog { name, .. } => Some(name.as_str()),
            TreeNode::Schema { catalog, .. }
            | TreeNode::Category { catalog, .. }
            | TreeNode::Leaf { catalog, .. } => catalog.as_deref(),
            _ => None,
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

// --- Sidebar State ---

pub struct SidebarState {
    pub tree: Vec<TreeNode>,
    pub tree_state: TreeState,
    /// Generic filter for any tree level
    pub object_filter: ObjectFilterState,
    // Sidebar object actions
    pub rename_buf: String,
    pub yank_conn: Option<String>, // yanked connection name for paste/duplicate
    pub pending_action: Option<PendingObjectAction>,
    /// Index: UPPER(table/view name) → schema. Rebuilt on tree mutations.
    pub table_schema_index: std::collections::HashMap<String, String>,
}

impl SidebarState {
    pub fn new() -> Self {
        Self {
            tree: vec![],
            tree_state: TreeState::new(),
            object_filter: ObjectFilterState::new(),
            rename_buf: String::new(),
            yank_conn: None,
            pending_action: None,
            table_schema_index: std::collections::HashMap::new(),
        }
    }

    /// Rebuild the table→schema index from the current tree.
    #[allow(dead_code)]
    pub fn rebuild_table_index(&mut self) {
        self.table_schema_index.clear();
        for node in &self.tree {
            if let TreeNode::Leaf {
                name, schema, kind, ..
            } = node
                && matches!(kind, LeafKind::Table | LeafKind::View)
            {
                self.table_schema_index
                    .entry(name.to_uppercase())
                    .or_insert_with(|| schema.clone());
            }
        }
    }
}

#[cfg(test)]
mod depth_tests {
    use super::*;
    use crate::core::models::ObjectPrivilege;

    fn schema(catalog: Option<&str>) -> TreeNode {
        TreeNode::Schema {
            name: "dbo".to_string(),
            catalog: catalog.map(str::to_string),
            expanded: false,
        }
    }

    fn category(catalog: Option<&str>) -> TreeNode {
        TreeNode::Category {
            label: "Views".to_string(),
            schema: "dbo".to_string(),
            catalog: catalog.map(str::to_string),
            kind: CategoryKind::Views,
            expanded: false,
        }
    }

    fn leaf(catalog: Option<&str>) -> TreeNode {
        TreeNode::Leaf {
            name: "v_thing".to_string(),
            schema: "dbo".to_string(),
            catalog: catalog.map(str::to_string),
            kind: LeafKind::View,
            valid: true,
            privilege: ObjectPrivilege::Full,
        }
    }

    #[test]
    fn without_a_catalog_depths_are_unchanged() {
        assert_eq!(schema(None).depth(), 2);
        assert_eq!(category(None).depth(), 3);
        assert_eq!(leaf(None).depth(), 4);
    }

    #[test]
    fn a_catalog_shifts_everything_below_it_down_one() {
        assert_eq!(
            TreeNode::Catalog {
                name: "db".to_string(),
                expanded: false
            }
            .depth(),
            2
        );
        assert_eq!(schema(Some("db")).depth(), 3);
        assert_eq!(category(Some("db")).depth(), 4);
        assert_eq!(leaf(Some("db")).depth(), 5);
    }

    /// Every subtree scan in the sidebar walks with `depth() > parent_depth`.
    /// A placeholder that is not deeper than its parent ends those scans early,
    /// so the stale nodes survive and later inserts land at the wrong index —
    /// which is how `(empty)` rows used to pile up on each expand/collapse.
    #[test]
    fn empty_placeholder_is_deeper_than_the_category_it_fills() {
        for catalog in [None, Some("db")] {
            let parent = category(catalog);
            let placeholder = TreeNode::Empty {
                depth: parent.depth() + 1,
            };
            assert!(
                placeholder.depth() > parent.depth(),
                "placeholder must be scannable as a child (catalog: {catalog:?})"
            );
        }
    }

    #[test]
    fn a_stale_placeholder_is_drained_with_the_rest_of_the_subtree() {
        // Category → its placeholder, in the catalog-bearing shape where the
        // old fixed depth of 4 collided with the category's own depth.
        let tree = [category(Some("db")), TreeNode::Empty { depth: 5 }];
        let parent_depth = tree[0].depth();
        let mut end = 1;
        while end < tree.len() && tree[end].depth() > parent_depth {
            end += 1;
        }
        assert_eq!(end, 2, "the placeholder should be inside the drained range");
    }
}
