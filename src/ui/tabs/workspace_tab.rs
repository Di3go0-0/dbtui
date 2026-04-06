use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::core::models::{Column, PackageContent, QueryResult};
use crate::core::virtual_fs::SyncState;
use vimltui::VimEditor;
use vimltui::VimModeConfig;

/// A single cell modification
pub struct CellEdit {
    pub col: usize,
    #[allow(dead_code)]
    pub original: String,
    pub value: String,
}

/// Pending change on a row
pub enum RowChange {
    Modified { edits: Vec<CellEdit> },
    New { values: Vec<String> },
    Deleted,
}

/// Unique identifier for each open tab
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

/// Which sub-pane has focus in a script split view
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubFocus {
    Editor,    // The main script editor (top)
    Results,   // The data grid or error editor (bottom-left for errors)
    QueryView, // The query editor in error view (bottom-right)
}

/// A single result tab inside a script tab
pub struct ResultTab {
    pub label: String,
    pub result: QueryResult,
    pub error_editor: Option<VimEditor>, // Read-only vim for error message
    pub query_editor: Option<VimEditor>, // Read-only vim for the failed SQL query
    pub scroll_row: usize,
    pub selected_row: usize,
    pub selected_col: usize,
    pub visible_height: usize,
    pub selection_anchor: Option<(usize, usize)>,
}

/// What kind of item a tab represents
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabKind {
    Script {
        file_path: Option<String>,
        name: String,
        conn_name: Option<String>,
    },
    Table {
        conn_name: String,
        schema: String,
        table: String,
    },
    Package {
        conn_name: String,
        schema: String,
        name: String,
    },
    Function {
        conn_name: String,
        schema: String,
        name: String,
    },
    Procedure {
        conn_name: String,
        schema: String,
        name: String,
    },
    DbType {
        conn_name: String,
        schema: String,
        name: String,
    },
    Trigger {
        conn_name: String,
        schema: String,
        name: String,
    },
}

impl TabKind {
    pub fn display_name(&self) -> &str {
        match self {
            TabKind::Script { name, .. } => name,
            TabKind::Table { table, .. } => table,
            TabKind::Package { name, .. } => name,
            TabKind::Function { name, .. } => name,
            TabKind::Procedure { name, .. } => name,
            TabKind::DbType { name, .. } => name,
            TabKind::Trigger { name, .. } => name,
        }
    }

    pub fn kind_label(&self) -> &str {
        match self {
            TabKind::Script { .. } => "script",
            TabKind::Table { .. } => "table",
            TabKind::Package { .. } => "package",
            TabKind::Function { .. } => "function",
            TabKind::Procedure { .. } => "procedure",
            TabKind::DbType { .. } => "type",
            TabKind::Trigger { .. } => "trigger",
        }
    }

    pub fn conn_name(&self) -> Option<&str> {
        match self {
            TabKind::Script { conn_name, .. } => conn_name.as_deref(),
            TabKind::Table { conn_name, .. } => Some(conn_name),
            TabKind::Package { conn_name, .. } => Some(conn_name),
            TabKind::Function { conn_name, .. } => Some(conn_name),
            TabKind::Procedure { conn_name, .. } => Some(conn_name),
            TabKind::DbType { conn_name, .. } => Some(conn_name),
            TabKind::Trigger { conn_name, .. } => Some(conn_name),
        }
    }

    pub fn icon(&self) -> &str {
        match self {
            TabKind::Script { .. } => "S",
            TabKind::Table { .. } => "T",
            TabKind::Package { .. } => "P",
            TabKind::Function { .. } => "\u{03bb}",  // λ
            TabKind::Procedure { .. } => "\u{0192}", // ƒ
            TabKind::DbType { .. } => "\u{22a4}",    // ⊤
            TabKind::Trigger { .. } => "\u{26a1}",   // ⚡
        }
    }

