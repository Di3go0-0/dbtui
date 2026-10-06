use std::sync::Arc;

use crate::core::DatabaseAdapter;
use crate::core::models::*;
use crate::ui::state::{CategoryKind, TreeNode};
use crate::ui::tabs::{TabId, TabKind};

use super::App;
use super::message_helpers::{extract_names, wrap_error_text};

impl App {
    /// Handle the CatalogsLoaded message: hang one node per database off the
    /// connection. Their schemas load lazily, when a database is expanded.
    pub(super) fn handle_catalogs_loaded(
        &mut self,
        conn_name: String,
        catalogs: Vec<crate::core::models::Catalog>,
    ) {
        let Some(idx) = self.connection_tree_idx(&conn_name) else {
            return;
        };
        self.replace_subtree_of(idx);

        if catalogs.is_empty() {
            let depth = self.state.sidebar.tree[idx].depth() + 1;
            self.state
                .sidebar
                .tree
                .insert(idx + 1, TreeNode::Empty { depth });
            return;
        }

        let batch: Vec<TreeNode> = catalogs
            .into_iter()
            .map(|c| TreeNode::Catalog {
                name: c.name,
                expanded: false,
            })
            .collect();
        let insert_pos = idx + 1;
        self.state
            .sidebar
            .tree
            .splice(insert_pos..insert_pos, batch);
    }

    fn connection_tree_idx(&self, conn_name: &str) -> Option<usize> {
        self.state
            .sidebar
            .tree
            .iter()
            .position(|n| matches!(n, TreeNode::Connection { name, .. } if name == conn_name))
    }

    /// Index of one database node under a given connection.
    ///
    /// The search starts at the connection and stops at the next connection,
    /// so two connections exposing databases of the same name stay distinct.
    fn catalog_tree_idx(&self, conn_name: &str, catalog: &str) -> Option<usize> {
        let start = self.connection_tree_idx(conn_name)?;
        self.state.sidebar.tree[start + 1..]
            .iter()
            .position(|n| match n {
                TreeNode::Catalog { name, .. } => name == catalog,
                _ => false,
            })
            .map(|offset| start + 1 + offset)
            .filter(|&idx| {
                !self.state.sidebar.tree[start + 1..idx]
                    .iter()
                    .any(|n| matches!(n, TreeNode::Connection { .. }))
            })
    }

    /// Drop every node nested under `idx`, leaving the node itself in place.
    fn replace_subtree_of(&mut self, idx: usize) {
        let d = self.state.sidebar.tree[idx].depth();
        let mut end = idx + 1;
        while end < self.state.sidebar.tree.len() && self.state.sidebar.tree[end].depth() > d {
            end += 1;
        }
        self.state.sidebar.tree.drain(idx + 1..end);
    }

