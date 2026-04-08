use super::*;

/// Return true if `sql` is a PL/SQL anonymous block (starts with
/// DECLARE, BEGIN, or a labelled `<<label>> ... BEGIN`). These blocks
/// must keep their trailing `END;` — stripping the semicolon would
/// leave an incomplete statement that Oracle rejects with PLS-00103.
fn is_plsql_block(sql: &str) -> bool {
    // Walk past any leading whitespace and SQL comments.
    let bytes = sql.as_bytes();
    let mut i = 0;
    loop {
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        break;
    }
    let rest = &sql[i..];
    let upper: String = rest
        .chars()
        .take(8)
        .flat_map(|c| c.to_uppercase())
        .collect();
    upper.starts_with("DECLARE") || upper.starts_with("BEGIN")
}

impl App {
    pub(super) fn spawn_load_schemas(&mut self, conn_name: &str) {
        if let Some(adapter) = self.adapter_for(conn_name) {
            let tx = self.msg_tx.clone();
            let name = conn_name.to_string();
            tokio::spawn(async move {
                match adapter.get_schemas().await {
                    Ok(schemas) => {
                        let _ = tx
                            .send(AppMessage::SchemasLoaded {
                                conn_name: name,
                                schemas,
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppMessage::Error(e.to_string())).await;
                    }
                }
            });
            return;
        }

        self.set_conn_status(conn_name, crate::ui::state::ConnStatus::Connecting);

        let config = self
            .state
            .dialogs
            .saved_connections
            .iter()
            .find(|c| c.name == conn_name)
            .cloned();

        if let Some(config) = config {
            let tx = self.msg_tx.clone();
            let name = conn_name.to_string();
            self.state.status_message = format!("Connecting to {name}...");
            self.state.loading = true;
            self.state.loading_since = Some(std::time::Instant::now());

            tokio::spawn(async move {
                match crate::drivers::create_adapter(&config).await {
                    Ok(adapter) => {
                        let adapter: Arc<dyn crate::core::DatabaseAdapter> = adapter.into();
                        let _ = tx.send(AppMessage::Connected { adapter, name }).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppMessage::Error(e.to_string())).await;
                    }
                }
            });
        } else {
            self.state.status_message =
                format!("No saved config for '{conn_name}' - press 'a' to add");
        }
    }

