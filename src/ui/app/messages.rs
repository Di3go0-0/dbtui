use super::*;

use crate::sql_engine::metadata::ObjectKind as ObjKind;

impl App {
    /// Generic handler for metadata loaded messages (Tables, Views, Procedures, etc.)
    fn handle_objects_loaded<T: HasName>(
        &mut self,
        conn_name: &str,
        schema: &str,
        items: Vec<T>,
        obj_kind: ObjKind,
        cat_kind: CategoryKind,
        leaf_kind: LeafKind,
    ) {
        let idx = self
            .state
            .engine
            .metadata_indexes
            .entry(conn_name.to_string())
            .or_default();
        for item in &items {
            idx.add_object(schema, &item.get_name(), obj_kind);
        }
        self.insert_leaves(conn_name, schema, cat_kind, items, leaf_kind);
        self.finish_loading();
    }

    /// Reset loading state after an async operation completes.
    fn finish_loading(&mut self) {
        self.state.loading = false;
        self.state.loading_since = None;
    }

    pub(super) fn handle_paste(&mut self, text: &str) {
        use crate::ui::state::Focus;
        use vimltui::VimMode;

        // Paste into export/import dialog path fields
        if matches!(self.state.overlay, Some(Overlay::ExportDialog)) {
            if let Some(ref mut d) = self.state.dialogs.export_dialog
                && d.focused == crate::ui::state::ExportField::Path
            {
                let clean: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                d.path.push_str(&clean);
                d.reset_completions();
            }
            return;
        }
        if matches!(self.state.overlay, Some(Overlay::ImportDialog)) {
            if let Some(ref mut d) = self.state.dialogs.import_dialog {
                match d.focused {
                    crate::ui::state::ImportField::Path => {
                        let clean: String =
                            text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                        d.path.push_str(&clean);
                        d.reset_completions();
                    }
                    crate::ui::state::ImportField::Password => {
                        let clean: String =
                            text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                        d.password.push_str(&clean);
                    }
                    crate::ui::state::ImportField::ShowPassword => {}
                }
            }
            return;
        }

        // Paste into connection dialog fields
        if matches!(self.state.overlay, Some(Overlay::ConnectionDialog)) {
            if !self.state.dialogs.connection_form.read_only
                && self.state.dialogs.connection_form.selected_field != 1
                && self.state.dialogs.connection_form.selected_field != 7
            {
                let clean: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                self.state
                    .dialogs
                    .connection_form
                    .active_field_mut()
                    .push_str(&clean);
                self.state.dialogs.connection_form.error_message.clear();
            }
            return;
        }

        if self.state.focus != Focus::TabContent {
            return;
        }
        if let Some(tab) = self.state.active_tab_mut()
            && let Some(editor) = tab.active_editor_mut()
        {
            // In search/command mode: feed chars as key events
            if editor.search.active || editor.command_active {
                use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
                for ch in text.chars() {
                    if ch != '\n' && ch != '\r' {
                        editor.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
                    }
                }
                return;
            }

            if !matches!(editor.mode, VimMode::Insert | VimMode::Replace) {
                return;
            }
            editor.save_undo();
            for ch in text.chars() {
                if ch == '\n' || ch == '\r' {
                    editor.insert_newline();
                } else {
                    editor.insert_char(ch);
                }
            }
        }
    }