    /// Handle the SchemasLoaded message: populate sidebar tree and warm-up metadata.
    pub(super) fn handle_schemas_loaded(
        &mut self,
        conn_name: String,
        catalog: Option<String>,
        schemas: Vec<Schema>,
    ) {
        // With a catalog level the schemas belong under their database node,
        // which is the one that was just expanded; otherwise under the
        // connection itself.
        let conn_idx = match &catalog {
            Some(cat) => self.catalog_tree_idx(&conn_name, cat),
            None => self.connection_tree_idx(&conn_name),
        };
        if let Some(idx) = conn_idx {
            self.replace_subtree_of(idx);

            // Build all nodes in a batch (avoids O(n^2) insert shifts)
            let db_type = self.db_type_of(&conn_name).or(self.state.conn.db_type);
            let cats_template: Vec<(&str, CategoryKind)> = match db_type {
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
                Some(DatabaseType::SqlServer) => vec![
                    ("Tables", CategoryKind::Tables),
                    ("Views", CategoryKind::Views),
                    ("Indexes", CategoryKind::Indexes),
                    ("Triggers", CategoryKind::Triggers),
                    ("Procedures", CategoryKind::Procedures),
                    ("Functions", CategoryKind::Functions),
                ],
            };
            // Schemas expanded under a Catalog node belong to that database;
            // under a Connection node there is no catalog level.
            let catalog = self.state.sidebar.tree[idx]
                .catalog()
                .map(|c| c.to_string());
            let mut batch = Vec::with_capacity(schemas.len() * (cats_template.len() + 1));
            for schema in &schemas {
                batch.push(TreeNode::Schema {
                    name: schema.name.clone(),
                    catalog: catalog.clone(),
                    expanded: false,
                });
                let qualified = match &catalog {
                    Some(db) => format!("{db}.{}", schema.name),
                    None => schema.name.clone(),
                };
                for (label, kind) in &cats_template {
                    batch.push(TreeNode::Category {
                        label: label.to_string(),
                        schema: qualified.clone(),
                        catalog: catalog.clone(),
                        kind: kind.clone(),
                        expanded: false,
                    });
                }
            }
            // Single splice instead of hundreds of inserts
            let insert_pos = idx + 1;
            self.state
                .sidebar
                .tree
                .splice(insert_pos..insert_pos, batch);

            // Populate MetadataIndex with schema names
            {
                let idx = self
                    .state
                    .engine
                    .metadata_indexes
                    .entry(conn_name.clone())
                    .or_default();
                // Register the same qualified form the tree hands to the
                // driver, so objects and schemas key alike and completion
                // resolves them.
                for schema in &schemas {
                    match &catalog {
                        Some(db) => idx.add_schema(&format!("{db}.{}", schema.name)),
                        None => idx.add_schema(&schema.name),
                    }
                }
            }

            // Determine the user's own schema for priority loading
            let user_schema = self
                .state
                .dialogs
                .saved_connections
                .iter()
                .find(|c| c.name == conn_name)
                .map(|c| match c.db_type {
                    DatabaseType::Oracle => c.username.to_uppercase(),
                    DatabaseType::MySQL => c.database.clone().unwrap_or_default(),
                    DatabaseType::PostgreSQL => "public".to_string(),
                    // SQL Server maps every login to `dbo` unless the DBA
                    // overrides it; with a catalog level it is qualified by the
                    // database whose schemas just arrived.
                    DatabaseType::SqlServer => match &catalog {
                        Some(db) => format!("{db}.dbo"),
                        None => "dbo".to_string(),
                    },
                });

            // Set per-connection current_schema in metadata index
            if let Some(ref us) = user_schema {
                if let Some(idx) = self.state.engine.metadata_indexes.get_mut(&conn_name) {
                    idx.set_current_schema(us);
                }
                // Only update global conn state if this is the active connection
                if self
                    .state
                    .conn
                    .name
                    .as_ref()
                    .is_some_and(|n| n == &conn_name)
                    || self.state.conn.current_schema.is_none()
                {
                    self.state.conn.current_schema = Some(us.clone());
                }
            }

            // Warm-up: core categories for user's schema; new metadata categories stay lazy
            if let Some(ref us) = user_schema
                && let Some(adapter) = self.adapter_for(&conn_name)
            {
                self.spawn_load_children_for(&conn_name, us, "Tables", &adapter);
                self.spawn_load_children_for(&conn_name, us, "Views", &adapter);
                self.spawn_load_children_for(&conn_name, us, "Procedures", &adapter);
                self.spawn_load_children_for(&conn_name, us, "Functions", &adapter);
                let db_type = self
                    .state
                    .dialogs
                    .saved_connections
                    .iter()
                    .find(|c| c.name == conn_name)
                    .map(|c| c.db_type);
                if matches!(db_type, Some(DatabaseType::Oracle)) {
                    self.spawn_load_children_for(&conn_name, us, "Packages", &adapter);
                }
            }

            // Load remaining schemas sequentially in background
            // Qualified the same way as the tree's categories and the user
            // schema above (`database.schema` under a catalog). A bare name
            // would be read by the driver as a schema of the connection's own
            // database, and would never match the user schema to skip it.
            let other_schemas: Vec<String> = schemas
                .iter()
                .map(|s| match &catalog {
                    Some(db) => format!("{db}.{}", s.name),
                    None => s.name.clone(),
                })
                .filter(|s| {
                    !user_schema
                        .as_ref()
                        .is_some_and(|us| s.eq_ignore_ascii_case(us))
                })
                .collect();

            if !other_schemas.is_empty() {
                let db_type = self
                    .state
                    .dialogs
                    .saved_connections
                    .iter()
                    .find(|c| c.name == conn_name)
                    .map(|c| c.db_type);
                let mut labels = vec![
                    "Tables".to_string(),
                    "Views".to_string(),
                    "Procedures".to_string(),
                    "Functions".to_string(),
                ];
                if matches!(db_type, Some(DatabaseType::Oracle)) {
                    labels.push("Packages".to_string());
                }
                self.spawn_load_remaining_schemas(&conn_name, other_schemas, labels);
            }
        }
        self.state.status_message = format!("Schemas loaded for {conn_name} - F to filter");
        self.finish_loading();
    }

