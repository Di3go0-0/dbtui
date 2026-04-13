use super::*;

/// Grid change SQL building context, extracted from tab state.
struct GridChangeContext {
    schema: String,
    table: String,
    pk_cols: Vec<(usize, String)>,
    all_col_names: Vec<String>,
}

/// Result of building grid change SQL statements.
enum GridBuildResult {
    /// Statements ready to execute.
    Statements(Vec<String>),
    /// An error message to show (e.g. no PK).
    Error(String),
    /// Nothing to do.
    Empty,
}

impl App {
    /// Dispatch a single `Action` returned by the event handler.
    /// This is the main action routing extracted from the `run()` loop.
    pub(super) fn dispatch_action(&mut self, action: Action) {
        match action {
            Action::Quit | Action::Render | Action::None => {}
            Action::LoadSchemas { conn_name } => {
                self.spawn_load_schemas(&conn_name);
            }
            Action::LoadChildren { schema, kind } => {
                self.spawn_load_children(&schema, &kind);
            }
            Action::RefreshSchema { schema, kinds } => {
                for kind in kinds {
                    self.spawn_load_children(&schema, &kind);
                }
            }
            Action::LoadPackageMembers { schema, package } => {
                self.spawn_load_package_members(&schema, &package);
            }
            Action::LoadFunctionReturnColumns {
                schema,
                package,
                function,
            } => {
                self.spawn_load_function_return_columns(
                    schema.as_deref(),
                    package.as_deref(),
                    &function,
                );
            }
            Action::LoadTableData {
                tab_id,
                schema,
                table,
            } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.streaming = true;
                    tab.streaming_since = Some(std::time::Instant::now());
                }
                self.spawn_load_table_data(tab_id, &schema, &table);
            }
            Action::LoadPackageContent {
                tab_id,
                schema,
                name,
            } => {
                self.state.loading = true;
                self.state.loading_since = Some(std::time::Instant::now());
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.streaming_since = Some(std::time::Instant::now());
                }
                self.state.status_message = format!("Loading {name}...");
                self.spawn_load_package_content(tab_id, &schema, &name);
            }
            Action::ExecuteQuery {
                tab_id,
                query,
                start_line,
            } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.streaming = true;
                    tab.streaming_since = Some(std::time::Instant::now());
                    tab.first_batch_pending = true;
                    tab.pending_query = Some((query.clone(), start_line));
                }
                self.spawn_execute_query_at(tab_id, &query, false, start_line);
            }
            Action::ExecuteQueryNewTab {
                tab_id,
                query,
                start_line,
            } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.streaming = true;
                    tab.streaming_since = Some(std::time::Instant::now());
                    tab.first_batch_pending = true;
                    tab.pending_query = Some((query.clone(), start_line));
                }
                self.spawn_execute_query_at(tab_id, &query, true, start_line);
            }
            Action::LoadSourceCode {
                tab_id,
                schema,
                name,
                obj_type,
            } => {
                self.state.loading = true;
                self.state.loading_since = Some(std::time::Instant::now());
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.streaming_since = Some(std::time::Instant::now());
                }
                self.spawn_load_source_code(tab_id, &schema, &name, &obj_type);
            }
            Action::OpenNewScript => {
                let script_num = self
                    .state
                    .tabs
                    .iter()
                    .filter(|t| matches!(t.kind, TabKind::Script { .. }))
                    .count()
                    + 1;
                let name = format!("Script {script_num}");
                self.state.open_or_focus_tab(TabKind::Script {
                    file_path: None,
                    name,
                    conn_name: None,
                });
            }
            Action::CloseTab => {
                self.handle_close_tab();
            }
            Action::SaveScript => {
                self.save_active_script();
            }
            Action::SaveScriptAs { name } => {
                self.do_save_script(Some(&name));
            }
            Action::ConfirmCloseYes => {
                self.save_active_script();
                self.abort_active_streaming();
                self.state.close_active_tab();
            }
            Action::ConfirmCloseNo => {
                self.abort_active_streaming();
                self.state.close_active_tab();
            }
            Action::OpenScript { name } => {
                self.open_script(&name);
            }
            Action::Connect => {
                self.spawn_connect();
            }
            Action::InlineConnSaveAndConnect => {
                self.inline_conn_save_and_connect();
            }
            Action::SaveConnection => {
                self.save_current_connection();
            }
            Action::DeleteConnection { name } => {
                self.delete_connection(&name);
            }
            Action::ConnectByName { name } => {
                self.connect_by_name(&name);
            }
            Action::DisconnectByName { name } => {
                self.disconnect_by_name(&name);
            }
            Action::SaveSchemaFilter => {
                self.save_object_filter();
            }
            Action::ValidateAndSave { tab_id } => {
                self.handle_validate_and_save(tab_id);
            }
            Action::CompileToDb { tab_id } => {
                if self.state.compile_confirmed {
                    self.state.compile_confirmed = false;
                    self.handle_compile_to_db(tab_id);
                } else {
                    self.state.overlay = Some(crate::ui::state::Overlay::ConfirmCompile);
                }
            }
            Action::CreateSplit => {
                self.handle_create_split();
            }
            Action::CloseGroup => {
                self.handle_close_group();
            }
            Action::MoveTabToOther => {
                self.handle_move_tab_to_other();
            }
            Action::OpenScriptConnPicker => {
                self.open_script_conn_picker();
            }
            Action::SetScriptConnection { conn_name } => {
                self.set_script_connection(&conn_name);
            }
            Action::OpenThemePicker => {
                self.state.overlay = Some(crate::ui::state::Overlay::ThemePicker);
            }
            Action::SetTheme { name } => {
                self.theme = crate::ui::theme::Theme::by_name(&name);
                self.save_theme_preference(&name);
                self.state.status_message = format!("Theme: {name}");
            }
            Action::CacheColumns { schema, table } => {
                let key = format!("{}.{}", schema.to_uppercase(), table.to_uppercase());
                if !self.state.engine.column_cache.contains_key(&key) {
                    self.spawn_cache_columns(&schema, &table, key);
                }
            }
            Action::CacheSchemaObjects { schema } => {
                let eff_conn = self
                    .state
                    .active_tab()
                    .and_then(|t| t.kind.conn_name().map(|s| s.to_string()))
                    .or_else(|| self.state.conn.name.clone());
                let has_objects = eff_conn
                    .as_ref()
                    .and_then(|cn| self.state.engine.metadata_indexes.get(cn))
                    .map(|idx| {
                        !idx.objects_by_kind(
                            Some(&schema),
                            &[
                                crate::sql_engine::metadata::ObjectKind::Table,
                                crate::sql_engine::metadata::ObjectKind::View,
                            ],
                        )
                        .is_empty()
                    })
                    .unwrap_or(false);
                if !has_objects {
                    self.spawn_load_children(&schema, "Tables");
                    self.spawn_load_children(&schema, "Views");
                }
            }
            Action::ScriptOp { op } => {
                self.handle_script_op(op);
            }
            Action::ReloadTableData => {
                if let Some(tab) = self.state.active_tab_mut() {
                    tab.grid_changes.clear();
                }
                let tab_id = self.state.tabs[self.state.active_tab_idx].id;
                if let Some(tab) = self.state.find_tab(tab_id)
                    && let TabKind::Table { schema, table, .. } = &tab.kind
                {
                    let s = schema.clone();
                    let t = table.clone();
                    self.spawn_load_table_data(tab_id, &s, &t);
                }
            }
            Action::SaveGridChanges => {
                self.execute_grid_changes();
            }
            Action::LoadTableDDL {
                tab_id,
                schema,
                table,
            } => {
                self.state.loading = true;
                self.state.loading_since = Some(std::time::Instant::now());
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.streaming_since = Some(std::time::Instant::now());
                }
                self.state.status_message = "Loading DDL...".to_string();
                self.spawn_load_table_ddl(tab_id, &schema, &table);
            }
            Action::LoadTypeInfo {
                tab_id,
                schema,
                name,
            } => {
                self.state.loading = true;
                self.state.loading_since = Some(std::time::Instant::now());
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.streaming_since = Some(std::time::Instant::now());
                }
                self.state.status_message = "Loading type info...".to_string();
                self.spawn_load_type_info(tab_id, &schema, &name);
            }
            Action::LoadTriggerInfo {
                tab_id,
                schema,
                name,
            } => {
                self.state.loading = true;
                self.state.loading_since = Some(std::time::Instant::now());
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.streaming_since = Some(std::time::Instant::now());
                }
                self.state.status_message = "Loading trigger info...".to_string();
                self.spawn_load_trigger_info(tab_id, &schema, &name);
            }
            Action::DropObject {
                conn_name,
                schema,
                name,
                obj_type,
            } => {
                self.spawn_drop_object(&conn_name, &schema, &name, &obj_type);
            }
            Action::RenameObject {
                conn_name,
                schema,
                old_name,
                new_name,
                obj_type,
            } => {
                if obj_type == "CONNECTION" {
                    self.rename_connection(&old_name, &new_name);
                } else {
                    self.spawn_rename_object(&conn_name, &schema, &old_name, &new_name, &obj_type);
                }
            }
            Action::CreateFromTemplate {
                conn_name,
                schema,
                obj_type,
            } => {
                self.open_template_script(&conn_name, &schema, &obj_type);
            }
            Action::DuplicateConnection {
                source_name,
                target_group,
            } => {
                self.duplicate_connection(&source_name, &target_group);
            }
            Action::ExportBundle => {
                self.handle_export();
            }
            Action::ImportBundle => {
                self.handle_import();
            }
        }
    }

    // ─── Grid Changes ───────────────────────────────────────────────────

    /// Build SQL statements for all pending grid changes (pure logic, no async).
    fn build_grid_change_statements(&self) -> GridBuildResult {
        use crate::ui::tabs::RowChange;

        let tab_idx = self.state.active_tab_idx;
        let tab = &self.state.tabs[tab_idx];

        let ctx = match Self::extract_grid_context(tab) {
            Some(c) => c,
            None => return GridBuildResult::Empty,
        };

        let mut statements: Vec<String> = Vec::new();

        let mut changes: Vec<(usize, &RowChange)> =
            tab.grid_changes.iter().map(|(k, v)| (*k, v)).collect();
        changes.sort_by_key(|(k, _)| *k);

        for (row_idx, change) in &changes {
            match change {
                RowChange::Modified { edits } => {
                    if ctx.pk_cols.is_empty() {
                        return GridBuildResult::Error(
                            "Cannot UPDATE: table has no primary key".to_string(),
                        );
                    }
                    let row_data = tab.query_result.as_ref().and_then(|r| r.rows.get(*row_idx));
                    if let Some(row_data) = row_data {
                        let set_clause: String = edits
                            .iter()
                            .map(|e| {
                                let col_name =
                                    ctx.all_col_names.get(e.col).cloned().unwrap_or_default();
                                format!("{} = '{}'", col_name, e.value.replace('\'', "''"))
                            })
                            .collect::<Vec<_>>()
                            .join(",\n       ");
                        let where_clause = Self::build_pk_where(&ctx.pk_cols, row_data);
                        statements.push(format!(
                            "UPDATE {}.{}\n  SET {set_clause}\n  WHERE {where_clause}",
                            ctx.schema, ctx.table
                        ));
                    }
                }
                RowChange::New { values } => {
                    let cols = ctx.all_col_names.join(", ");
                    let vals: String = values
                        .iter()
                        .map(|v| {
                            if v == "NULL" {
                                "NULL".to_string()
                            } else {
                                format!("'{}'", v.replace('\'', "''"))
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    statements.push(format!(
                        "INSERT INTO {}.{}\n  ({cols})\n  VALUES ({vals})",
                        ctx.schema, ctx.table
                    ));
                }
                RowChange::Deleted => {
                    if ctx.pk_cols.is_empty() {
                        return GridBuildResult::Error(
                            "Cannot DELETE: table has no primary key".to_string(),
                        );
                    }
                    let row_data = tab.query_result.as_ref().and_then(|r| r.rows.get(*row_idx));
                    if let Some(row_data) = row_data {
                        let where_clause = Self::build_pk_where(&ctx.pk_cols, row_data);
                        statements.push(format!(
                            "DELETE FROM {}.{}\n  WHERE {where_clause}",
                            ctx.schema, ctx.table
                        ));
                    }
                }
            }
        }

        if statements.is_empty() {
            GridBuildResult::Empty
        } else {
            GridBuildResult::Statements(statements)
        }
    }

    /// Extract schema/table/PK/column info from the active tab for grid changes.
    fn extract_grid_context(tab: &WorkspaceTab) -> Option<GridChangeContext> {
        let (schema, table) = match &tab.kind {
            TabKind::Table { schema, table, .. } => (schema.clone(), table.clone()),
            _ => return None,
        };

        let pk_cols: Vec<(usize, String)> = tab
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_primary_key)
            .map(|(i, c)| (i, c.name.clone()))
            .collect();

        let all_col_names: Vec<String> = tab
            .query_result
            .as_ref()
            .map(|r| r.columns.clone())
            .unwrap_or_default();

        Some(GridChangeContext {
            schema,
            table,
            pk_cols,
            all_col_names,
        })
    }

    /// Build a WHERE clause from primary key columns and row data.
    fn build_pk_where(pk_cols: &[(usize, String)], row_data: &[String]) -> String {
        pk_cols
            .iter()
            .map(|(i, name)| {
                let val = row_data.get(*i).cloned().unwrap_or_default();
                format!("{} = '{}'", name, val.replace('\'', "''"))
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }

    /// Execute pending grid changes: build SQL, spawn async execution.
    pub(super) fn execute_grid_changes(&mut self) {
        let statements = match self.build_grid_change_statements() {
            GridBuildResult::Statements(s) => s,
            GridBuildResult::Error(msg) => {
                self.state.status_message = msg;
                return;
            }
            GridBuildResult::Empty => {
                self.state.status_message = "No changes to save".to_string();
                return;
            }
        };

        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => {
                self.state.status_message = "No active connection".to_string();
                return;
            }
        };

        let tx = self.msg_tx.clone();
        let tab_id = self.state.tabs[self.state.active_tab_idx].id;
        let stmt_count = statements.len();

        tokio::spawn(async move {
            let mut failed_sql = Vec::new();
            let mut error_msgs = Vec::new();
            let mut success_count = 0;
            for stmt in &statements {
                match adapter.execute(stmt).await {
                    Ok(_) => success_count += 1,
                    Err(e) => {
                        failed_sql.push(stmt.clone());
                        error_msgs.push(e.to_string());
                    }
                }
            }
            if error_msgs.is_empty() {
                let _ = tx
                    .send(AppMessage::GridChangesSaved {
                        tab_id,
                        count: success_count,
                    })
                    .await;
            } else {
                let sql_text = failed_sql.join(";\n\n");
                let error_text = error_msgs.join("\n\n");
                let _ = tx
                    .send(AppMessage::GridChangesError {
                        tab_id,
                        error_text,
                        sql_text,
                    })
                    .await;
            }
        });

        self.state.status_message = format!("Executing {stmt_count} statements...");
        self.state.loading = true;
        self.state.loading_since = Some(std::time::Instant::now());
    }

    // ─── Compile to DB ──────────────────────────────────────────────────

    /// Collect the SQL statements to compile for a source tab.
    /// Returns (conn_name, schema, obj_name, obj_type, statements).
    fn collect_compile_statements(
        tab: &WorkspaceTab,
    ) -> Option<(String, String, String, String, Vec<String>)> {
        let (conn_name, obj_schema, obj_type) = match extract_source_info(tab) {
            Some((cn, schema, _content, ot)) => (cn, schema, ot),
            None => return None,
        };
        let obj_name = tab.kind.display_name().to_string();

        let sql_statements = if matches!(tab.kind, TabKind::Package { .. }) {
            let decl = tab
                .decl_editor
                .as_ref()
                .map(|e| e.content())
                .unwrap_or_default();
            let body = tab
                .body_editor
                .as_ref()
                .map(|e| e.content())
                .unwrap_or_default();
            let mut stmts = Vec::new();
            if !decl.trim().is_empty() {
                stmts.push(decl.trim().to_string());
            }
            if !body.trim().is_empty() {
                stmts.push(body.trim().to_string());
            }
            stmts
        } else {
            vec![
                tab.editor
                    .as_ref()
                    .map(|e| e.content())
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            ]
        };

        Some((conn_name, obj_schema, obj_name, obj_type, sql_statements))
    }

    /// Spawn the compile-to-DB task. Includes Oracle-specific error checking.
    pub(super) fn handle_compile_to_db(&mut self, tab_id: TabId) {
        let tab = match self.state.find_tab(tab_id) {
            Some(t) => t,
            None => return,
        };

        let (conn_name, obj_schema, obj_name, obj_type, sql_statements) =
            match Self::collect_compile_statements(tab) {
                Some(info) => info,
                None => return,
            };

        let adapter = match self.adapter_for(&conn_name) {
            Some(a) => a,
            None => {
                self.state.status_message = "Not connected".to_string();
                return;
            }
        };

        let db_type = adapter.db_type();
        let tx = self.msg_tx.clone();
        self.state.status_message = "Compiling to database...".to_string();
        self.state.loading = true;
        self.state.loading_since = Some(std::time::Instant::now());

        // First save locally, then compile
        self.sync_tab_to_vfs(tab_id, true);

        tokio::spawn(async move {
            let part_names: Vec<&str> = if sql_statements.len() > 1 {
                vec!["DECLARATION", "BODY"]
            } else {
                vec!["SOURCE"]
            };

            for (idx, sql) in sql_statements.iter().enumerate() {
                if let Err(e) = adapter.execute(sql).await {
                    let _ = tx
                        .send(AppMessage::CompileResult {
                            tab_id,
                            success: false,
                            message: e.to_string(),
                            failed_sql: sql.clone(),
                            failed_part: part_names.get(idx).unwrap_or(&"SOURCE").to_string(),
                        })
                        .await;
                    return;
                }

                // Oracle: check ALL_ERRORS for compilation errors after DDL
                if let Some(error_msg) = check_oracle_compilation_errors(
                    db_type,
                    &adapter,
                    &obj_schema,
                    &obj_name,
                    &obj_type,
                    &part_names,
                    idx,
                )
                .await
                {
                    let _ = tx
                        .send(AppMessage::CompileResult {
                            tab_id,
                            success: false,
                            message: error_msg,
                            failed_sql: sql.clone(),
                            failed_part: part_names.get(idx).unwrap_or(&"SOURCE").to_string(),
                        })
                        .await;
                    return;
                }
            }

            let _ = tx
                .send(AppMessage::CompileResult {
                    tab_id,
                    success: true,
                    message: "OK".to_string(),
                    failed_sql: String::new(),
                    failed_part: String::new(),
                })
                .await;
        });
    }

    // ─── Script Operations ──────────────────────────────────────────────

    /// Update open tabs when a script file path changes (rename/move).
    fn update_tabs_for_script_path_change(
        tabs: &mut [WorkspaceTab],
        old_path: &str,
        new_path: &str,
        new_name: Option<&str>,
    ) {
        for tab in tabs.iter_mut() {
            if let TabKind::Script {
                ref mut name,
                ref mut file_path,
                ..
            } = tab.kind
                && file_path.as_deref() == Some(old_path)
            {
                if let Some(n) = new_name {
                    *name = n.to_string();
                }
                *file_path = Some(new_path.to_string());
            }
        }
    }

    /// Update open tabs when a collection is renamed (prefix change).
    fn update_tabs_for_collection_rename(
        tabs: &mut [WorkspaceTab],
        old_prefix: &str,
        new_prefix: &str,
    ) {
        for tab in tabs.iter_mut() {
            if let TabKind::Script {
                ref mut file_path, ..
            } = tab.kind
                && let Some(fp) = file_path
                && fp.starts_with(&format!("{old_prefix}/"))
            {
                *fp = fp.replacen(old_prefix, new_prefix, 1);
            }
        }
    }

    /// Handle all script panel operations (create, delete, rename, move).
    pub(super) fn handle_script_op(&mut self, op: crate::ui::events::ScriptOperation) {
        use crate::ui::events::ScriptOperation;
        if let Ok(store) = crate::core::storage::ScriptStore::new() {
            match op {
                ScriptOperation::Create {
                    name,
                    in_collection,
                } => {
                    if name.ends_with('/') {
                        let dir_name = name.trim_end_matches('/');
                        let full_path = match &in_collection {
                            Some(coll) => format!("{coll}/{dir_name}"),
                            None => dir_name.to_string(),
                        };
                        if let Err(e) = store.create_collection(&full_path) {
                            self.state.status_message = format!("Error: {e}");
                        }
                    } else {
                        let path = match &in_collection {
                            Some(coll) => format!("{coll}/{name}"),
                            None => name.clone(),
                        };
                        if let Err(e) = store.save(&path, "") {
                            self.state.status_message = format!("Error: {e}");
                        }
                    }
                }
                ScriptOperation::Delete { path } => {
                    if let Err(e) = store.delete(&path) {
                        self.state.status_message = format!("Error: {e}");
                    }
                }
                ScriptOperation::DeleteCollection { name } => {
                    if let Err(e) = store.delete_collection(&name) {
                        self.state.status_message = format!("Cannot delete: {e}");
                    }
                }
                ScriptOperation::Rename { old_path, new_name } => {
                    let prefix = old_path.rfind('/').map(|i| &old_path[..=i]).unwrap_or("");
                    let new_path = format!("{prefix}{new_name}.sql");
                    if let Ok(content) = store.read(&old_path) {
                        let _ = store.save(&new_path, &content);
                        let _ = store.delete(&old_path);
                        Self::update_tabs_for_script_path_change(
                            &mut self.state.tabs,
                            &old_path,
                            &new_path,
                            Some(&new_name),
                        );
                    }
                }
                ScriptOperation::RenameCollection { old_name, new_name } => {
                    if let Err(e) = store.rename_collection(&old_name, &new_name) {
                        self.state.status_message = format!("Error: {e}");
                    } else {
                        Self::update_tabs_for_collection_rename(
                            &mut self.state.tabs,
                            &old_name,
                            &new_name,
                        );
                    }
                }
                ScriptOperation::Move {
                    from,
                    to_collection,
                } => {
                    let filename = from.rsplit('/').next().unwrap_or(&from);
                    let to = match &to_collection {
                        Some(coll) => format!("{coll}/{filename}"),
                        None => filename.to_string(),
                    };
                    if from != to {
                        if let Err(e) = store.move_script(&from, &to) {
                            self.state.status_message = format!("Error: {e}");
                        } else {
                            Self::update_tabs_for_script_path_change(
                                &mut self.state.tabs,
                                &from,
                                &to,
                                None,
                            );
                            self.state.status_message =
                                format!("Moved to {}", to_collection.as_deref().unwrap_or("root"));
                        }
                    }
                }
            }
        }
        self.refresh_scripts_list();
    }
}

/// Check Oracle ALL_ERRORS for compilation errors after executing a DDL statement.
/// Returns `Some(error_text)` if compilation errors were found, `None` otherwise.
async fn check_oracle_compilation_errors(
    db_type: crate::core::models::DatabaseType,
    adapter: &Arc<dyn crate::core::DatabaseAdapter>,
    obj_schema: &str,
    obj_name: &str,
    obj_type: &str,
    part_names: &[&str],
    idx: usize,
) -> Option<String> {
    if !matches!(db_type, crate::core::models::DatabaseType::Oracle) {
        return None;
    }

    let oracle_type = match part_names.get(idx) {
        Some(&"BODY") => format!("{obj_type} BODY"),
        _ => obj_type.to_string(),
    };
    let error_sql = format!(
        "SELECT line, position, text FROM all_errors \
         WHERE owner = '{}' AND name = '{}' AND type = '{}' \
         ORDER BY sequence",
        obj_schema.to_uppercase(),
        obj_name.to_uppercase(),
        oracle_type.to_uppercase(),
    );
    if let Ok(result) = adapter.execute(&error_sql).await
        && !result.rows.is_empty()
        && result.columns.len() >= 3
    {
        let mut error_text = String::new();
        for row in &result.rows {
            let line = &row[0];
            let pos = &row[1];
            let text = &row[2];
            error_text.push_str(&format!("Line {line}, Col {pos}: {text}\n"));
        }
        return Some(error_text.trim().to_string());
    }

    None
}