    pub(super) fn handle_message(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::SchemasLoaded { conn_name, schemas } => {
                let conn_idx = self.state.sidebar.tree.iter().position(
                    |n| matches!(n, TreeNode::Connection { name, .. } if name == &conn_name),
                );
                if let Some(idx) = conn_idx {
                    let d = self.state.sidebar.tree[idx].depth();
                    let mut end = idx + 1;
                    while end < self.state.sidebar.tree.len()
                        && self.state.sidebar.tree[end].depth() > d
                    {
                        end += 1;
                    }
                    self.state.sidebar.tree.drain(idx + 1..end);

                    // Build all nodes in a batch (avoids O(n²) insert shifts)
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
            AppMessage::TablesLoaded {
                conn_name,
                schema,
                items,
            } => {
                self.handle_objects_loaded(
                    &conn_name,
                    &schema,
                    items,
                    ObjKind::Table,
                    CategoryKind::Tables,
                    LeafKind::Table,
                );
                // Mark metadata ready once the primary schema's tables are loaded
                let is_primary_schema = self
                    .state
                    .engine
                    .metadata_indexes
                    .get(&conn_name)
                    .and_then(|idx| idx.current_schema())
                    .is_some_and(|cs| cs.eq_ignore_ascii_case(&schema));
                if !self.state.metadata_ready && is_primary_schema {
                    self.state.metadata_ready = true;
                    self.state.status_message = "Context ready".to_string();
                    self.refresh_active_diagnostics();
                }
            }
            AppMessage::ViewsLoaded {
                conn_name,
                schema,
                items,
            } => {
                self.handle_objects_loaded(
                    &conn_name,
                    &schema,
                    items,
                    ObjKind::View,
                    CategoryKind::Views,
                    LeafKind::View,
                );
            }
            AppMessage::PackagesLoaded {
                conn_name,
                schema,
                items,
            } => {
                {
                    let idx = self
                        .state
                        .engine
                        .metadata_indexes
                        .entry(conn_name.clone())
                        .or_default();
                    for item in &items {
                        idx.add_object(&schema, &item.name, ObjKind::Package);
                    }
                }
                self.insert_package_leaves(&conn_name, &schema, items);
                self.finish_loading();
            }
            AppMessage::ProceduresLoaded {
                conn_name,
                schema,
                items,
            } => {
                self.handle_objects_loaded(
                    &conn_name,
                    &schema,
                    items,
                    ObjKind::Procedure,
                    CategoryKind::Procedures,
                    LeafKind::Procedure,
                );
            }
            AppMessage::FunctionsLoaded {
                conn_name,
                schema,
                items,
            } => {
                self.handle_objects_loaded(
                    &conn_name,
                    &schema,
                    items,
                    ObjKind::Function,
                    CategoryKind::Functions,
                    LeafKind::Function,
                );
            }
            AppMessage::MaterializedViewsLoaded {
                conn_name,
                schema,
                items,
            } => {
                self.handle_objects_loaded(
                    &conn_name,
                    &schema,
                    items,
                    ObjKind::MaterializedView,
                    CategoryKind::MaterializedViews,
                    LeafKind::MaterializedView,
                );
            }
            AppMessage::IndexesLoaded {
                conn_name,
                schema,
                items,
            } => {
                self.handle_objects_loaded(
                    &conn_name,
                    &schema,
                    items,
                    ObjKind::Index,
                    CategoryKind::Indexes,
                    LeafKind::Index,
                );
            }
            AppMessage::SequencesLoaded {
                conn_name,
                schema,
                items,
            } => {
                self.handle_objects_loaded(
                    &conn_name,
                    &schema,
                    items,
                    ObjKind::Sequence,
                    CategoryKind::Sequences,
                    LeafKind::Sequence,
                );
            }
            AppMessage::TypesLoaded {
                conn_name,
                schema,
                items,
            } => {
                self.handle_objects_loaded(
                    &conn_name,
                    &schema,
                    items,
                    ObjKind::Type,
                    CategoryKind::Types,
                    LeafKind::Type,
                );
            }
            AppMessage::TriggersLoaded {
                conn_name,
                schema,
                items,
            } => {
                self.handle_objects_loaded(
                    &conn_name,
                    &schema,
                    items,
                    ObjKind::Trigger,
                    CategoryKind::Triggers,
                    LeafKind::Trigger,
                );
            }
            AppMessage::EventsLoaded {
                conn_name,
                schema,
                items,
            } => {
                self.handle_objects_loaded(
                    &conn_name,
                    &schema,
                    items,
                    ObjKind::Event,
                    CategoryKind::Events,
                    LeafKind::Event,
                );
            }
            AppMessage::TypeInfoLoaded {
                tab_id,
                attributes,
                methods,
                declaration,
                body,
            } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.streaming_since = None;
                    tab.type_attributes = Some(attributes);
                    tab.type_methods = Some(methods);
                    if let Some(editor) = tab.decl_editor.as_mut() {
                        editor.set_content(&declaration);
                    }
                    if let Some(editor) = tab.body_editor.as_mut() {
                        editor.set_content(&body);
                    }
                    tab.sync_grid_for_subview();
                }
                self.finish_loading();
            }
            AppMessage::TriggerInfoLoaded {
                tab_id,
                columns,
                declaration,
            } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.streaming_since = None;
                    tab.trigger_columns = Some(columns);
                    if let Some(editor) = tab.decl_editor.as_mut() {
                        editor.set_content(&declaration);
                    }
                    tab.sync_grid_for_subview();
                }
                self.finish_loading();
            }
            AppMessage::TableDataLoaded { tab_id, result } => {
                let row_count = result.rows.len();
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    let had_data = tab.query_result.is_some();
                    tab.query_result = Some(result);
                    if !had_data {
                        tab.grid_selected_row = 0;
                        tab.grid_scroll_row = 0;
                        tab.grid_on_header = true;
                    } else {
                        // Clamp position to new row count
                        tab.grid_selected_row =
                            tab.grid_selected_row.min(row_count.saturating_sub(1));
                        if tab.grid_scroll_row > tab.grid_selected_row {
                            tab.grid_scroll_row = tab.grid_selected_row;
                        }
                    }
                }
                self.state.status_message = format!("Loading... {row_count} rows");
            }
            AppMessage::TableDataBatch { tab_id, rows, done } => {
                let batch_len = rows.len();
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    if let Some(ref mut qr) = tab.query_result {
                        qr.rows.extend(rows);
                    }
                    if done {
                        tab.streaming = false;
                        tab.streaming_since = None;
                    }
                }
                let total_rows = self
                    .state
                    .find_tab(tab_id)
                    .and_then(|t| t.query_result.as_ref())
                    .map(|qr| qr.rows.len())
                    .unwrap_or(0);
                if done {
                    self.state.status_message = format!("{total_rows} rows loaded");
                    self.finish_loading();
                } else {
                    self.state.status_message =
                        format!("Loading... {total_rows} rows (+{batch_len})");
                }
            }
            AppMessage::ColumnsLoaded { tab_id, columns } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.columns = columns;
                }
                self.finish_loading();
            }
            AppMessage::PackageContentLoaded { tab_id, content } => {
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

                // Note: package member caching for completion now lives in
                // PackageMembersLoaded too, so this code path stays focused
                // on the tab. Cache the package members in the per-connection
                // MetadataIndex
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
            AppMessage::PackageMembersLoaded {
                conn_name,
                schema,
                package,
                declaration,
            } => {
                // Reuse the existing extractor: pull function/procedure names
                // out of the declaration text and stash them in the per-connection
                // MetadataIndex so completion picks them up on the next keystroke.
                use crate::sql_engine::metadata::{PackageMember, PackageMemberKind};
                let funcs = extract_names(&declaration, "FUNCTION");
                let procs = extract_names(&declaration, "PROCEDURE");
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
                if let Some(idx) = self.state.engine.metadata_indexes.get_mut(&conn_name) {
                    idx.set_package_members(&schema, &package, members);
                }
                // Re-fire completion so the popup picks up the freshly cached
                // members without the user needing to type another keystroke.
                let _ = crate::ui::events::editor::update_completion_impl(&mut self.state, true);
            }
            AppMessage::FunctionReturnColumnsLoaded {
                conn_name,
                schema,
                package,
                function,
                columns,
            } => {
                use crate::sql_engine::models::ResolvedColumn;
                let resolved: Vec<ResolvedColumn> = columns
                    .into_iter()
                    .map(|c| ResolvedColumn {
                        name: c.name,
                        data_type: c.data_type,
                        nullable: c.nullable,
                        is_primary_key: c.is_primary_key,
                        table_schema: schema.clone().unwrap_or_default(),
                        table_name: function.clone(),
                    })
                    .collect();
                if let Some(idx) = self.state.engine.metadata_indexes.get_mut(&conn_name) {
                    idx.cache_function_return_columns(
                        schema.as_deref(),
                        package.as_deref(),
                        &function,
                        resolved,
                    );
                }
                // Re-fire completion so the popup refreshes with the new columns.
                let _ = crate::ui::events::editor::update_completion_impl(&mut self.state, true);
            }
            AppMessage::QueryBatch {
                tab_id,
                columns,
                rows,
                done,
                new_tab,
                elapsed,
            } => {
                let batch_len = rows.len();
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    let is_script = matches!(tab.kind, TabKind::Script { .. });
                    if is_script {
                        // For scripts: append to active result tab or create one
                        let rt_idx = if tab.result_tabs.is_empty() || (new_tab && !tab.streaming) {
                            use crate::ui::tabs::ResultTab;
                            let label = format!("Result {}", tab.result_tabs.len() + 1);
                            let rt = ResultTab {
                                label,
                                result: QueryResult {
                                    columns,
                                    rows,
                                    elapsed: None,
                                },
                                error_editor: None,
                                query_editor: None,
                                scroll_row: 0,
                                selected_row: 0,
                                selected_col: 0,
                                visible_height: 20,
                                selection_anchor: None,
                            };
                            tab.result_tabs.push(rt);
                            tab.active_result_idx = tab.result_tabs.len() - 1;
                            tab.grid_focused = false;
                            tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
                            tab.result_tabs.len() - 1
                        } else if tab.streaming {
                            // Append to the current streaming result tab
                            let idx = tab.active_result_idx;
                            if idx < tab.result_tabs.len() {
                                tab.result_tabs[idx].result.rows.extend(rows);
                            }
                            idx
                        } else {
                            // Replace active result tab
                            use crate::ui::tabs::ResultTab;
                            let idx = tab.active_result_idx;
                            let label = format!("Result {}", idx + 1);
                            let rt = ResultTab {
                                label,
                                result: QueryResult {
                                    columns,
                                    rows,
                                    elapsed: None,
                                },
                                error_editor: None,
                                query_editor: None,
                                scroll_row: 0,
                                selected_row: 0,
                                selected_col: 0,
                                visible_height: 20,
                                selection_anchor: None,
                            };
                            if idx < tab.result_tabs.len() {
                                tab.result_tabs[idx] = rt;
                            } else {
                                tab.result_tabs.push(rt);
                                tab.active_result_idx = tab.result_tabs.len() - 1;
                            }
                            tab.active_result_idx
                        };
                        tab.streaming = !done;
                        if done {
                            tab.streaming_abort = None;
                        }
                        let _ = rt_idx;
                    } else {
                        // Table/view tab: append rows
                        if let Some(ref mut qr) = tab.query_result {
                            qr.rows.extend(rows);
                        } else {
                            tab.query_result = Some(QueryResult {
                                columns,
                                rows,
                                elapsed: None,
                            });
                            tab.grid_selected_row = 0;
                            tab.grid_scroll_row = 0;
                        }
                        tab.streaming = !done;
                        if done {
                            tab.streaming_abort = None;
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
                    self.state.status_message =
                        format!("Loading... {total_rows} rows (+{batch_len})");
                }
            }
            AppMessage::QueryFailed {
                tab_id,
                error,
                query,
                new_tab,
                start_line,
            } => {
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
                        let mut q_editor =
                            VimEditor::new(&query, vimltui::VimModeConfig::read_only());
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
                    }
                }
                self.finish_loading();
            }
            AppMessage::TableDDLLoaded { tab_id, ddl } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.streaming_since = None;
                    if let Some(editor) = tab.ddl_editor.as_mut() {
                        editor.set_content(&ddl);
                    }
                }
                self.finish_loading();
            }
            AppMessage::GridChangesSaved { tab_id, count } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.grid_changes.clear();
                    tab.grid_error_editor = None;
                    tab.grid_query_editor = None;
                    tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
                }
                self.state.status_message = format!("{count} changes saved");
                self.finish_loading();
                // Reload table data to get fresh state
                if let Some(tab) = self.state.find_tab(tab_id)
                    && let TabKind::Table { schema, table, .. } = &tab.kind
                {
                    let s = schema.clone();
                    let t = table.clone();
                    self.spawn_load_table_data(tab_id, &s, &t);
                }
            }
            AppMessage::GridChangesError {
                tab_id,
                error_text,
                sql_text,
            } => {
                use vimltui::VimEditor;

                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    let header = "-- Save Error --\n\n";
                    let formatted = format!("{header}{}", wrap_error_text(&error_text, 40));
                    let mut err_editor =
                        VimEditor::new(&formatted, vimltui::VimModeConfig::read_only());
                    err_editor.mode = vimltui::VimMode::Normal;

                    let mut q_editor =
                        VimEditor::new(&sql_text, vimltui::VimModeConfig::read_only());
                    q_editor.mode = vimltui::VimMode::Normal;

                    tab.grid_error_editor = Some(err_editor);
                    tab.grid_query_editor = Some(q_editor);
                    tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
                }
                self.state.status_message = "Save failed — see error below".to_string();
                self.finish_loading();
            }
            AppMessage::SourceCodeLoaded { tab_id, source } => {
                let conn_name = self.state.find_tab(tab_id).and_then(|t| match &t.kind {
                    TabKind::Function { conn_name, .. } | TabKind::Procedure { conn_name, .. } => {
                        Some(conn_name.clone())
                    }
                    _ => None,
                });

                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.streaming_since = None;
                    tab.original_source = Some(source.clone());
                    if let Some(editor) = tab.editor.as_mut() {
                        editor.set_content(&source);
                    }
                }

                // Register in VFS
                if let Some(cn) = conn_name {
                    self.register_in_vfs(tab_id, &cn);
                }
                self.finish_loading();
            }
            AppMessage::Connected { adapter, name } => {
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
                                    .send(AppMessage::SchemasLoaded { conn_name, schemas })
                                    .await;
                            }
                            Err(e) => {
                                let _ = tx.send(AppMessage::Error(e.to_string())).await;
                            }
                        }
                    });
                } else {
                    self.add_connection(adapter, &name);
                }

                self.state.status_message = format!("Connected to {name}");
                self.finish_loading();
            }
            AppMessage::ObjectDropped {
                schema,
                name,
                obj_type,
            } => {
                // Remove from tree
                if let Some(idx) = self.state.sidebar.tree.iter().position(|n| {
                    matches!(n, TreeNode::Leaf { name: n, schema: s, .. } if n == &name && s == &schema)
                }) {
                    self.state.sidebar.tree.remove(idx);
                }
                self.state.status_message = format!("{obj_type} {schema}.{name} dropped");
                self.finish_loading();
            }
            AppMessage::ObjectRenamed {
                schema,
                old_name,
                new_name,
                obj_type,
            } => {
                // Update name in tree
                for node in &mut self.state.sidebar.tree {
                    if let TreeNode::Leaf {
                        name, schema: s, ..
                    } = node
                        && *name == old_name
                        && *s == schema
                    {
                        *name = new_name.clone();
                        break;
                    }
                }
                self.state.status_message = format!("{obj_type} {schema}.{old_name} → {new_name}");
                self.finish_loading();
            }
            AppMessage::ObjectError { error, sql } => {
                // Show error in active tab if it has an editor, or in status bar
                if let Some(tab) = self.state.active_tab_mut() {
                    use vimltui::VimEditor;
                    let formatted = format!("-- Error --\n\n{error}");
                    let mut err_editor =
                        VimEditor::new(&formatted, vimltui::VimModeConfig::read_only());
                    err_editor.mode = vimltui::VimMode::Normal;
                    let mut q_editor = VimEditor::new(&sql, vimltui::VimModeConfig::read_only());
                    q_editor.mode = vimltui::VimMode::Normal;
                    tab.grid_error_editor = Some(err_editor);
                    tab.grid_query_editor = Some(q_editor);
                }
                self.state.status_message = format!("Error: {error}");
                self.finish_loading();
            }
            AppMessage::DdlExecuted { query } => {
                // Refresh the relevant tree category based on the DDL statement
                let upper = query.trim_start().to_uppercase();
                let kind = if upper.contains("TABLE") {
                    Some("Tables")
                } else if upper.contains("VIEW") {
                    Some("Views")
                } else if upper.contains("PACKAGE") {
                    Some("Packages")
                } else if upper.contains("INDEX") {
                    Some("Indexes")
                } else if upper.contains("SEQUENCE") {
                    Some("Sequences")
                } else if upper.contains("TRIGGER") {
                    Some("Triggers")
                } else if upper.contains("TYPE") {
                    Some("Types")
                } else if upper.contains("FUNCTION") {
                    Some("Functions")
                } else if upper.contains("PROCEDURE") {
                    Some("Procedures")
                } else {
                    None
                };
                if let Some(kind) = kind
                    && let Some(schema) = self.state.conn.current_schema.clone()
                {
                    self.spawn_load_children(&schema, kind);
                }
            }
            AppMessage::Error(msg) => {
                if matches!(
                    self.state.overlay,
                    Some(crate::ui::state::Overlay::ConnectionDialog)
                ) {
                    self.state.dialogs.connection_form.error_message = msg.clone();
                    self.state.dialogs.connection_form.connecting = false;
                    self.state.dialogs.connection_form.connecting_since = None;

                    let config = self.state.dialogs.connection_form.to_connection_config();
                    if !config.name.is_empty() {
                        self.save_connection_config(&config);
                        let exists = self.state.sidebar.tree.iter().any(|n| {
                            matches!(n, TreeNode::Connection { name, .. } if name == &config.name)
                        });
                        if !exists {
                            let insert_idx = self.find_or_create_group_insert_idx(&config.group);
                            self.state.sidebar.tree.insert(
                                insert_idx,
                                TreeNode::Connection {
                                    name: config.name.clone(),
                                    expanded: false,
                                    status: crate::ui::state::ConnStatus::Failed,
                                },
                            );
                        } else {
                            self.set_conn_status(
                                &config.name,
                                crate::ui::state::ConnStatus::Failed,
                            );
                        }
                    }
                }
                for node in &mut self.state.sidebar.tree {
                    if let TreeNode::Connection { status, .. } = node
                        && *status == crate::ui::state::ConnStatus::Connecting
                    {
                        *status = crate::ui::state::ConnStatus::Failed;
                    }
                }
                // Clear any per-tab streaming/loading spinners — without this
                // the tab's "fetching data..." indicator stays on forever after
                // a failed DDL / source / type fetch.
                for tab in &mut self.state.tabs {
                    if tab.streaming_since.is_some() {
                        tab.streaming_since = None;
                        tab.streaming = false;
                    }
                }
                // Status bar only shows the first line — friendly connection
                // errors are multi-line and the detail/hint lines are already
                // rendered inside the connection dialog itself.
                let headline = msg.lines().next().unwrap_or(&msg);
                self.state.status_message = format!("Error: {headline}");
                self.finish_loading();
            }
            AppMessage::ValidationResult { tab_id, report } => {
                if report.is_valid {
                    // Validation passed - save locally
                    if let Some(tab) = self.state.find_tab_mut(tab_id) {
                        // Mark editors as not modified (saved locally)
                        if let Some(editor) = tab.active_editor_mut() {
                            editor.modified = false;
                        }
                    }
                    self.sync_tab_to_vfs(tab_id, true);
                    self.state.status_message = "Saved locally (Ctrl+S)".to_string();
                } else {
                    // Validation failed
                    let error_msg = report.error_summary();
                    self.sync_tab_to_vfs_error(tab_id, error_msg.clone());
                    self.state.status_message = format!("Validation failed: {error_msg}");
                }
                self.finish_loading();
            }
            AppMessage::CompileResult {
                tab_id,
                success,
                message,
                failed_sql,
                failed_part,
            } => {
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
                    self.state.status_message = format!("✓ {obj_label} compiled successfully");
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

                        let mut q_editor =
                            VimEditor::new(&failed_sql, vimltui::VimModeConfig::read_only());
                        q_editor.mode = vimltui::VimMode::Normal;

                        tab.grid_error_editor = Some(err_editor);
                        tab.grid_query_editor = Some(q_editor);
                        tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
                    }

                    self.state.status_message = format!("Compilation failed: {message}");
                }
                self.finish_loading();
            }
            AppMessage::ColumnsCached {
                conn_name,
                key,
                columns,
            } => {
                // Also populate MetadataIndex with resolved columns
                if let Some(dot) = key.find('.') {
                    let schema = &key[..dot];
                    let table = &key[dot + 1..];
                    let resolved: Vec<crate::sql_engine::models::ResolvedColumn> = columns
                        .iter()
                        .map(|c| crate::sql_engine::models::ResolvedColumn {
                            name: c.name.clone(),
                            data_type: c.data_type.clone(),
                            nullable: c.nullable,
                            is_primary_key: c.is_primary_key,
                            table_schema: schema.to_string(),
                            table_name: table.to_string(),
                        })
                        .collect();
                    let idx = self
                        .state
                        .engine
                        .metadata_indexes
                        .entry(conn_name)
                        .or_default();
                    idx.cache_columns(schema, table, resolved);
                }
                self.state.engine.column_cache.insert(key, columns);
            }
        }
    }

    fn insert_leaves<T: HasName>(
        &mut self,
        conn_name: &str,
        schema: &str,
        category: CategoryKind,
        items: Vec<T>,
        leaf_kind: LeafKind,
    ) {
        let cat_idx = self.find_category_in_connection(conn_name, schema, &category);
        if let Some(idx) = cat_idx {
            self.remove_children_of(idx);

            if items.is_empty() {
                self.state.sidebar.tree.insert(idx + 1, TreeNode::Empty);
                return;
            }

            // Build batch and splice (O(n) instead of O(n²))
            let batch: Vec<TreeNode> = items
                .iter()
                .map(|item| TreeNode::Leaf {
                    name: item.get_name(),
                    schema: schema.to_string(),
                    kind: leaf_kind.clone(),
                    valid: item.is_valid(),
                    privilege: item.get_privilege(),
                })
                .collect();
            let insert_pos = idx + 1;
            self.state
                .sidebar
                .tree
                .splice(insert_pos..insert_pos, batch);
        }
    }

    fn insert_package_leaves(&mut self, conn_name: &str, schema: &str, items: Vec<Package>) {
        let cat_idx = self.find_category_in_connection(conn_name, schema, &CategoryKind::Packages);
        if let Some(idx) = cat_idx {
            self.remove_children_of(idx);

            if items.is_empty() {
                self.state.sidebar.tree.insert(idx + 1, TreeNode::Empty);
                return;
            }

            let batch: Vec<TreeNode> = items
                .into_iter()
                .map(|pkg| TreeNode::Leaf {
                    name: pkg.name,
                    schema: schema.to_string(),
                    kind: LeafKind::Package,
                    valid: pkg.valid,
                    privilege: pkg.privilege,
                })
                .collect();
            let insert_pos = idx + 1;
            self.state
                .sidebar
                .tree
                .splice(insert_pos..insert_pos, batch);
        }
    }

    /// Find a Category node within a specific connection's subtree.
    fn find_category_in_connection(
        &self,
        conn_name: &str,
        schema: &str,
        category: &CategoryKind,
    ) -> Option<usize> {
        let tree = &self.state.sidebar.tree;
        // Find the connection node first
        let conn_idx = tree
            .iter()
            .position(|n| matches!(n, TreeNode::Connection { name, .. } if name == conn_name))?;
        let conn_depth = tree[conn_idx].depth();
        // Search within this connection's subtree
        let mut i = conn_idx + 1;
        while i < tree.len() && tree[i].depth() > conn_depth {
            if matches!(&tree[i], TreeNode::Category { schema: s, kind, .. } if s == schema && kind == category)
            {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    fn remove_children_of(&mut self, parent_idx: usize) {
        let parent_depth = self.state.sidebar.tree[parent_idx].depth();
        let start = parent_idx + 1;
        let mut end = start;
        while end < self.state.sidebar.tree.len()
            && self.state.sidebar.tree[end].depth() > parent_depth
        {
            end += 1;
        }
        if end > start {
            self.state.sidebar.tree.drain(start..end);
        }
    }
}

trait HasName {
    fn get_name(&self) -> String;
    fn is_valid(&self) -> bool;
    fn get_privilege(&self) -> ObjectPrivilege;
}
impl HasName for Table {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        self.privilege
    }
}
impl HasName for View {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        self.valid
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        self.privilege
    }
}
impl HasName for Procedure {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        self.valid
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        self.privilege
    }
}
impl HasName for Function {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        self.valid
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        self.privilege
    }
}
impl HasName for MaterializedView {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        self.valid
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        self.privilege
    }
}
impl HasName for Index {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        ObjectPrivilege::Unknown
    }
}
impl HasName for Sequence {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        ObjectPrivilege::Unknown
    }
}
impl HasName for DbType {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        ObjectPrivilege::Unknown
    }
}
impl HasName for Trigger {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        ObjectPrivilege::Unknown
    }
}
impl HasName for DbEvent {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        ObjectPrivilege::Unknown
    }
}