    /// Handle the CompileResult message: update editors and show success/error.
    pub(super) fn handle_compile_result(
        &mut self,
        tab_id: TabId,
        success: bool,
        message: String,
        position: Option<crate::core::error::ErrorPosition>,
        failed_sql: String,
        failed_part: String,
    ) {
        if success {
            self.sync_tab_to_vfs_compiled(tab_id);
            self.clear_server_diagnostics(tab_id);
            if let Some(tab) = self.state.find_tab_mut(tab_id) {
                // Update originals to current content and clear signs
                if let Some(editor) = tab.decl_editor.as_ref() {
                    tab.original_decl = Some(editor.content());
                }
                if let Some(editor) = tab.body_editor.as_ref() {
                    tab.original_body = Some(editor.content());
                }
                if let Some(editor) = tab.editor.as_ref() {
                    tab.original_source = Some(editor.content());
                }
                // Clear signs on all editors
                if let Some(editor) = tab.decl_editor.as_mut() {
                    editor.modified = false;
                    editor.gutter = None;
                }
                if let Some(editor) = tab.body_editor.as_mut() {
                    editor.modified = false;
                    editor.gutter = None;
                }
                if let Some(editor) = tab.editor.as_mut() {
                    editor.modified = false;
                    editor.gutter = None;
                }
            }
            // Show success with object name
            let obj_label = self
                .state
                .find_tab(tab_id)
                .map(|t| match &t.kind {
                    TabKind::Package { schema, name, .. } => {
                        format!("PACKAGE {schema}.{name}")
                    }
                    TabKind::Function { schema, name, .. } => {
                        format!("FUNCTION {schema}.{name}")
                    }
                    TabKind::Procedure { schema, name, .. } => {
                        format!("PROCEDURE {schema}.{name}")
                    }
                    _ => "object".to_string(),
                })
                .unwrap_or_else(|| "object".to_string());
            // Also clear error panels if present
            if let Some(tab) = self.state.find_tab_mut(tab_id) {
                tab.grid_error_editor = None;
                tab.grid_query_editor = None;
            }
            self.state.status_message = format!("\u{2713} {obj_label} compiled successfully");
        } else {
            self.sync_tab_to_vfs_error(tab_id, message.clone());

            if let Some(tab) = self.state.find_tab_mut(tab_id) {
                use crate::ui::tabs::SubView;
                use vimltui::VimEditor;

                // Switch to the sub-view where the error occurred
                let showing = |views: [SubView; 2]| {
                    tab.active_sub_view
                        .as_ref()
                        .is_some_and(|view| views.contains(view))
                };
                match failed_part.as_str() {
                    "DECLARATION"
                        if !showing([SubView::PackageDeclaration, SubView::TypeDeclaration]) =>
                    {
                        tab.active_sub_view = Some(SubView::PackageDeclaration);
                    }
                    "BODY" if !showing([SubView::PackageBody, SubView::TypeBody]) => {
                        tab.active_sub_view = Some(SubView::PackageBody);
                    }
                    _ => {}
                }

                // Create error + SQL panels (same pattern as script query errors)
                let err_header = format!(
                    "-- Compile Error ({}) --\n\n{}",
                    failed_part,
                    wrap_error_text(&message, 40)
                );
                let mut err_editor =
                    VimEditor::new(&err_header, vimltui::VimModeConfig::read_only());
                err_editor.mode = vimltui::VimMode::Normal;

                let mut q_editor = VimEditor::new(&failed_sql, vimltui::VimModeConfig::read_only());
                q_editor.mode = vimltui::VimMode::Normal;

                tab.grid_error_editor = Some(err_editor);
                tab.grid_query_editor = Some(q_editor);
                tab.sub_focus = crate::ui::tabs::SubFocus::Editor;

                mark_compile_errors(tab, &message, position);
            }

            let headline = message.lines().next().unwrap_or_default();
            self.state.status_message = format!("Compilation failed: {headline}");
        }
        self.refresh_diagnostics_if_active(tab_id);
        self.finish_loading();
    }

    /// Handle the Connected message: register adapter and trigger schema loading.
    pub(super) fn handle_connected(&mut self, adapter: Arc<dyn DatabaseAdapter>, name: String) {
        // Only the dialog that started this attempt is closed and saved. A
        // connect started elsewhere (sidebar, opening a script) must not
        // close whatever overlay is open, let alone persist a half-typed form.
        if self.dialog_is_connecting(&name) {
            let config = self.state.dialogs.connection_form.to_connection_config();
            self.save_connection_config(&config);
            self.state.overlay = None;
            self.state.dialogs.connection_form.connecting = false;
            self.state.dialogs.connection_form.connecting_since = None;
        }

        self.set_conn_status(&name, crate::ui::state::ConnStatus::Connected);

        let already_in_tree = self
            .state
            .sidebar
            .tree
            .iter()
            .any(|n| matches!(n, TreeNode::Connection { name: n, .. } if n == &name));

        if already_in_tree {
            self.adapters.insert(name.clone(), Arc::clone(&adapter));
            self.state.conn.connected = true;
            self.state.conn.name = Some(name.clone());
            self.state.conn.db_type = Some(adapter.db_type());

            let tx = self.msg_tx.clone();
            let conn_name = name.clone();
            tokio::spawn(super::spawns::load_tree_root(adapter, conn_name, tx));
        } else {
            self.add_connection(adapter, &name);
        }

        self.state.status_message = format!("Connected to {name}");
        self.finish_loading();
    }

