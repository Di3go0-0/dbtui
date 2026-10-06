use super::*;

use super::message_helpers::{extract_names, wrap_error_text};
use super::query_results::{QueryBatchArrival, QueryFailure};
use crate::sql_engine::metadata::ObjectKind as ObjKind;

impl App {
    pub(super) fn handle_paste(&mut self, text: &str) {
        use crate::ui::state::Focus;
        use vimltui::VimMode;

        // Paste into the inline connection editor's current text field. It is
        // checked first: the editor floats over whatever has focus, and the
        // text below it must not receive a pasted secret.
        if let Some(editor) = self.state.dialogs.inline_conn_editor.as_mut() {
            let field = editor.current_field();
            if let Some(value) = editor.field_value_mut(field) {
                value.extend(text.chars().filter(|c| *c != '\n' && *c != '\r'));
                editor.error_message.clear();
            }
            return;
        }

        // Paste into export/import dialog fields
        if matches!(self.state.overlay, Some(Overlay::ExportDialog)) {
            if let Some(ref mut d) = self.state.dialogs.export_dialog {
                let clean: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                match d.focused {
                    crate::ui::state::ExportField::Path => {
                        d.path.push_str(&clean);
                        d.reset_completions();
                    }
                    crate::ui::state::ExportField::Password => d.password.push_str(&clean),
                    crate::ui::state::ExportField::Confirm => d.confirm.push_str(&clean),
                    _ => {}
                }
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

        // Paste into bind variables dialog
        if matches!(self.state.overlay, Some(Overlay::BindVariables)) {
            if let Some(ref mut bv) = self.state.dialogs.bind_variables {
                let clean: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                if let Some((_, value)) = bv.variables.get_mut(bv.selected_idx) {
                    value.push_str(&clean);
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
                        crate::ui::vim_utf8::handle_key(
                            editor,
                            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                        );
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
                    crate::ui::vim_utf8::insert_char(editor, ch);
                }
            }
        }
    }

    pub(super) fn handle_message(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::CatalogsLoaded {
                conn_name,
                catalogs,
            } => {
                self.handle_catalogs_loaded(conn_name, catalogs);
            }
            AppMessage::SchemasLoaded {
                conn_name,
                catalog,
                schemas,
            } => {
                self.handle_schemas_loaded(conn_name, catalog, schemas);
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
                if is_primary_schema && self.state.metadata_ready.insert(conn_name.clone()) {
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
            AppMessage::TableDataLoaded {
                tab_id,
                run_id,
                result,
            } => {
                let row_count = result.rows.len();
                let Some(tab) = self.state.find_tab_mut(tab_id) else {
                    return;
                };
                if tab.query_run_id != run_id {
                    return;
                }
                let showing_data = tab.showing_table_data();
                let had_data = tab.table_data_mut().is_some();
                *tab.table_data_mut() = Some(result);
                if showing_data {
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
            AppMessage::TableDataBatch {
                tab_id,
                run_id,
                rows,
                done,
            } => {
                let batch_len = rows.len();
                let Some(tab) = self.state.find_tab_mut(tab_id) else {
                    return;
                };
                if tab.query_run_id != run_id {
                    return;
                }
                if let Some(data) = tab.table_data_mut() {
                    data.rows.extend(rows);
                }
                if done {
                    tab.streaming = false;
                    tab.streaming_since = None;
                    tab.streaming_abort = None;
                }
                let total_rows = tab.table_data_mut().as_ref().map_or(0, |d| d.rows.len());
                if done {
                    self.state.status_message = format!("{total_rows} rows loaded");
                    self.finish_loading();
                } else {
                    self.state.status_message =
                        format!("Loading... {total_rows} rows (+{batch_len})");
                }
            }
            AppMessage::TableDataFailed {
                tab_id,
                run_id,
                error,
            } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    if tab.query_run_id != run_id {
                        return;
                    }
                    tab.streaming = false;
                    tab.streaming_since = None;
                    tab.streaming_abort = None;
                }
                let headline = error.lines().next().unwrap_or(&error);
                self.state.status_message = format!("Error: {headline}");
                self.finish_loading();
            }
            AppMessage::ColumnsLoaded { tab_id, columns } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.columns = columns;
                }
                self.finish_loading();
            }
            AppMessage::PackageContentLoaded { tab_id, content } => {
                self.handle_package_content_loaded(tab_id, content);
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
                run_id,
                columns,
                rows,
                done,
                new_tab,
                elapsed,
            } => {
                self.handle_query_batch(QueryBatchArrival {
                    tab_id,
                    run_id,
                    columns,
                    rows,
                    done,
                    new_tab,
                    elapsed,
                });
            }
            AppMessage::QueryFailed {
                tab_id,
                run_id,
                error,
                position,
                query,
                new_tab,
                start_line,
            } => {
                self.handle_query_failed(QueryFailure {
                    tab_id,
                    run_id,
                    error,
                    position,
                    query,
                    new_tab,
                    start_line,
                });
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
                applied_rows,
                error_text,
                sql_text,
            } => {
                use vimltui::VimEditor;

                let applied = applied_rows.len();
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    super::grid_changes::drop_applied_changes(tab, &applied_rows);
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
                self.state.status_message = if applied == 0 {
                    "Save failed — see error below".to_string()
                } else {
                    format!("{applied} changes saved, the rest failed — fix them and save again")
                };
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
                self.handle_connected(adapter, name);
            }
            AppMessage::ObjectDropped {
                conn_name,
                schema,
                name,
                obj_type,
            } => {
                // Remove from the tree of the connection it was dropped on —
                // another connection may list an object with the same name.
                if let Some(idx) = self.find_leaf_in_connection(&conn_name, &schema, &name) {
                    self.state.sidebar.tree.remove(idx);
                }
                self.state.status_message = format!("{obj_type} {schema}.{name} dropped");
                self.finish_loading();
            }
            AppMessage::ObjectRenamed {
                conn_name,
                schema,
                old_name,
                new_name,
                obj_type,
            } => {
                if let Some(idx) = self.find_leaf_in_connection(&conn_name, &schema, &old_name)
                    && let TreeNode::Leaf { name, .. } = &mut self.state.sidebar.tree[idx]
                {
                    *name = new_name.clone();
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
            AppMessage::ServerDiagnosticsResult {
                diagnostics,
                generation,
            } => {
                // Drop stale results from earlier dispatches.
                if generation == self.state.engine.server_diag_generation {
                    use crate::sql_engine::diagnostics::{
                        Diagnostic as EngineDiag, DiagnosticSeverity, DiagnosticSource,
                    };
                    let server_diags: Vec<crate::ui::diagnostics::Diagnostic> = diagnostics
                        .into_iter()
                        .map(|cd| {
                            let severity = match cd.severity.to_uppercase().as_str() {
                                "WARNING" => DiagnosticSeverity::Warning,
                                "INFO" | "NOTE" => DiagnosticSeverity::Info,
                                _ => DiagnosticSeverity::Error,
                            };
                            crate::ui::diagnostics::Diagnostic::from_engine(EngineDiag {
                                row: cd.line.saturating_sub(1), // 1-based → 0-based
                                col_start: cd.col.saturating_sub(1),
                                col_end: cd.col, // highlight at least one char
                                message: cd.message,
                                severity,
                                source: DiagnosticSource::Server,
                            })
                        })
                        .collect();

                    // Merge: keep local diagnostics, replace server diagnostics.
                    self.state
                        .engine
                        .diagnostics
                        .retain(|d| d.source != crate::ui::diagnostics::Source::Server);
                    self.state.engine.diagnostics.extend(server_diags);
                    self.state
                        .engine
                        .diagnostics
                        .sort_by_key(|d| (d.row, d.col_start));

                    // Re-apply gutter signs on the active editor.
                    let tab_idx = self.state.active_tab_idx;
                    crate::ui::diagnostics::apply_gutter_signs(&mut self.state, tab_idx);
                }
            }
            AppMessage::ConnectFailed { name, error } => {
                self.handle_connect_failed(name, error);
            }
            AppMessage::Error(msg) => {
                // Clear any per-tab streaming/loading spinners — without this
                // the tab's "fetching data..." indicator stays on forever after
                // a failed DDL / source / type fetch.
                for tab in &mut self.state.tabs {
                    if tab.streaming_since.is_some() {
                        tab.streaming_since = None;
                        tab.streaming = false;
                    }
                }
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
                position,
                failed_sql,
                failed_part,
            } => {
                self.handle_compile_result(
                    tab_id,
                    success,
                    message,
                    position,
                    failed_sql,
                    failed_part,
                );
            }
            AppMessage::ColumnsCached {
                conn_name,
                key,
                columns,
            } => {
                // Also populate MetadataIndex with resolved columns
                // The key is `schema.table`; on SQL Server the schema part is
                // itself `database.schema`, so the table is after the last dot.
                if let Some(dot) = key.rfind('.') {
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
}
