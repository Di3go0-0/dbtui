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
    /// Empty values are substituted as `NULL`.
    pub fn substituted_query(&self) -> String {
        let mut result = self.query.clone();
        for (name, value) in &self.variables {
            let placeholder = format!(":{name}");
            let substitution = if value.is_empty() { "NULL" } else { value };
            result = result.replace(&placeholder, substitution);
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

// --- Scripts State ---

pub struct ScriptsState {
    pub tree: Vec<ScriptNode>,
    pub cursor: usize,
    pub offset: usize,
    pub mode: ScriptsMode,
    pub yank: Option<String>,
    pub save_name: Option<String>,
}

impl ScriptsState {
    pub fn new() -> Self {
        Self {
            tree: vec![],
            cursor: 0,
            offset: 0,
            mode: ScriptsMode::Normal,
            yank: None,
            save_name: None,
        }
    }

    /// Walk the flat `tree` vec and return every node whose full ancestry
    /// chain is currently expanded. Collection names are full paths
    /// (`"parent"` / `"parent/child"`) so the ancestry check boils down to
    /// testing each prefix of the path.
    pub fn visible_scripts(&self) -> Vec<(usize, &ScriptNode)> {
        // Pre-compute the expand state of every collection path so the
        // lookup is O(1) per ancestor check.
        let mut expanded: std::collections::HashMap<&str, bool> = std::collections::HashMap::new();
        for node in &self.tree {
            if let ScriptNode::Collection { name, expanded: e } = node {
                expanded.insert(name.as_str(), *e);
            }
        }

        // A collection path is "visible" iff every *strict* ancestor
        // (not the path itself) is expanded. The node itself being
        // collapsed just hides *its* children, not itself.
        let all_ancestors_expanded = |path: &str| -> bool {
            let mut start = 0;
            while let Some(pos) = path[start..].find('/') {
                let ancestor = &path[..start + pos];
                if expanded.get(ancestor).copied() != Some(true) {
                    return false;
                }
                start += pos + 1;
            }
            true
        };

        let mut visible = Vec::new();
        for (i, node) in self.tree.iter().enumerate() {
            match node {
                ScriptNode::Collection { name, .. } => {
                    if all_ancestors_expanded(name) {
                        visible.push((i, node));
                    }
                }
                ScriptNode::Script {
                    collection, name, ..
                } => {
                    match collection {
                        Some(coll) => {
                            // Script visible iff its collection and every
                            // ancestor of that collection are expanded.
                            if expanded.get(coll.as_str()).copied() == Some(true)
                                && all_ancestors_expanded(coll)
                            {
                                visible.push((i, node));
                            }
                        }
                        None => {
                            // Root-level script — always visible.
                            let _ = name; // keep borrow alive
                            visible.push((i, node));
                        }
                    }
                }
            }
        }
        visible
    }

    #[allow(dead_code)]
    pub fn selected_script_node(&self) -> Option<&ScriptNode> {
        let visible = self.visible_scripts();
        visible.get(self.cursor).map(|(_, node)| *node)
    }

    #[allow(dead_code)]
    /// Resolve where a "create new" operation should go.
    ///
    /// Rules (the cursor's intent should be obvious):
    /// - Cursor on an EXPANDED collection → create inside it
    /// - Cursor on a script whose parent collection is currently expanded
    ///   → create inside that parent
    /// - Cursor on a COLLAPSED collection → create at root
    /// - No cursor / no selection → create at root
    pub fn current_collection(&self) -> Option<String> {
        let visible = self.visible_scripts();
        let (_, node) = visible.get(self.cursor)?;
        match node {
            ScriptNode::Collection { name, expanded } => {
                if *expanded {
                    Some(name.clone())
                } else {
                    None
                }
            }
            // The script is visible, so its parent collection is necessarily
            // expanded — return that collection (or None for root-level scripts).
            ScriptNode::Script { collection, .. } => collection.clone(),
        }
    }
}