    /// Handle the PackageContentLoaded message: populate tab editors and cache members.
    pub(super) fn handle_package_content_loaded(&mut self, tab_id: TabId, content: PackageContent) {
        // Get connection name + schema + package name before mutating state
        let pkg_info = self.state.find_tab(tab_id).and_then(|t| match &t.kind {
            TabKind::Package {
                conn_name,
                schema,
                name,
            } => Some((conn_name.clone(), schema.clone(), name.clone())),
            _ => None,
        });
        let conn_name = pkg_info.as_ref().map(|p| p.0.clone());

        if let Some(tab) = self.state.find_tab_mut(tab_id) {
            tab.streaming_since = None;
            tab.package_functions = extract_names(&content.declaration, "FUNCTION");
            tab.package_procedures = extract_names(&content.declaration, "PROCEDURE");
            tab.package_list_cursor = 0;

            tab.original_decl = Some(content.declaration.clone());
            tab.original_body = content.body.clone();

            if let Some(editor) = tab.decl_editor.as_mut() {
                editor.set_content(&content.declaration);
            }
            if let Some(editor) = tab.body_editor.as_mut() {
                editor.set_content(content.body.as_deref().unwrap_or(""));
            }
            tab.package_content = Some(content);
        }

        // Cache the package members in the per-connection MetadataIndex
        // so the SQL completion engine can suggest pkg.foo() from any
        // editor — not only from inside this package's tab.
        if let Some((cn, schema, pkg_name)) = pkg_info {
            let funcs = self
                .state
                .find_tab(tab_id)
                .map(|t| t.package_functions.clone())
                .unwrap_or_default();
            let procs = self
                .state
                .find_tab(tab_id)
                .map(|t| t.package_procedures.clone())
                .unwrap_or_default();
            use crate::sql_engine::metadata::{PackageMember, PackageMemberKind};
            let mut members: Vec<PackageMember> = funcs
                .into_iter()
                .map(|name| PackageMember {
                    name,
                    kind: PackageMemberKind::Function,
                })
                .collect();
            members.extend(procs.into_iter().map(|name| PackageMember {
                name,
                kind: PackageMemberKind::Procedure,
            }));
            if let Some(idx) = self.state.engine.metadata_indexes.get_mut(&cn) {
                idx.set_package_members(&schema, &pkg_name, members);
            }
        }

        // Register in VFS
        if let Some(cn) = conn_name {
            self.register_in_vfs(tab_id, &cn);
        }
        self.finish_loading();
    }
}

/// Turn a failed compile into diagnostics on the editor that was compiled:
/// one per line the server listed, or a single one at the position it
/// rejected the statement.
fn mark_compile_errors(
    tab: &mut crate::ui::tabs::WorkspaceTab,
    message: &str,
    position: Option<crate::core::error::ErrorPosition>,
) {
    use super::message_helpers::{CompileMark, parse_compile_marks};
    use crate::ui::diagnostics::{Diagnostic, error_span};

    let mut marks = parse_compile_marks(message);
    if marks.is_empty()
        && let Some(position) = position
    {
        marks.push(CompileMark {
            line: position.line,
            col: position.col.unwrap_or(1),
            text: message.lines().next().unwrap_or_default().to_string(),
        });
    }

    let Some(editor) = tab.active_editor() else {
        return;
    };
    // The source is compiled trimmed, so the server's line 1 is the first
    // non-blank line of the buffer.
    let first_line = editor
        .lines
        .iter()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(0);
    let last_row = editor.lines.len().saturating_sub(1);

    let diagnostics = marks
        .into_iter()
        .filter_map(|mark| {
            let row = (first_line + mark.line.saturating_sub(1)).min(last_row);
            let line = editor.lines.get(row)?;
            Some(Diagnostic::server_error(
                row,
                error_span(line, Some(mark.col)),
                mark.text,
            ))
        })
        .collect();

    tab.server_diagnostics = diagnostics;
    tab.server_diagnostics_view = tab.active_sub_view.clone();
}