    /// Check if two TabKinds refer to the same object (for deduplication)
    pub fn same_object(&self, other: &TabKind) -> bool {
        match (self, other) {
            (
                TabKind::Table {
                    conn_name: c1,
                    schema: s1,
                    table: t1,
                },
                TabKind::Table {
                    conn_name: c2,
                    schema: s2,
                    table: t2,
                },
            ) => c1 == c2 && s1 == s2 && t1 == t2,
            (
                TabKind::Package {
                    conn_name: c1,
                    schema: s1,
                    name: n1,
                },
                TabKind::Package {
                    conn_name: c2,
                    schema: s2,
                    name: n2,
                },
            ) => c1 == c2 && s1 == s2 && n1 == n2,
            (
                TabKind::Function {
                    conn_name: c1,
                    schema: s1,
                    name: n1,
                },
                TabKind::Function {
                    conn_name: c2,
                    schema: s2,
                    name: n2,
                },
            ) => c1 == c2 && s1 == s2 && n1 == n2,
            (
                TabKind::Procedure {
                    conn_name: c1,
                    schema: s1,
                    name: n1,
                },
                TabKind::Procedure {
                    conn_name: c2,
                    schema: s2,
                    name: n2,
                },
            ) => c1 == c2 && s1 == s2 && n1 == n2,
            (
                TabKind::DbType {
                    conn_name: c1,
                    schema: s1,
                    name: n1,
                },
                TabKind::DbType {
                    conn_name: c2,
                    schema: s2,
                    name: n2,
                },
            ) => c1 == c2 && s1 == s2 && n1 == n2,
            (
                TabKind::Trigger {
                    conn_name: c1,
                    schema: s1,
                    name: n1,
                },
                TabKind::Trigger {
                    conn_name: c2,
                    schema: s2,
                    name: n2,
                },
            ) => c1 == c2 && s1 == s2 && n1 == n2,
            (TabKind::Script { name: n1, .. }, TabKind::Script { name: n2, .. }) => n1 == n2,
            _ => false,
        }
    }
}

/// Sub-views available within each tab kind
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubView {
    // Table
    TableData,
    TableProperties,
    TableDDL,
    // Package
    PackageBody,
    PackageDeclaration,
    PackageFunctions,
    PackageProcedures,
    // Type
    TypeAttributes,
    TypeMethods,
    TypeDeclaration,
    TypeBody,
    // Trigger
    TriggerColumns,
    TriggerDeclaration,
}

impl SubView {
    pub fn label(&self) -> &str {
        match self {
            SubView::TableData => "Data",
            SubView::TableProperties => "Properties",
            SubView::TableDDL => "DDL",
            SubView::PackageBody => "Body",
            SubView::PackageDeclaration => "Declaration",
            SubView::PackageFunctions => "Functions",
            SubView::PackageProcedures => "Procedures",
            SubView::TypeAttributes => "Attributes",
            SubView::TypeMethods => "Methods",
            SubView::TypeDeclaration => "Declaration",
            SubView::TypeBody => "Body",
            SubView::TriggerColumns => "Columns",
            SubView::TriggerDeclaration => "Declaration",
        }
    }
}

/// A workspace tab with all its state
pub struct WorkspaceTab {
    pub id: TabId,
    pub kind: TabKind,
    pub active_sub_view: Option<SubView>,

    // --- Table / Grid state ---
    pub query_result: Option<QueryResult>, // Active grid data (swapped by sub-view)
    pub table_data_result: Option<QueryResult>, // Original table data (preserved across sub-view switches)
    pub columns: Vec<Column>,
    pub result_tabs: Vec<ResultTab>, // Script result tabs
    pub active_result_idx: usize,    // Which result tab is active
    pub grid_scroll_row: usize,
    pub grid_scroll_col: usize,
    pub grid_selected_row: usize,
    pub grid_selected_col: usize,
    pub grid_visible_height: usize,
    pub grid_selection_anchor: Option<(usize, usize)>, // (row, col) where visual selection started
    pub grid_visual_mode: bool,                        // true = visual selection active
    pub grid_on_header: bool,                          // true = cursor is on the header row
    pub grid_focused: bool,                            // legacy: true if any bottom pane has focus
    pub streaming: bool,                               // true while query is streaming batches
    pub streaming_since: Option<std::time::Instant>,   // when streaming started
    pub streaming_abort: Option<tokio::task::AbortHandle>, // abort handle for cancellation
    pub sub_focus: SubFocus,                           // which sub-pane has focus
    pub ddl_editor: Option<VimEditor>,

    // --- Inline editing state ---
    pub grid_error_editor: Option<VimEditor>, // read-only error message pane
    pub grid_query_editor: Option<VimEditor>, // read-only failed SQL pane
    pub grid_changes: HashMap<usize, RowChange>, // pending changes keyed by row index
    pub grid_editing: Option<(usize, usize)>, // (row, col) being edited inline
    pub grid_edit_buffer: String,             // text buffer for inline editing
    pub grid_edit_cursor: usize,              // cursor position in edit buffer

