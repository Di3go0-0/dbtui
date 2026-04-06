use super::*;

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
        schemas: Vec<String>,
        category_labels: Vec<String>,
    ) {
        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();

        tokio::spawn(async move {
            for schema in schemas {
                for label in &category_labels {
                    let result =
                        match label.as_str() {
                            "Tables" => adapter.get_tables(&schema).await.map(|items| {
                                AppMessage::TablesLoaded {
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Views" => adapter.get_views(&schema).await.map(|items| {
                                AppMessage::ViewsLoaded {
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Materialized Views" => adapter
                                .get_materialized_views(&schema)
                                .await
                                .map(|items| AppMessage::MaterializedViewsLoaded {
                                    schema: schema.clone(),
                                    items,
                                }),
                            "Indexes" => adapter.get_indexes(&schema).await.map(|items| {
                                AppMessage::IndexesLoaded {
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Sequences" => adapter.get_sequences(&schema).await.map(|items| {
                                AppMessage::SequencesLoaded {
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Types" => adapter.get_types(&schema).await.map(|items| {
                                AppMessage::TypesLoaded {
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Triggers" => adapter.get_triggers(&schema).await.map(|items| {
                                AppMessage::TriggersLoaded {
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Events" => adapter.get_events(&schema).await.map(|items| {
                                AppMessage::EventsLoaded {
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Packages" => adapter.get_packages(&schema).await.map(|items| {
                                AppMessage::PackagesLoaded {
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Procedures" => adapter.get_procedures(&schema).await.map(|items| {
                                AppMessage::ProceduresLoaded {
                                    schema: schema.clone(),
                                    items,
                                }
                            }),
                            "Functions" => adapter.get_functions(&schema).await.map(|items| {
                                AppMessage::FunctionsLoaded {
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
        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let schema = schema.to_string();
        let kind = kind.to_string();

        tokio::spawn(async move {
            let result = match kind.as_str() {
                "Tables" => adapter
                    .get_tables(&schema)
                    .await
                    .map(|items| AppMessage::TablesLoaded { schema, items }),
                "Views" => adapter
                    .get_views(&schema)
                    .await
                    .map(|items| AppMessage::ViewsLoaded { schema, items }),
                "Materialized Views" => adapter
                    .get_materialized_views(&schema)
                    .await
                    .map(|items| AppMessage::MaterializedViewsLoaded { schema, items }),
                "Indexes" => adapter
                    .get_indexes(&schema)
                    .await
                    .map(|items| AppMessage::IndexesLoaded { schema, items }),
                "Sequences" => adapter
                    .get_sequences(&schema)
                    .await
                    .map(|items| AppMessage::SequencesLoaded { schema, items }),
                "Types" => adapter
                    .get_types(&schema)
                    .await
                    .map(|items| AppMessage::TypesLoaded { schema, items }),
                "Triggers" => adapter
                    .get_triggers(&schema)
                    .await
                    .map(|items| AppMessage::TriggersLoaded { schema, items }),
                "Events" => adapter
                    .get_events(&schema)
                    .await
                    .map(|items| AppMessage::EventsLoaded { schema, items }),
                "Packages" => adapter
                    .get_packages(&schema)
                    .await
                    .map(|items| AppMessage::PackagesLoaded { schema, items }),
                "Procedures" => adapter
                    .get_procedures(&schema)
                    .await
                    .map(|items| AppMessage::ProceduresLoaded { schema, items }),
                "Functions" => adapter
                    .get_functions(&schema)
                    .await
                    .map(|items| AppMessage::FunctionsLoaded { schema, items }),
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
        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let s = schema.to_string();
        let t = table.to_string();

        tokio::spawn(async move {
            if let Ok(columns) = adapter.get_columns(&s, &t).await {
                let _ = tx.send(AppMessage::ColumnsCached { key, columns }).await;
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
        // Strip trailing semicolons — Oracle/MySQL/PG drivers don't accept them
        let query = query
            .trim_end()
            .trim_end_matches(';')
            .trim_end()
            .to_string();

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
        let db_type = self.state.db_type;

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
        let db_type = self.state.db_type;

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
        let config = self.state.connection_form.to_connection_config();
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
