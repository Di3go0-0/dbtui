use super::*;

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
                self.spawn_execute_query_at(tab_id, &query, false, start_line);
            }
            Action::ExecuteQueryNewTab {
                tab_id,
                query,
                start_line,
            } => {
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
                    schema: None,
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
            Action::LoadCatalogSchemas { catalog } => {
                let conn_name = self
                    .state
                    .selected_tree_index()
                    .and_then(|idx| self.state.connection_for_tree_idx(idx))
                    .map(|c| c.to_string());
                if let Some(conn_name) = conn_name {
                    self.spawn_load_catalog_schemas(&conn_name, &catalog);
                }
            }
            Action::OpenScriptSchemaPicker => {
                self.open_script_schema_picker();
            }
            Action::SetScriptSchema { schema } => {
                self.set_script_schema(schema);
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
                            position: e.position(),
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
                            position: None,
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
                    position: None,
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
                    // Renaming to the same name would write the file and then
                    // delete it under its "old" path — the same file.
                    if new_path == old_path {
                        return;
                    }
                    if store.read(&new_path).is_ok() {
                        self.state.status_message =
                            format!("A script named '{new_name}' already exists");
                        return;
                    }
                    match store.read(&old_path) {
                        Ok(content) => {
                            // The original only goes once the copy is on disk.
                            if let Err(e) = store.save(&new_path, &content) {
                                self.state.status_message = format!("Cannot rename: {e}");
                                return;
                            }
                            if let Err(e) = store.delete(&old_path) {
                                self.state.status_message =
                                    format!("Renamed, but the old file remains: {e}");
                            }
                            Self::update_tabs_for_script_path_change(
                                &mut self.state.tabs,
                                &old_path,
                                &new_path,
                                Some(&new_name),
                            );
                        }
                        Err(e) => self.state.status_message = format!("Cannot rename: {e}"),
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