/// Extract FUNCTION or PROCEDURE names from a PL/SQL package declaration/body.
/// Looks for lines like "FUNCTION name" or "PROCEDURE name".
fn extract_names(source: &str, kind: &str) -> Vec<String> {
    let kind_upper = kind.to_uppercase();
    let kind_len = kind_upper.len();
    let mut names = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        let trimmed_upper = trimmed.to_uppercase();
        if let Some(rest_upper) = trimmed_upper.strip_prefix(&kind_upper)
            && rest_upper.starts_with(|c: char| c.is_whitespace())
        {
            // Get the original-case name from the original line
            let original_rest = &trimmed[kind_len..].trim_start();
            let name: String = original_rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Word-wrap error text to fit within `max_width` columns.
fn wrap_error_text(error: &str, max_width: usize) -> String {
    let mut lines = Vec::new();

    // Extract line number from error (MySQL: "at line N", PostgreSQL: "LINE N:", Oracle: "line N")
    let line_num = extract_error_line(error);
    let header = match line_num {
        Some(n) => format!("-- Query Error (line {n}) --"),
        None => "-- Query Error --".to_string(),
    };
    lines.push(header);
    lines.push(String::new());

    // Strip SQL snippets from error (already shown in Query pane)
    // e.g. "...near 'SELECT * FROM...' at line 1"
    let cleaned = if let Some(pos) = error.find(" near '") {
        let before = &error[..pos];
        // Try to find "at line N" after the snippet
        let after = error[pos..]
            .find("' at line ")
            .map(|p| &error[pos + p + 1..])
            .unwrap_or("");
        format!("{before}{after}")
    } else {
        error.to_string()
    };

    // Split on ": " to break long error chains into sections
    for section in cleaned.split(": ") {
        let section = section.trim();
        if section.is_empty() {
            continue;
        }
        // Word-wrap each section
        let mut current_line = String::new();
        for word in section.split_whitespace() {
            if current_line.is_empty() {
                current_line.push_str(word);
            } else if current_line.len() + 1 + word.len() > max_width {
                lines.push(current_line);
                current_line = format!("  {word}"); // indent continuation
            } else {
                current_line.push(' ');
                current_line.push_str(word);
            }
        }
        if !current_line.is_empty() {
            lines.push(current_line);
        }
    }

    lines.push(String::new());
    lines.join("\n")
}

/// Extract line number from database error messages.
/// Matches patterns: "at line N", "LINE N:", "line N,", "ORA-NNNNN: ... line N"
fn extract_error_line(error: &str) -> Option<usize> {
    let lower = error.to_lowercase();

    // "at line N" (MySQL)
    if let Some(pos) = lower.find("at line ") {
        let after = &error[pos + 8..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = num.parse::<usize>() {
            return Some(n);
        }
    }

    // "LINE N:" (PostgreSQL)
    if let Some(pos) = lower.find("line ") {
        let after = &error[pos + 5..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = num.parse::<usize>() {
            return Some(n);
        }
    }

    None
}