    // --- Package state ---
    pub package_content: Option<PackageContent>,
    pub body_editor: Option<VimEditor>,
    pub decl_editor: Option<VimEditor>,
    pub package_functions: Vec<String>,
    pub package_procedures: Vec<String>,
    pub package_list_cursor: usize,

    // --- Type state ---
    pub type_attributes: Option<QueryResult>,
    pub type_methods: Option<QueryResult>,

    // --- Trigger state ---
    pub trigger_columns: Option<QueryResult>,

    // --- Script / Function / Procedure state ---
    pub editor: Option<VimEditor>,

    // --- Diff signs: original content for comparison ---
    pub original_decl: Option<String>,
    pub original_body: Option<String>,
    pub original_source: Option<String>,

    /// Hash of the last saved/opened content — used to detect when edits revert
    /// to the original state so we can clear the modified flag.
    pub saved_content_hash: u64,

    // --- VFS sync state (updated by App from VFS) ---
    pub sync_state: Option<SyncState>,
}

impl WorkspaceTab {
    pub fn new_script(
        id: TabId,
        name: String,
        file_path: Option<String>,
        conn_name: Option<String>,
    ) -> Self {
        Self {
            id,
            kind: TabKind::Script {
                file_path,
                name,
                conn_name,
            },
            active_sub_view: None,
            editor: Some(VimEditor::new_empty(VimModeConfig::default())),
            ..Self::empty(id)
        }
    }

    pub fn new_table(id: TabId, conn_name: String, schema: String, table: String) -> Self {
        Self {
            id,
            kind: TabKind::Table {
                conn_name,
                schema,
                table,
            },
            active_sub_view: Some(SubView::TableData),
            ddl_editor: Some(VimEditor::new_empty(VimModeConfig::read_only())),
            ..Self::empty(id)
        }
    }

    pub fn new_package(id: TabId, conn_name: String, schema: String, name: String) -> Self {
        Self {
            id,
            kind: TabKind::Package {
                conn_name,
                schema,
                name,
            },
            active_sub_view: Some(SubView::PackageDeclaration),
            decl_editor: Some(VimEditor::new_empty(VimModeConfig::default())),
            body_editor: Some(VimEditor::new_empty(VimModeConfig::default())),
            ..Self::empty(id)
        }
    }

    pub fn new_function(id: TabId, conn_name: String, schema: String, name: String) -> Self {
        Self {
            id,
            kind: TabKind::Function {
                conn_name,
                schema,
                name,
            },
            active_sub_view: None,
            editor: Some(VimEditor::new_empty(VimModeConfig::default())),
            ..Self::empty(id)
        }
    }

    pub fn new_procedure(id: TabId, conn_name: String, schema: String, name: String) -> Self {
        Self {
            id,
            kind: TabKind::Procedure {
                conn_name,
                schema,
                name,
            },
            active_sub_view: None,
            editor: Some(VimEditor::new_empty(VimModeConfig::default())),
            ..Self::empty(id)
        }
    }

    pub fn new_db_type(id: TabId, conn_name: String, schema: String, name: String) -> Self {
        Self {
            id,
            kind: TabKind::DbType {
                conn_name,
                schema,
                name,
            },
            active_sub_view: Some(SubView::TypeAttributes),
            decl_editor: Some(VimEditor::new_empty(VimModeConfig::read_only())),
            body_editor: Some(VimEditor::new_empty(VimModeConfig::read_only())),
            ..Self::empty(id)
        }
    }

    pub fn new_trigger(id: TabId, conn_name: String, schema: String, name: String) -> Self {
        Self {
            id,
            kind: TabKind::Trigger {
                conn_name,
                schema,
                name,
            },
            active_sub_view: Some(SubView::TriggerColumns),
            decl_editor: Some(VimEditor::new_empty(VimModeConfig::read_only())),
            ..Self::empty(id)
        }
    }

