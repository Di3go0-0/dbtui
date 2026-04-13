use std::sync::Arc;

use crate::core::DatabaseAdapter;
use crate::core::models::*;
use crate::ui::state::{CategoryKind, TreeNode};
use crate::ui::tabs::{TabId, TabKind};

use super::App;
use super::message_helpers::{extract_names, wrap_error_text};

impl App {
    /// Handle the SchemasLoaded message: populate sidebar tree and warm-up metadata.
    pub(super) fn handle_schemas_loaded(&mut self, conn_name: String, schemas: Vec<Schema>) {
        let conn_idx = self
            .state
            .sidebar
            .tree
            .iter()
            .position(|n| matches!(n, TreeNode::Connection { name, .. } if name == &conn_name));
        if let Some(idx) = conn_idx {
            let d = self.state.sidebar.tree[idx].depth();
            let mut end = idx + 1;
            while end < self.state.sidebar.tree.len() && self.state.sidebar.tree[end].depth() > d {
                end += 1;
            }
            self.state.sidebar.tree.drain(idx + 1..end);

            // Build all nodes in a batch (avoids O(n^2) insert shifts)
            let cats_template: Vec<(&str, CategoryKind)> = match self.state.conn.db_type {
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
            let mut batch = Vec::with_capacity(schemas.len() * (cats_template.len() + 1));
            for schema in &schemas {
                batch.push(TreeNode::Schema {
                    name: schema.name.clone(),
                    expanded: false,
                });
                for (label, kind) in &cats_template {
                    batch.push(TreeNode::Category {
                        label: label.to_string(),
                        schema: schema.name.clone(),
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
                for schema in &schemas {
                    idx.add_schema(&schema.name);
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
            let other_schemas: Vec<String> = schemas
                .iter()
                .map(|s| s.name.clone())
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

    /// Handle the QueryBatch message: append rows to a script result tab or table/view tab.
    pub(super) fn handle_query_batch(
        &mut self,
        tab_id: TabId,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        done: bool,
        new_tab: bool,
        elapsed: Option<std::time::Duration>,
    ) {
        let batch_len = rows.len();
        if let Some(tab) = self.state.find_tab_mut(tab_id) {
            let is_script = matches!(tab.kind, TabKind::Script { .. });
            if is_script {
                // Decide whether this batch starts a new result tab or
                // appends to an existing one. `first_batch_pending` is
                // set at Execute dispatch and cleared here — it's the
                // only reliable "this is the first batch of a fresh
                // query" signal now that `tab.streaming` is also
                // set upfront for the loading placeholder.
                let is_first = tab.first_batch_pending;
                tab.first_batch_pending = false;
                // Consume the SQL stashed at dispatch time so the
                // result tab knows how to re-execute itself (for
                // manual refresh / auto-refresh). Only read on the
                // first batch of a fresh query — subsequent batches
                // just append rows.
                let (src_query, src_line) = if is_first {
                    tab.pending_query.take().unwrap_or_default()
                } else {
                    (String::new(), 0)
                };

                let rt_idx = if tab.result_tabs.is_empty() {
                    // No prior results at all -> create the first one.
                    use crate::ui::tabs::ResultTab;
                    let label = format!("Result {}", tab.result_tabs.len() + 1);
                    let rt = ResultTab::new_data(label, columns, rows, src_query, src_line);
                    tab.result_tabs.push(rt);
                    tab.active_result_idx = tab.result_tabs.len() - 1;
                    tab.grid_focused = false;
                    tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
                    tab.result_tabs.len() - 1
                } else if is_first {
                    // First batch of a fresh query. `new_tab` decides
                    // whether to push a brand-new result tab or
                    // replace the active one in-place.
                    use crate::ui::tabs::ResultTab;
                    if new_tab {
                        let label = format!("Result {}", tab.result_tabs.len() + 1);
                        let rt = ResultTab::new_data(label, columns, rows, src_query, src_line);
                        tab.result_tabs.push(rt);
                        tab.active_result_idx = tab.result_tabs.len() - 1;
                        tab.grid_focused = false;
                        tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
                        tab.active_result_idx
                    } else {
                        // Replace the active result tab in place so
                        // <leader>Enter overwrites the previous
                        // result instead of appending rows to it.
                        // Carry the run_count + auto_refresh across
                        // so the user sees the counter climb and
                        // auto-refresh keeps running through the
                        // replacement — but only when the *same*
                        // query is being re-executed. If the user
                        // edited the SQL between runs we treat it
                        // as a brand-new result and reset the
                        // counter to 1.
                        let idx = tab.active_result_idx;
                        let label = format!("Result {}", idx + 1);
                        let mut rt =
                            ResultTab::new_data(label, columns, rows, src_query.clone(), src_line);
                        if idx < tab.result_tabs.len() {
                            let prev = &tab.result_tabs[idx];
                            let same_query = prev.source_query.trim() == src_query.trim();
                            if same_query {
                                rt.run_count = prev.run_count + 1;
                                rt.auto_refresh = prev.auto_refresh.clone();
                            }
                            tab.result_tabs[idx] = rt;
                        } else {
                            tab.result_tabs.push(rt);
                            tab.active_result_idx = tab.result_tabs.len() - 1;
                        }
                        tab.active_result_idx
                    }
                } else {
                    // Continuing the same stream — append rows to
                    // the active result tab.
                    let idx = tab.active_result_idx;
                    if idx < tab.result_tabs.len() {
                        tab.result_tabs[idx].result.rows.extend(rows);
                    }
                    idx
                };
                tab.streaming = !done;
                if done {
                    tab.streaming_abort = None;
                    tab.streaming_since = None;
                    // Clear the auto-refresh in_flight flag so the
                    // next tick can fire — and refresh `next_at`
                    // from "now" so the cadence is measured from
                    // when the previous run *finished*, not when
                    // it started (avoids drift if refreshes are
                    // slower than the interval).
                    let cur_idx = tab.active_result_idx;
                    if let Some(rt) = tab.result_tabs.get_mut(cur_idx)
                        && let Some(ar) = rt.auto_refresh.as_mut()
                    {
                        ar.in_flight = false;
                        ar.next_at = std::time::Instant::now() + ar.interval;
                    }
                }
                // Store elapsed time on the result tab when the stream finishes
                if let Some(dur) = elapsed
                    && let Some(rt) = tab.result_tabs.get_mut(rt_idx)
                {
                    rt.result.elapsed = Some(dur);
                }
            } else {
                // Table/view tab: append rows
                if let Some(ref mut qr) = tab.query_result {
                    qr.rows.extend(rows);
                } else {
                    tab.query_result = Some(QueryResult {
                        columns,
                        rows,
                        elapsed,
                    });
                    tab.grid_selected_row = 0;
                    tab.grid_scroll_row = 0;
                }
                tab.streaming = !done;
                if done {
                    tab.streaming_abort = None;
                    tab.streaming_since = None;
                }
            }
        }

        // Total row count from the tab
        let total_rows = self
            .state
            .find_tab(tab_id)
            .map(|tab| {
                let is_script = matches!(tab.kind, TabKind::Script { .. });
                if is_script {
                    tab.result_tabs
                        .get(tab.active_result_idx)
                        .map(|rt| rt.result.rows.len())
                        .unwrap_or(0)
                } else {
                    tab.query_result
                        .as_ref()
                        .map(|qr| qr.rows.len())
                        .unwrap_or(0)
                }
            })
            .unwrap_or(0);

        if done {
            self.finish_loading();
            self.state.status_message = if let Some(d) = elapsed {
                let ms = d.as_millis();
                if ms < 1000 {
                    format!("{total_rows} rows returned ({ms} ms)")
                } else {
                    format!("{total_rows} rows returned ({:.2} s)", d.as_secs_f64())
                }
            } else {
                format!("{total_rows} rows returned")
            };
        } else {
            self.state.status_message = format!("Loading... {total_rows} rows (+{batch_len})");
        }
    }

    /// Handle the QueryFailed message: show error in result tab.
    pub(super) fn handle_query_failed(
        &mut self,
        tab_id: TabId,
        error: String,
        query: String,
        new_tab: bool,
        start_line: usize,
    ) {
        if let Some(tab) = self.state.find_tab_mut(tab_id) {
            let is_script = matches!(tab.kind, TabKind::Script { .. });
            if is_script {
                use crate::ui::tabs::ResultTab;
                use vimltui::VimEditor;

                // Error editor (left pane) — show real line number
                let header = format!("-- Query Error (line {}) --\n\n", start_line + 1);
                let wrap_width = 40;
                let formatted = format!("{header}{}", wrap_error_text(&error, wrap_width));
                let mut err_editor =
                    VimEditor::new(&formatted, vimltui::VimModeConfig::read_only());
                err_editor.mode = vimltui::VimMode::Normal;

                // Query editor (right pane) — the SQL that failed
                let mut q_editor = VimEditor::new(&query, vimltui::VimModeConfig::read_only());
                q_editor.mode = vimltui::VimMode::Normal;

                let label = format!("Error {}", tab.result_tabs.len() + 1);
                let rt = ResultTab {
                    label,
                    result: QueryResult {
                        columns: vec![],
                        rows: vec![],
                        elapsed: None,
                    },
                    error_editor: Some(err_editor),
                    query_editor: Some(q_editor),
                    scroll_row: 0,
                    selected_row: 0,
                    selected_col: 0,
                    visible_height: 20,
                    selection_anchor: None,
                    on_header: false,
                    anchor_on_header: false,
                    run_count: 1,
                    last_run_at: Some(std::time::SystemTime::now()),
                    flashed_at: Some(std::time::Instant::now()),
                    source_query: query.clone(),
                    source_start_line: start_line,
                    auto_refresh: None,
                };
                if new_tab || tab.result_tabs.is_empty() {
                    tab.result_tabs.push(rt);
                    tab.active_result_idx = tab.result_tabs.len() - 1;
                } else {
                    let idx = tab.active_result_idx;
                    if idx < tab.result_tabs.len() {
                        tab.result_tabs[idx] = rt;
                    } else {
                        tab.result_tabs.push(rt);
                        tab.active_result_idx = tab.result_tabs.len() - 1;
                    }
                }
                // Stay in editor — user navigates to results manually
                tab.grid_focused = false;
                tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
                // The query is done (it failed) — clear every
                // streaming marker so subsequent logic (placeholder
                // render, close-cancels-query, etc.) doesn't think a
                // query is still in flight.
                tab.streaming = false;
                tab.streaming_since = None;
                tab.streaming_abort = None;
                tab.first_batch_pending = false;
                tab.pending_query = None;
                // A failed auto-refresh should still release the
                // in_flight slot so the user can fix the query
                // and let the next tick try again.
                let cur_idx = tab.active_result_idx;
                if let Some(rt) = tab.result_tabs.get_mut(cur_idx)
                    && let Some(ar) = rt.auto_refresh.as_mut()
                {
                    ar.in_flight = false;
                    ar.next_at = std::time::Instant::now() + ar.interval;
                }
            }
        }
        self.finish_loading();
    }

    /// Handle the CompileResult message: update editors and show success/error.
    pub(super) fn handle_compile_result(
        &mut self,
        tab_id: TabId,
        success: bool,
        message: String,
        failed_sql: String,
        failed_part: String,
    ) {
        if success {
            self.sync_tab_to_vfs_compiled(tab_id);
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
                match failed_part.as_str() {
                    "DECLARATION" => {
                        if tab.active_sub_view != Some(SubView::PackageDeclaration)
                            && tab.active_sub_view != Some(SubView::TypeDeclaration)
                        {
                            tab.active_sub_view = Some(SubView::PackageDeclaration);
                        }
                    }
                    "BODY" => {
                        if tab.active_sub_view != Some(SubView::PackageBody)
                            && tab.active_sub_view != Some(SubView::TypeBody)
                        {
                            tab.active_sub_view = Some(SubView::PackageBody);
                        }
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
            }

            self.state.status_message = format!("Compilation failed: {message}");
        }
        self.finish_loading();
    }

    /// Handle the Connected message: register adapter and trigger schema loading.
    pub(super) fn handle_connected(&mut self, adapter: Arc<dyn DatabaseAdapter>, name: String) {
        if self.state.overlay.is_some() {
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
            tokio::spawn(async move {
                match adapter.get_schemas().await {
                    Ok(schemas) => {
                        let _ = tx
                            .send(super::AppMessage::SchemasLoaded { conn_name, schemas })
                            .await;
                    }
                    Err(e) => {
                        let _ = tx.send(super::AppMessage::Error(e.to_string())).await;
                    }
                }
            });
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