    /// Load remaining schemas sequentially (one at a time) to avoid saturating the connection.
    pub(super) fn spawn_load_remaining_schemas(
        &self,
        conn_name: &str,
        schemas: Vec<String>,
        category_labels: Vec<String>,
    ) {
        let adapter = match self.adapter_for(conn_name) {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let cn = conn_name.to_string();

        tokio::spawn(async move {
            for schema in schemas {
                for label in &category_labels {
                    let result =
                        match label.as_str() {
                            "Tables" => adapter.get_tables(&schema).await.map(|items| {
                                AppMessage::TablesLoaded {
                                    conn_name: cn.clone(),
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Views" => adapter.get_views(&schema).await.map(|items| {
                                AppMessage::ViewsLoaded {
                                    conn_name: cn.clone(),
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Materialized Views" => adapter
                                .get_materialized_views(&schema)
                                .await
                                .map(|items| AppMessage::MaterializedViewsLoaded {
                                    conn_name: cn.clone(),
                                    schema: schema.clone(),
                                    items,
                                }),
                            "Indexes" => adapter.get_indexes(&schema).await.map(|items| {
                                AppMessage::IndexesLoaded {
                                    conn_name: cn.clone(),
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Sequences" => adapter.get_sequences(&schema).await.map(|items| {
                                AppMessage::SequencesLoaded {
                                    conn_name: cn.clone(),
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Types" => adapter.get_types(&schema).await.map(|items| {
                                AppMessage::TypesLoaded {
                                    conn_name: cn.clone(),
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Triggers" => adapter.get_triggers(&schema).await.map(|items| {
                                AppMessage::TriggersLoaded {
                                    conn_name: cn.clone(),
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Events" => adapter.get_events(&schema).await.map(|items| {
                                AppMessage::EventsLoaded {
                                    conn_name: cn.clone(),
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Packages" => adapter.get_packages(&schema).await.map(|items| {
                                AppMessage::PackagesLoaded {
                                    conn_name: cn.clone(),
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Procedures" => adapter.get_procedures(&schema).await.map(|items| {
                                AppMessage::ProceduresLoaded {
                                    conn_name: cn.clone(),
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Functions" => adapter.get_functions(&schema).await.map(|items| {
                                AppMessage::FunctionsLoaded {
                                    conn_name: cn.clone(),
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            _ => continue,
                        };
                    if let Ok(msg) = result {
                        let _ = tx.send(msg).await;
                    }
                }
                // Yield between schemas to keep UI responsive
                tokio::task::yield_now().await;
            }
        });
    }

    pub(super) fn spawn_load_children(&self, schema: &str, kind: &str) {
        let (conn_name, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        self.spawn_load_children_for(&conn_name, schema, kind, &adapter);
    }

    pub(super) fn spawn_load_children_for(
        &self,
        conn_name: &str,
        schema: &str,
        kind: &str,
        adapter: &Arc<dyn DatabaseAdapter>,
    ) {
        let tx = self.msg_tx.clone();
        let schema = schema.to_string();
        let kind = kind.to_string();
        let cn = conn_name.to_string();
        let adapter = Arc::clone(adapter);

        tokio::spawn(async move {
            let result =
                match kind.as_str() {
                    "Tables" => {
                        adapter
                            .get_tables(&schema)
                            .await
                            .map(|items| AppMessage::TablesLoaded {
                                conn_name: cn.clone(),
                                schema,
                                items,
                            })
                    }
                    "Views" => {
                        adapter
                            .get_views(&schema)
                            .await
                            .map(|items| AppMessage::ViewsLoaded {
                                conn_name: cn.clone(),
                                schema,
                                items,
                            })
                    }
                    "Materialized Views" => {
                        adapter.get_materialized_views(&schema).await.map(|items| {
                            AppMessage::MaterializedViewsLoaded {
                                conn_name: cn.clone(),
                                schema,
                                items,
                            }
                        })
                    }
                    "Indexes" => {
                        adapter
                            .get_indexes(&schema)
                            .await
                            .map(|items| AppMessage::IndexesLoaded {
                                conn_name: cn.clone(),
                                schema,
                                items,
                            })
                    }
                    "Sequences" => adapter.get_sequences(&schema).await.map(|items| {
                        AppMessage::SequencesLoaded {
                            conn_name: cn.clone(),
                            schema,
                            items,
                        }
                    }),
                    "Types" => {
                        adapter
                            .get_types(&schema)
                            .await
                            .map(|items| AppMessage::TypesLoaded {
                                conn_name: cn.clone(),
                                schema,
                                items,
                            })
                    }
                    "Triggers" => adapter.get_triggers(&schema).await.map(|items| {
                        AppMessage::TriggersLoaded {
                            conn_name: cn.clone(),
                            schema,
                            items,
                        }
                    }),
                    "Events" => {
                        adapter
                            .get_events(&schema)
                            .await
                            .map(|items| AppMessage::EventsLoaded {
                                conn_name: cn.clone(),
                                schema,
                                items,
                            })
                    }
                    "Packages" => adapter.get_packages(&schema).await.map(|items| {
                        AppMessage::PackagesLoaded {
                            conn_name: cn.clone(),
                            schema,
                            items,
                        }
                    }),
                    "Procedures" => adapter.get_procedures(&schema).await.map(|items| {
                        AppMessage::ProceduresLoaded {
                            conn_name: cn.clone(),
                            schema,
                            items,
                        }
                    }),
                    "Functions" => adapter.get_functions(&schema).await.map(|items| {
                        AppMessage::FunctionsLoaded {
                            conn_name: cn.clone(),
                            schema,
                            items,
                        }
                    }),
                    _ => return,
                };
            match result {
                Ok(msg) => {
                    let _ = tx.send(msg).await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(e.to_string())).await;
                }
            }
        });
    }

    pub(super) fn spawn_load_table_data(&self, tab_id: TabId, schema: &str, table: &str) {
        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let adapter_cols = Arc::clone(&adapter);
        let tx = self.msg_tx.clone();
        let query = format!("SELECT * FROM {schema}.{table}");
        let schema_owned = schema.to_string();
        let table_owned = table.to_string();

        tokio::spawn(async move {
            let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(4);
            let query_clone = query.clone();
            let tx2 = tx.clone();

            let stream_handle =
                tokio::spawn(
                    async move { adapter.execute_streaming(&query_clone, batch_tx).await },
                );

            let mut first = true;
            while let Some(batch_result) = batch_rx.recv().await {
                match batch_result {
                    Ok(batch) => {
                        let done = batch.done;
                        if first {
                            first = false;
                            let _ = tx2
                                .send(AppMessage::TableDataLoaded {
                                    tab_id,
                                    result: QueryResult {
                                        columns: batch.columns,
                                        rows: batch.rows,
                                        elapsed: None,
                                    },
                                })
                                .await;
                        } else {
                            let _ = tx2
                                .send(AppMessage::TableDataBatch {
                                    tab_id,
                                    rows: batch.rows,
                                    done,
                                })
                                .await;
                        }
                        if done {
                            let _ = tx2
                                .send(AppMessage::TableDataBatch {
                                    tab_id,
                                    rows: vec![],
                                    done: true,
                                })
                                .await;
                        }
                    }
                    Err(e) => {
                        let _ = tx2.send(AppMessage::Error(e.to_string())).await;
                        break;
                    }
                }
            }

            if let Ok(Err(e)) = stream_handle.await {
                let _ = tx2.send(AppMessage::Error(e.to_string())).await;
            }

            // Load columns
            match adapter_cols.get_columns(&schema_owned, &table_owned).await {
                Ok(columns) => {
                    let _ = tx.send(AppMessage::ColumnsLoaded { tab_id, columns }).await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(e.to_string())).await;
                }
            }
        });
    }

    #[allow(dead_code)]
    pub(super) fn spawn_load_columns(&self, tab_id: TabId, schema: &str, table: &str) {
        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let schema = schema.to_string();
        let table = table.to_string();

        tokio::spawn(async move {
            match adapter.get_columns(&schema, &table).await {
                Ok(columns) => {
                    let _ = tx.send(AppMessage::ColumnsLoaded { tab_id, columns }).await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(e.to_string())).await;
                }
            }
        });
    }

    pub(super) fn spawn_cache_columns(&self, schema: &str, table: &str, key: String) {
        let (conn_name, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let s = schema.to_string();
        let t = table.to_string();

        tokio::spawn(async move {
            if let Ok(columns) = adapter.get_columns(&s, &t).await {
                let _ = tx
                    .send(AppMessage::ColumnsCached {
                        conn_name,
                        key,
                        columns,
                    })
                    .await;
            }
        });
    }

    pub(super) fn spawn_load_package_content(&self, tab_id: TabId, schema: &str, name: &str) {
        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let schema = schema.to_string();
        let name = name.to_string();

        tokio::spawn(async move {
            match adapter.get_package_content(&schema, &name).await {
                Ok(Some(content)) => {
                    let _ = tx
                        .send(AppMessage::PackageContentLoaded { tab_id, content })
                        .await;
                }
                Ok(None) => {
                    let _ = tx
                        .send(AppMessage::Error("Package not found".to_string()))
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(e.to_string())).await;
                }
            }
        });
    }

    /// On-demand load of a package's callable members so completion can
    /// suggest `pkg.foo()` without the user having to open the package in
    /// a tab first. Reuses get_package_content to grab the declaration
    /// (cheap on Oracle since DBA already has the source) and emits a
    /// PackageMembersLoaded message that the messages handler stashes in
    /// the connection's MetadataIndex.
    /// On-demand load of the pseudo-columns returned by a PL/SQL function
    /// used inside `TABLE(...)`, so that `alias.<cursor>` can suggest them
    /// after the user types the alias. Mirrors spawn_load_package_members
    /// — errors are swallowed because this fires from the completion path.
    pub(super) fn spawn_load_function_return_columns(
        &self,
        schema: Option<&str>,
        package: Option<&str>,
        function: &str,
    ) {
        let (conn_name, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let schema = schema.map(|s| s.to_string());
        let package = package.map(|s| s.to_string());
        let function = function.to_string();

        tokio::spawn(async move {
            let result = adapter
                .get_function_return_columns(schema.as_deref(), package.as_deref(), &function)
                .await;
            if let Ok(columns) = result {
                let _ = tx
                    .send(AppMessage::FunctionReturnColumnsLoaded {
                        conn_name,
                        schema,
                        package,
                        function,
                        columns,
                    })
                    .await;
            }
        });
    }

    pub(super) fn spawn_load_package_members(&self, schema: &str, package: &str) {
        // Pick the active connection's adapter — this is invoked from the
        // completion path which lives inside the active editor.
        let (conn_name, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let schema = schema.to_string();
        let package = package.to_string();

        tokio::spawn(async move {
            match adapter.get_package_content(&schema, &package).await {
                Ok(Some(content)) => {
                    let _ = tx
                        .send(AppMessage::PackageMembersLoaded {
                            conn_name,
                            schema,
                            package,
                            declaration: content.declaration,
                        })
                        .await;
                }
                _ => {
                    // Silently ignore — completion just won't have suggestions
                    // for this package. The user is mid-typing, no need to
                    // pop a noisy error.
                }
            }
        });
    }
    pub(super) fn spawn_execute_query_at(
        &mut self,
        tab_id: TabId,
        query: &str,
        new_tab: bool,
        start_line: usize,
    ) {
        // If the tab is a script with an assigned connection, use that adapter
        let adapter = self
            .state
            .find_tab(tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::Script {
                    conn_name: Some(cn),
                    ..
                } => self.adapter_for(cn),
                _ => None,
            })
            .or_else(|| self.active_adapter().map(|(_, a)| a));

        let adapter = match adapter {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        // Strip trailing semicolon for regular SQL statements — the three
        // drivers reject it. PL/SQL anonymous blocks (DECLARE/BEGIN...END;)
        // are the opposite: they REQUIRE the trailing `;` on the final END,
        // so we must leave those alone.
        let query = {
            let trimmed = query.trim_end();
            if is_plsql_block(trimmed) {
                trimmed.to_string()
            } else {
                trimmed.trim_end_matches(';').trim_end().to_string()
            }
        };

        let handle = tokio::spawn(async move {
            let start = std::time::Instant::now();
            let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(4);

            let query_clone = query.clone();
            let stream_handle =
                tokio::spawn(
                    async move { adapter.execute_streaming(&query_clone, batch_tx).await },
                );

            let mut had_error = false;
            while let Some(batch_result) = batch_rx.recv().await {
                match batch_result {
                    Ok(batch) => {
                        let done = batch.done;
                        if tx
                            .send(AppMessage::QueryBatch {
                                tab_id,
                                columns: batch.columns,
                                rows: batch.rows,
                                done,
                                new_tab,
                                elapsed: if done { Some(start.elapsed()) } else { None },
                            })
                            .await
                            .is_err()
                        {
                            // UI channel closed — abort the DB query
                            stream_handle.abort();
                            return;
                        }
                    }
                    Err(e) => {
                        had_error = true;
                        let _ = tx
                            .send(AppMessage::QueryFailed {
                                tab_id,
                                error: e.to_string(),
                                query: query.clone(),
                                new_tab,
                                start_line,
                            })
                            .await;
                        break;
                    }
                }
            }

            // If DDL succeeded, notify to refresh tree
            if !had_error {
                let trimmed = query.trim_start().to_uppercase();
                if trimmed.starts_with("CREATE")
                    || trimmed.starts_with("DROP")
                    || trimmed.starts_with("ALTER")
                    || trimmed.starts_with("RENAME")
                {
                    let _ = tx
                        .send(AppMessage::DdlExecuted {
                            query: query.clone(),
                        })
                        .await;
                }
            }

            // Check if the streaming task itself failed
            if !had_error {
                match stream_handle.await {
                    Ok(Err(e)) => {
                        let _ = tx
                            .send(AppMessage::QueryFailed {
                                tab_id,
                                error: e.to_string(),
                                query,
                                new_tab,
                                start_line,
                            })
                            .await;
                    }
                    Err(_) => {} // Task was aborted (cancelled by user)
                    _ => {}
                }
            }
        });

        // Store the abort handle so the streaming task can be cancelled
        if let Some(tab) = self.state.find_tab_mut(tab_id) {
            tab.streaming_abort = Some(handle.abort_handle());
        }
    }
    pub(super) fn spawn_load_table_ddl(&self, tab_id: TabId, schema: &str, table: &str) {
        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let schema = schema.to_string();
        let table = table.to_string();

        tokio::spawn(async move {
            match adapter.get_table_ddl(&schema, &table).await {
                Ok(ddl) => {
                    let _ = tx.send(AppMessage::TableDDLLoaded { tab_id, ddl }).await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(e.to_string())).await;
                }
            }
        });
    }

    pub(super) fn spawn_load_type_info(&self, tab_id: TabId, schema: &str, name: &str) {
        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let schema = schema.to_string();
        let name = name.to_string();

        tokio::spawn(async move {
            let attributes =
                adapter
                    .get_type_attributes(&schema, &name)
                    .await
                    .unwrap_or(QueryResult {
                        columns: vec![],
                        rows: vec![],
                        elapsed: None,
                    });
            let methods = adapter
                .get_type_methods(&schema, &name)
                .await
                .unwrap_or(QueryResult {
                    columns: vec![],
                    rows: vec![],
                    elapsed: None,
                });
            let declaration = adapter
                .get_source_code(&schema, &name, "TYPE")
                .await
                .unwrap_or_default();
            let body = adapter
                .get_source_code(&schema, &name, "TYPE_BODY")
                .await
                .unwrap_or_default();
            let _ = tx
                .send(AppMessage::TypeInfoLoaded {
                    tab_id,
                    attributes,
                    methods,
                    declaration,
                    body,
                })
                .await;
        });
    }

    pub(super) fn spawn_load_trigger_info(&self, tab_id: TabId, schema: &str, name: &str) {
        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let schema = schema.to_string();
        let name = name.to_string();

        tokio::spawn(async move {
            let columns = adapter
                .get_trigger_info(&schema, &name)
                .await
                .unwrap_or(QueryResult {
                    columns: vec![],
                    rows: vec![],
                    elapsed: None,
                });
            let declaration = adapter
                .get_source_code(&schema, &name, "TRIGGER")
                .await
                .unwrap_or_default();
            let _ = tx
                .send(AppMessage::TriggerInfoLoaded {
                    tab_id,
                    columns,
                    declaration,
                })
                .await;
        });
    }

    pub(super) fn spawn_drop_object(
        &self,
        conn_name: &str,
        schema: &str,
        name: &str,
        obj_type: &str,
    ) {
        let adapter = match self.adapter_for(conn_name) {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let schema = schema.to_string();
        let name = name.to_string();
        let obj_type = obj_type.to_string();
        let db_type = self.state.conn.db_type;

        let sql = match (obj_type.as_str(), db_type) {
            ("TABLE", Some(DatabaseType::MySQL)) => format!("DROP TABLE `{schema}`.`{name}`"),
            ("VIEW", Some(DatabaseType::MySQL)) => format!("DROP VIEW `{schema}`.`{name}`"),
            _ => format!("DROP {obj_type} {schema}.{name}"),
        };

        let sql_clone = sql.clone();
        tokio::spawn(async move {
            match adapter.execute(&sql_clone).await {
                Ok(_) => {
                    let _ = tx
                        .send(AppMessage::ObjectDropped {
                            schema,
                            name,
                            obj_type,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(AppMessage::ObjectError {
                            error: e.to_string(),
                            sql,
                        })
                        .await;
                }
            }
        });
    }

    pub(super) fn spawn_rename_object(
        &self,
        conn_name: &str,
        schema: &str,
        old_name: &str,
        new_name: &str,
        obj_type: &str,
    ) {
        let adapter = match self.adapter_for(conn_name) {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let schema = schema.to_string();
        let old_name = old_name.to_string();
        let new_name = new_name.to_string();
        let obj_type = obj_type.to_string();
        let db_type = self.state.conn.db_type;

        let sql = match (obj_type.as_str(), db_type) {
            ("TABLE", Some(DatabaseType::Oracle)) => {
                format!("ALTER TABLE {schema}.{old_name} RENAME TO {new_name}")
            }
            ("TABLE", Some(DatabaseType::PostgreSQL)) => {
                format!("ALTER TABLE {schema}.{old_name} RENAME TO {new_name}")
            }
            ("TABLE", Some(DatabaseType::MySQL)) => {
                format!("RENAME TABLE `{schema}`.`{old_name}` TO `{schema}`.`{new_name}`")
            }
            ("VIEW", Some(DatabaseType::PostgreSQL)) => {
                format!("ALTER VIEW {schema}.{old_name} RENAME TO {new_name}")
            }
            ("VIEW", Some(DatabaseType::MySQL)) => {
                format!("RENAME TABLE `{schema}`.`{old_name}` TO `{schema}`.`{new_name}`")
            }
            _ => {
                // Oracle views/packages can't be renamed via ALTER
                let _ = tx.blocking_send(AppMessage::ObjectError {
                    error: format!("Rename not supported for {obj_type} in this database"),
                    sql: String::new(),
                });
                return;
            }
        };

        let sql_clone = sql.clone();
        tokio::spawn(async move {
            match adapter.execute(&sql_clone).await {
                Ok(_) => {
                    let _ = tx
                        .send(AppMessage::ObjectRenamed {
                            schema,
                            old_name,
                            new_name,
                            obj_type,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(AppMessage::ObjectError {
                            error: e.to_string(),
                            sql,
                        })
                        .await;
                }
            }
        });
    }
    pub(super) fn spawn_load_source_code(
        &self,
        tab_id: TabId,
        schema: &str,
        name: &str,
        obj_type: &str,
    ) {
        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let schema = schema.to_string();
        let name = name.to_string();
        let obj_type = obj_type.to_string();

        tokio::spawn(async move {
            match adapter.get_source_code(&schema, &name, &obj_type).await {
                Ok(source) => {
                    let _ = tx
                        .send(AppMessage::SourceCodeLoaded { tab_id, source })
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(e.to_string())).await;
                }
            }
        });
    }

    pub(super) fn spawn_connect(&mut self) {
        let config = self.state.dialogs.connection_form.to_connection_config();
        let tx = self.msg_tx.clone();
        let conn_name = config.name.clone();

        self.state.status_message = format!("Connecting to {}...", conn_name);
        self.state.loading = true;
        self.state.loading_since = Some(std::time::Instant::now());

        tokio::spawn(async move {
            match crate::drivers::create_adapter(&config).await {
                Ok(adapter) => {
                    let adapter: Arc<dyn crate::core::DatabaseAdapter> = adapter.into();
                    let _ = tx
                        .send(AppMessage::Connected {
                            adapter,
                            name: conn_name,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(e.to_string())).await;
                }
            }
        });
    }
}