    /// Create a fresh independent copy of this tab with a new TabId, used by tab group splits.
    /// For scripts, copies the editor content. For other types, creates a blank instance
    /// (will need to reload data from DB on access).
    pub fn clone_for_split(&self, new_id: TabId) -> Self {
        let mut tab = match &self.kind {
            TabKind::Script {
                file_path,
                name,
                conn_name,
            } => Self::new_script(new_id, name.clone(), file_path.clone(), conn_name.clone()),
            TabKind::Table {
                conn_name,
                schema,
                table,
            } => Self::new_table(new_id, conn_name.clone(), schema.clone(), table.clone()),
            TabKind::Package {
                conn_name,
                schema,
                name,
            } => Self::new_package(new_id, conn_name.clone(), schema.clone(), name.clone()),
            TabKind::Function {
                conn_name,
                schema,
                name,
            } => Self::new_function(new_id, conn_name.clone(), schema.clone(), name.clone()),
            TabKind::Procedure {
                conn_name,
                schema,
                name,
            } => Self::new_procedure(new_id, conn_name.clone(), schema.clone(), name.clone()),
            TabKind::DbType {
                conn_name,
                schema,
                name,
            } => Self::new_db_type(new_id, conn_name.clone(), schema.clone(), name.clone()),
            TabKind::Trigger {
                conn_name,
                schema,
                name,
            } => Self::new_trigger(new_id, conn_name.clone(), schema.clone(), name.clone()),
        };

        // For scripts, copy editor content so the user sees the same code in both halves
        if matches!(self.kind, TabKind::Script { .. })
            && let Some(src_editor) = &self.editor
            && let Some(dst_editor) = tab.editor.as_mut()
        {
            dst_editor.set_content(&src_editor.content());
            dst_editor.modified = src_editor.modified;
        }

        tab
    }

    fn empty(id: TabId) -> Self {
        Self {
            id,
            kind: TabKind::Script {
                file_path: None,
                name: String::new(),
                conn_name: None,
            },
            active_sub_view: None,
            query_result: None,
            table_data_result: None,
            columns: Vec::new(),
            result_tabs: Vec::new(),
            active_result_idx: 0,
            grid_scroll_row: 0,
            grid_scroll_col: 0,
            grid_selected_row: 0,
            grid_selected_col: 0,
            grid_visible_height: 20,
            grid_selection_anchor: None,
            grid_visual_mode: false,
            grid_on_header: false,
            grid_focused: false,
            streaming: false,
            streaming_since: None,
            streaming_abort: None,
            sub_focus: SubFocus::Editor,
            ddl_editor: None,
            grid_error_editor: None,
            grid_query_editor: None,
            grid_changes: HashMap::new(),
            grid_editing: None,
            grid_edit_buffer: String::new(),
            grid_edit_cursor: 0,
            package_content: None,
            body_editor: None,
            decl_editor: None,
            package_functions: Vec::new(),
            package_procedures: Vec::new(),
            package_list_cursor: 0,
            type_attributes: None,
            type_methods: None,
            trigger_columns: None,
            editor: None,
            original_decl: None,
            original_body: None,
            original_source: None,
            saved_content_hash: 0,
            sync_state: None,
        }
    }

    /// Get available sub-views for this tab kind
    pub fn available_sub_views(&self) -> Vec<SubView> {
        match &self.kind {
            TabKind::Table { .. } => vec![
                SubView::TableData,
                SubView::TableProperties,
                SubView::TableDDL,
            ],
            TabKind::Package { .. } => vec![
                SubView::PackageDeclaration,
                SubView::PackageBody,
                SubView::PackageFunctions,
                SubView::PackageProcedures,
            ],
            TabKind::DbType { .. } => vec![
                SubView::TypeAttributes,
                SubView::TypeMethods,
                SubView::TypeDeclaration,
                SubView::TypeBody,
            ],
            TabKind::Trigger { .. } => vec![SubView::TriggerColumns, SubView::TriggerDeclaration],
            TabKind::Script { .. } | TabKind::Function { .. } | TabKind::Procedure { .. } => {
                vec![]
            }
        }
    }

    /// Cycle to next sub-view
    pub fn next_sub_view(&mut self) {
        let views = self.available_sub_views();
        if views.len() <= 1 {
            return;
        }
        if let Some(current) = &self.active_sub_view
            && let Some(idx) = views.iter().position(|v| v == current)
        {
            self.active_sub_view = Some(views[(idx + 1) % views.len()].clone());
        }
    }

    /// Cycle to previous sub-view
    pub fn prev_sub_view(&mut self) {
        let views = self.available_sub_views();
        if views.len() <= 1 {
            return;
        }
        if let Some(current) = &self.active_sub_view
            && let Some(idx) = views.iter().position(|v| v == current)
        {
            let prev = if idx == 0 { views.len() - 1 } else { idx - 1 };
            self.active_sub_view = Some(views[prev].clone());
        }
    }

