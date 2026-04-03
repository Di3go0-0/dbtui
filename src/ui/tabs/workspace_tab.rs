use crate::core::models::{Column, PackageContent, QueryResult};
use crate::core::virtual_fs::SyncState;
use crate::ui::vim::buffer::VimEditor;
use crate::ui::vim::VimModeConfig;

/// Unique identifier for each open tab
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

/// What kind of item a tab represents
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabKind {
    Script {
        file_path: Option<String>,
        name: String,
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
}

impl TabKind {
    pub fn display_name(&self) -> &str {
        match self {
            TabKind::Script { name, .. } => name,
            TabKind::Table { table, .. } => table,
            TabKind::Package { name, .. } => name,
            TabKind::Function { name, .. } => name,
            TabKind::Procedure { name, .. } => name,
        }
    }

    pub fn icon(&self) -> &str {
        match self {
            TabKind::Script { .. } => "S",
            TabKind::Table { .. } => "T",
            TabKind::Package { .. } => "P",
            TabKind::Function { .. } => "\u{03bb}",  // λ
            TabKind::Procedure { .. } => "\u{0192}", // ƒ
        }
    }

    /// Check if two TabKinds refer to the same object (for deduplication)
    pub fn same_object(&self, other: &TabKind) -> bool {
        match (self, other) {
            (
                TabKind::Table { conn_name: c1, schema: s1, table: t1 },
                TabKind::Table { conn_name: c2, schema: s2, table: t2 },
            ) => c1 == c2 && s1 == s2 && t1 == t2,
            (
                TabKind::Package { conn_name: c1, schema: s1, name: n1 },
                TabKind::Package { conn_name: c2, schema: s2, name: n2 },
            ) => c1 == c2 && s1 == s2 && n1 == n2,
            (
                TabKind::Function { conn_name: c1, schema: s1, name: n1 },
                TabKind::Function { conn_name: c2, schema: s2, name: n2 },
            ) => c1 == c2 && s1 == s2 && n1 == n2,
            (
                TabKind::Procedure { conn_name: c1, schema: s1, name: n1 },
                TabKind::Procedure { conn_name: c2, schema: s2, name: n2 },
            ) => c1 == c2 && s1 == s2 && n1 == n2,
            // Scripts are never deduplicated (each is unique)
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
        }
    }
}

/// A workspace tab with all its state
pub struct WorkspaceTab {
    pub id: TabId,
    pub kind: TabKind,
    pub active_sub_view: Option<SubView>,

    // --- Table state ---
    pub query_result: Option<QueryResult>,
    pub columns: Vec<Column>,
    pub grid_scroll_row: usize,
    pub grid_selected_row: usize,
    pub grid_visible_height: usize,
    pub ddl_editor: Option<VimEditor>,

    // --- Package state ---
    pub package_content: Option<PackageContent>,
    pub body_editor: Option<VimEditor>,
    pub decl_editor: Option<VimEditor>,
    pub package_functions: Vec<String>,
    pub package_procedures: Vec<String>,
    pub package_list_cursor: usize,

    // --- Script / Function / Procedure state ---
    pub editor: Option<VimEditor>,

    // --- VFS sync state (updated by App from VFS) ---
    pub sync_state: Option<SyncState>,
}

impl WorkspaceTab {
    pub fn new_script(id: TabId, name: String, file_path: Option<String>) -> Self {
        Self {
            id,
            kind: TabKind::Script { file_path, name },
            active_sub_view: None,
            editor: Some(VimEditor::new_empty(VimModeConfig::default())),
            ..Self::empty(id)
        }
    }

    pub fn new_table(id: TabId, conn_name: String, schema: String, table: String) -> Self {
        Self {
            id,
            kind: TabKind::Table { conn_name, schema, table },
            active_sub_view: Some(SubView::TableData),
            ddl_editor: Some(VimEditor::new_empty(VimModeConfig::read_only())),
            ..Self::empty(id)
        }
    }

    pub fn new_package(id: TabId, conn_name: String, schema: String, name: String) -> Self {
        Self {
            id,
            kind: TabKind::Package { conn_name, schema, name },
            active_sub_view: Some(SubView::PackageDeclaration),
            decl_editor: Some(VimEditor::new_empty(VimModeConfig::default())),
            body_editor: Some(VimEditor::new_empty(VimModeConfig::default())),
            ..Self::empty(id)
        }
    }

    pub fn new_function(id: TabId, conn_name: String, schema: String, name: String) -> Self {
        Self {
            id,
            kind: TabKind::Function { conn_name, schema, name },
            active_sub_view: None,
            editor: Some(VimEditor::new_empty(VimModeConfig::default())),
            ..Self::empty(id)
        }
    }

    pub fn new_procedure(id: TabId, conn_name: String, schema: String, name: String) -> Self {
        Self {
            id,
            kind: TabKind::Procedure { conn_name, schema, name },
            active_sub_view: None,
            editor: Some(VimEditor::new_empty(VimModeConfig::default())),
            ..Self::empty(id)
        }
    }

    fn empty(id: TabId) -> Self {
        Self {
            id,
            kind: TabKind::Script {
                file_path: None,
                name: String::new(),
            },
            active_sub_view: None,
            query_result: None,
            columns: Vec::new(),
            grid_scroll_row: 0,
            grid_selected_row: 0,
            grid_visible_height: 20,
            ddl_editor: None,
            package_content: None,
            body_editor: None,
            decl_editor: None,
            package_functions: Vec::new(),
            package_procedures: Vec::new(),
            package_list_cursor: 0,
            editor: None,
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
        if let Some(current) = &self.active_sub_view {
            if let Some(idx) = views.iter().position(|v| v == current) {
                self.active_sub_view = Some(views[(idx + 1) % views.len()].clone());
            }
        }
    }

    /// Cycle to previous sub-view
    pub fn prev_sub_view(&mut self) {
        let views = self.available_sub_views();
        if views.len() <= 1 {
            return;
        }
        if let Some(current) = &self.active_sub_view {
            if let Some(idx) = views.iter().position(|v| v == current) {
                let prev = if idx == 0 { views.len() - 1 } else { idx - 1 };
                self.active_sub_view = Some(views[prev].clone());
            }
        }
    }

    /// Get the active VimEditor for the current sub-view (if any)
    pub fn active_editor(&self) -> Option<&VimEditor> {
        match &self.active_sub_view {
            Some(SubView::TableDDL) => self.ddl_editor.as_ref(),
            Some(SubView::PackageBody) => self.body_editor.as_ref(),
            Some(SubView::PackageDeclaration) => self.decl_editor.as_ref(),
            None => self.editor.as_ref(), // Script/Function/Procedure
            _ => None,
        }
    }

    /// Get the active VimEditor mutably
    pub fn active_editor_mut(&mut self) -> Option<&mut VimEditor> {
        match &self.active_sub_view {
            Some(SubView::TableDDL) => self.ddl_editor.as_mut(),
            Some(SubView::PackageBody) => self.body_editor.as_mut(),
            Some(SubView::PackageDeclaration) => self.decl_editor.as_mut(),
            None => self.editor.as_mut(), // Script/Function/Procedure
            _ => None,
        }
    }
}
