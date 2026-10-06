use super::*;

/// Load the top level of a connection's tree.
///
/// Engines with a catalog level (SQL Server) answer with their databases and
/// the schemas are fetched lazily when one is expanded; everything else goes
/// straight to schemas.
pub(super) async fn load_tree_root(
    adapter: Arc<dyn crate::core::DatabaseAdapter>,
    conn_name: String,
    tx: mpsc::Sender<AppMessage>,
) {
    let catalogs = match adapter.get_catalogs().await {
        Ok(catalogs) => catalogs,
        Err(e) => {
            let _ = tx.send(AppMessage::Error(e.to_string())).await;
            return;
        }
    };

    if !catalogs.is_empty() {
        let _ = tx
            .send(AppMessage::CatalogsLoaded {
                conn_name,
                catalogs,
            })
            .await;
        return;
    }

    match adapter.get_schemas().await {
        Ok(schemas) => {
            let _ = tx
                .send(AppMessage::SchemasLoaded {
                    conn_name,
                    catalog: None,
                    schemas,
                })
                .await;
        }
        Err(e) => {
            let _ = tx.send(AppMessage::Error(e.to_string())).await;
        }
    }
}

impl App {
    pub(super) fn spawn_load_schemas(&mut self, conn_name: &str) {
        if let Some(adapter) = self.adapter_for(conn_name) {
            let tx = self.msg_tx.clone();
            let name = conn_name.to_string();
            tokio::spawn(load_tree_root(adapter, name, tx));
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

    pub(super) fn spawn_load_catalog_schemas(&mut self, conn_name: &str, catalog: &str) {
        let Some(adapter) = self.adapter_for(conn_name) else {
            return;
        };
        let tx = self.msg_tx.clone();
        let name = conn_name.to_string();
        let catalog = catalog.to_string();
        tokio::spawn(async move {
            match adapter.get_schemas_in(&catalog).await {
                Ok(schemas) => {
                    let _ = tx
                        .send(AppMessage::SchemasLoaded {
                            conn_name: name,
                            catalog: Some(catalog),
                            schemas,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(e.to_string())).await;
                }
            }
        });
    }

    /// Load a table tab's rows, streaming them in batches.
    ///
    /// Each load gets a run id that its messages carry, and replaces any load
    /// still in flight for the tab — a refresh during a slow load used to
    /// interleave both streams and leave duplicate rows.
    pub(super) fn spawn_load_table_data(&mut self, tab_id: TabId, schema: &str, table: &str) {
        let Some(adapter) = self.require_tab_adapter(tab_id) else {
            return;
        };
        self.next_run_id += 1;
        let run_id = self.next_run_id;
        if let Some(tab) = self.state.find_tab_mut(tab_id) {
            if let Some(previous) = tab.streaming_abort.take() {
                previous.abort();
            }
            tab.query_run_id = run_id;
            tab.streaming = true;
            tab.streaming_since = Some(std::time::Instant::now());
        }

        let adapter_cols = Arc::clone(&adapter);
        let tx = self.msg_tx.clone();
        let query = format!(
            "SELECT * FROM {}",
            crate::sql_engine::quoting::quote_qualified(Some(adapter.db_type()), schema, table)
        );
        let schema_owned = schema.to_string();
        let table_owned = table.to_string();

        let handle = tokio::spawn(async move {
            let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(4);
            let stream_handle =
                tokio::spawn(async move { adapter.execute_streaming(&query, batch_tx).await });

            let failed = |error: String| AppMessage::TableDataFailed {
                tab_id,
                run_id,
                error,
            };

            let mut first = true;
            while let Some(batch_result) = batch_rx.recv().await {
                let batch = match batch_result {
                    Ok(batch) => batch,
                    Err(e) => {
                        let _ = tx.send(failed(e.to_string())).await;
                        return;
                    }
                };
                let message = if std::mem::take(&mut first) {
                    AppMessage::TableDataLoaded {
                        tab_id,
                        run_id,
                        result: QueryResult {
                            columns: batch.columns,
                            rows: batch.rows,
                            elapsed: None,
                        },
                    }
                } else {
                    AppMessage::TableDataBatch {
                        tab_id,
                        run_id,
                        rows: batch.rows,
                        done: false,
                    }
                };
                let _ = tx.send(message).await;
            }

            match stream_handle.await {
                Ok(Ok(())) => {
                    let _ = tx
                        .send(AppMessage::TableDataBatch {
                            tab_id,
                            run_id,
                            rows: vec![],
                            done: true,
                        })
                        .await;
                }
                Ok(Err(e)) => {
                    let _ = tx.send(failed(e.to_string())).await;
                    return;
                }
                Err(join) if join.is_panic() => {
                    let _ = tx
                        .send(failed(
                            "the driver hit an internal error while reading the table".to_string(),
                        ))
                        .await;
                    return;
                }
                Err(_) => return, // superseded or cancelled
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

        if let Some(tab) = self.state.find_tab_mut(tab_id) {
            tab.streaming_abort = Some(handle.abort_handle());
        }
    }

    #[allow(dead_code)]
    pub(super) fn spawn_load_columns(&mut self, tab_id: TabId, schema: &str, table: &str) {
        let Some(adapter) = self.require_tab_adapter(tab_id) else {
            return;
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
        let Some((conn_name, adapter)) = self.completion_adapter() else {
            return;
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

    pub(super) fn spawn_load_package_content(&mut self, tab_id: TabId, schema: &str, name: &str) {
        let Some(adapter) = self.require_tab_adapter(tab_id) else {
            return;
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
        let Some((conn_name, adapter)) = self.completion_adapter() else {
            return;
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
        let Some((conn_name, adapter)) = self.completion_adapter() else {
            return;
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
    pub(super) fn spawn_load_table_ddl(&mut self, tab_id: TabId, schema: &str, table: &str) {
        let Some(adapter) = self.require_tab_adapter(tab_id) else {
            return;
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

    pub(super) fn spawn_load_type_info(&mut self, tab_id: TabId, schema: &str, name: &str) {
        let Some(adapter) = self.require_tab_adapter(tab_id) else {
            return;
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

    pub(super) fn spawn_load_trigger_info(&mut self, tab_id: TabId, schema: &str, name: &str) {
        let Some(adapter) = self.require_tab_adapter(tab_id) else {
            return;
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
        let conn_name = conn_name.to_string();
        let schema = schema.to_string();
        let name = name.to_string();
        let obj_type = obj_type.to_string();
        let sql = format!(
            "DROP {obj_type} {}",
            crate::sql_engine::quoting::quote_qualified(Some(adapter.db_type()), &schema, &name)
        );

        let sql_clone = sql.clone();
        tokio::spawn(async move {
            match adapter.execute(&sql_clone).await {
                Ok(_) => {
                    let _ = tx
                        .send(AppMessage::ObjectDropped {
                            conn_name,
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
        let conn_name = conn_name.to_string();
        let schema = schema.to_string();
        let old_name = old_name.to_string();
        let new_name = new_name.to_string();
        let obj_type = obj_type.to_string();
        // The connection the object lives on decides the dialect — not
        // whichever connection happened to connect last.
        let db_type = adapter.db_type();
        let qualified =
            crate::sql_engine::quoting::quote_qualified(Some(db_type), &schema, &old_name);
        let target = crate::sql_engine::quoting::quote_ident(Some(db_type), &new_name);

        let sql = match (obj_type.as_str(), db_type) {
            ("TABLE", DatabaseType::Oracle | DatabaseType::PostgreSQL) => {
                format!("ALTER TABLE {qualified} RENAME TO {target}")
            }
            ("VIEW", DatabaseType::PostgreSQL) => {
                format!("ALTER VIEW {qualified} RENAME TO {target}")
            }
            ("TABLE" | "VIEW", DatabaseType::MySQL) => {
                let renamed =
                    crate::sql_engine::quoting::quote_qualified(Some(db_type), &schema, &new_name);
                format!("RENAME TABLE {qualified} TO {renamed}")
            }
            _ => {
                // Oracle views/packages can't be renamed via ALTER, and SQL
                // Server renames go through sp_rename. This runs on the
                // runtime thread, where `blocking_send` panics.
                let _ = tx.try_send(AppMessage::ObjectError {
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
                            conn_name,
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
        &mut self,
        tab_id: TabId,
        schema: &str,
        name: &str,
        obj_type: &str,
    ) {
        let Some(adapter) = self.require_tab_adapter(tab_id) else {
            return;
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
        let failed_name = conn_name.clone();

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
                    let _ = tx
                        .send(AppMessage::ConnectFailed {
                            name: failed_name,
                            error: e.to_string(),
                        })
                        .await;
                }
            }
        });
    }

    /// Spawn an async server-side compile check (Pass 4 diagnostics).
    /// Bumps the generation counter and sends results back via AppMessage.
    pub(super) fn spawn_server_diagnostics(&mut self, conn_name: &str, sql: String) {
        let adapter = match self.adapter_for(conn_name) {
            Some(a) => a,
            None => return,
        };
        // Bump generation so stale results from previous dispatches are ignored.
        self.state.engine.server_diag_generation += 1;
        let generation = self.state.engine.server_diag_generation;
        self.state.engine.last_server_diag_dispatch = Some(std::time::Instant::now());

        let tx = self.msg_tx.clone();
        tokio::spawn(async move {
            match adapter.compile_check(&sql).await {
                Ok(diagnostics) => {
                    let _ = tx
                        .send(AppMessage::ServerDiagnosticsResult {
                            diagnostics,
                            generation,
                        })
                        .await;
                }
                Err(_) => {
                    // Server diagnostics are best-effort; silently drop errors
                    // to avoid noisy popups on every keystroke.
                }
            }
        });
    }
}