    /// Sync query_result with the correct data source for Type/Trigger sub-views.
    /// This allows the data grid to work with h/j/k/l navigation, visual selection, copy.
    pub fn sync_grid_for_subview(&mut self) {
        // When switching to a non-Data sub-view, preserve the original table data
        if self.table_data_result.is_none()
            && self.query_result.is_some()
            && !matches!(self.active_sub_view, Some(SubView::TableData))
        {
            self.table_data_result = self.query_result.clone();
        }

        let reset_grid = |s: &mut Self| {
            s.grid_selected_row = 0;
            s.grid_selected_col = 0;
            s.grid_scroll_row = 0;
            s.grid_scroll_col = 0;
            s.grid_on_header = true;
            s.grid_visual_mode = false;
            s.grid_selection_anchor = None;
        };

        match &self.active_sub_view {
            Some(SubView::TableData) => {
                // Restore original table data
                if let Some(data) = self.table_data_result.take() {
                    self.query_result = Some(data);
                }
                reset_grid(self);
            }
            Some(SubView::TypeAttributes) => {
                self.query_result = self.type_attributes.clone();
                reset_grid(self);
            }
            Some(SubView::TypeMethods) => {
                self.query_result = self.type_methods.clone();
                reset_grid(self);
            }
            Some(SubView::TriggerColumns) => {
                self.query_result = self.trigger_columns.clone();
                reset_grid(self);
            }
            Some(SubView::TableProperties) => {
                self.query_result = Some(QueryResult {
                    columns: vec![
                        "Column".to_string(),
                        "Type".to_string(),
                        "Nullable".to_string(),
                        "PK".to_string(),
                    ],
                    rows: self
                        .columns
                        .iter()
                        .map(|col| {
                            vec![
                                col.name.clone(),
                                col.data_type.clone(),
                                if col.nullable {
                                    "YES".to_string()
                                } else {
                                    "NO".to_string()
                                },
                                if col.is_primary_key {
                                    "\u{2713}".to_string()
                                } else {
                                    String::new()
                                },
                            ]
                        })
                        .collect(),
                    elapsed: None,
                });
                reset_grid(self);
            }
            _ => {}
        }
    }

    /// Compute a hash of the given content for modified-state tracking.
    pub fn content_hash(content: &str) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.hash(&mut hasher);
        hasher.finish()
    }

    /// Snapshot the current editor content as the "saved" baseline.
    /// Call this after opening or saving a script.
    pub fn mark_saved(&mut self) {
        if let Some(editor) = self.active_editor() {
            self.saved_content_hash = Self::content_hash(&editor.content());
            // Note: we don't clear modified here — the caller should do that
        }
    }

    /// Check if the active editor's content matches the saved baseline.
    /// If so, clear the modified flag.
    pub fn check_modified(&mut self) {
        if let Some(editor) = self.active_editor() {
            let current = Self::content_hash(&editor.content());
            if current == self.saved_content_hash {
                // Content reverted to saved state — clear modified
                if let Some(e) = self.active_editor_mut() {
                    e.modified = false;
                }
            }
        }
    }

    /// Get the active VimEditor for the current sub-view (if any)
    pub fn active_editor(&self) -> Option<&VimEditor> {
        match &self.active_sub_view {
            Some(SubView::TableDDL) => self.ddl_editor.as_ref(),
            Some(SubView::PackageBody) | Some(SubView::TypeBody) => self.body_editor.as_ref(),
            Some(SubView::PackageDeclaration)
            | Some(SubView::TypeDeclaration)
            | Some(SubView::TriggerDeclaration) => self.decl_editor.as_ref(),
            None => self.editor.as_ref(), // Script/Function/Procedure
            _ => None,
        }
    }

    /// Get the active VimEditor mutably
    pub fn active_editor_mut(&mut self) -> Option<&mut VimEditor> {
        match &self.active_sub_view {
            Some(SubView::TableDDL) => self.ddl_editor.as_mut(),
            Some(SubView::PackageBody) | Some(SubView::TypeBody) => self.body_editor.as_mut(),
            Some(SubView::PackageDeclaration)
            | Some(SubView::TypeDeclaration)
            | Some(SubView::TriggerDeclaration) => self.decl_editor.as_mut(),
            None => self.editor.as_mut(), // Script/Function/Procedure
            _ => None,
        }
    }
}
