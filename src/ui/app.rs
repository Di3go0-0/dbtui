use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::core::models::*;
use crate::core::DatabaseAdapter;
use crate::ui::events::{self, Action};
use crate::ui::layout;
use crate::ui::state::{AppState, CategoryKind, LeafKind, TreeNode};
use crate::ui::tabs::{TabId, TabKind, WorkspaceTab};
use crate::ui::theme::Theme;

pub enum AppMessage {
    SchemasLoaded {
        conn_name: String,
        schemas: Vec<Schema>,
    },
    TablesLoaded {
        schema: String,
        items: Vec<Table>,
    },
    ViewsLoaded {
        schema: String,
        items: Vec<View>,
    },
    PackagesLoaded {
        schema: String,
        items: Vec<Package>,
    },
    ProceduresLoaded {
        schema: String,
        items: Vec<Procedure>,
    },
    FunctionsLoaded {
        schema: String,
        items: Vec<Function>,
    },
    TableDataLoaded {
        tab_id: TabId,
        result: QueryResult,
    },
    ColumnsLoaded {
        tab_id: TabId,
        columns: Vec<Column>,
    },
    PackageContentLoaded {
        tab_id: TabId,
        content: PackageContent,
    },
    QueryExecuted {
        tab_id: TabId,
        result: QueryResult,
    },
    TableDDLLoaded {
        tab_id: TabId,
        ddl: String,
    },
    SourceCodeLoaded {
        tab_id: TabId,
        source: String,
    },
    Connected {
        adapter: Arc<dyn DatabaseAdapter>,
        name: String,
    },
    Error(String),
}

pub struct App {
    pub state: AppState,
    pub theme: Theme,
    /// Multiple adapters, keyed by connection name
    pub adapters: HashMap<String, Arc<dyn DatabaseAdapter>>,
    pub msg_tx: mpsc::Sender<AppMessage>,
    pub msg_rx: mpsc::Receiver<AppMessage>,
}

impl App {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(64);
        Self {
            state: AppState::new(),
            theme: Theme::default(),
            adapters: HashMap::new(),
            msg_tx: tx,
            msg_rx: rx,
        }
    }

    /// Add a connected adapter and its tree node
    pub fn add_connection(&mut self, adapter: Arc<dyn DatabaseAdapter>, conn_name: &str) {
        self.adapters
            .insert(conn_name.to_string(), Arc::clone(&adapter));

        // Add connection node to tree (don't replace existing)
        self.state.tree.push(TreeNode::Connection {
            name: conn_name.to_string(),
            expanded: false,
            status: crate::ui::state::ConnStatus::Connected,
        });

        self.state.connected = true;
        self.state.connection_name = Some(conn_name.to_string());
        self.state.db_type = Some(adapter.db_type());
    }

    /// Get the adapter for a connection name
    fn adapter_for(&self, conn_name: &str) -> Option<Arc<dyn DatabaseAdapter>> {
        self.adapters.get(conn_name).cloned()
    }

    /// Get the adapter for the currently active connection (from tree selection)
    fn active_adapter(&self) -> Option<(String, Arc<dyn DatabaseAdapter>)> {
        // Walk up from selected node to find its Connection parent
        let selected = self.state.selected_tree_index()?;
        let mut idx = selected;
        loop {
            match &self.state.tree[idx] {
                TreeNode::Connection { name, .. } => {
                    let adapter = self.adapters.get(name)?;
                    return Some((name.clone(), Arc::clone(adapter)));
                }
                _ => {
                    if idx == 0 {
                        break;
                    }
                    idx -= 1;
                }
            }
        }
        // Fallback: first adapter
        self.adapters
            .iter()
            .next()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
    }

    pub async fn run(
        &mut self,
        terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> crate::core::error::AppResult<()> {
        loop {
            terminal.draw(|frame| {
                layout::render(frame, &mut self.state, &self.theme);
            })?;

            while let Ok(msg) = self.msg_rx.try_recv() {
                self.handle_message(msg);
            }

            if let Some(key) = events::poll_event(Duration::from_millis(50)) {
                let action = events::handle_key(&mut self.state, key);
                match action {
                    Action::Quit => break,
                    Action::Render | Action::None => {}
                    Action::LoadSchemas { conn_name } => {
                        self.spawn_load_schemas(&conn_name);
                    }
                    Action::LoadChildren { schema, kind } => {
                        self.spawn_load_children(&schema, &kind);
                    }
                    Action::LoadTableData { tab_id, schema, table } => {
                        self.spawn_load_table_data(tab_id, &schema, &table);
                    }
                    Action::LoadColumns { tab_id, schema, table } => {
                        self.spawn_load_columns(tab_id, &schema, &table);
                    }
                    Action::LoadPackageContent { tab_id, schema, name } => {
                        self.spawn_load_package_content(tab_id, &schema, &name);
                    }
                    Action::ExecuteQuery { tab_id, query } => {
                        self.spawn_execute_query(tab_id, &query);
                    }
                    Action::LoadSourceCode { tab_id, schema, name, obj_type } => {
                        self.spawn_load_source_code(tab_id, &schema, &name, &obj_type);
                    }
                    Action::LoadTableDDL { tab_id, schema, table } => {
                        self.spawn_load_table_ddl(tab_id, &schema, &table);
                    }
                    Action::OpenNewScript => {
                        let script_num = self.state.tabs.iter()
                            .filter(|t| matches!(t.kind, TabKind::Script { .. }))
                            .count() + 1;
                        let name = format!("Script {script_num}");
                        self.state.open_or_focus_tab(TabKind::Script {
                            file_path: None,
                            name,
                        });
                    }
                    Action::CloseTab => {
                        self.handle_close_tab();
                    }
                    Action::SaveScript => {
                        self.save_active_script();
                    }
                    Action::ConfirmCloseYes => {
                        self.save_active_script();
                        self.state.close_active_tab();
                    }
                    Action::ConfirmCloseNo => {
                        self.state.close_active_tab();
                    }
                    Action::Connect => {
                        self.spawn_connect();
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
                    Action::EditConnection { name } => {
                        let _ = name;
                    }
                    Action::SaveSchemaFilter => {
                        self.save_object_filter();
                    }
                }
            }
        }
        Ok(())
    }

    fn handle_message(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::SchemasLoaded {
                conn_name,
                schemas,
            } => {
                let conn_idx = self.state.tree.iter().position(|n| {
                    matches!(n, TreeNode::Connection { name, .. } if name == &conn_name)
                });
                if let Some(idx) = conn_idx {
                    let next_conn = self.state.tree[idx + 1..]
                        .iter()
                        .position(|n| matches!(n, TreeNode::Connection { .. }))
                        .map(|p| idx + 1 + p)
                        .unwrap_or(self.state.tree.len());
                    self.state.tree.drain(idx + 1..next_conn);

                    let insert_pos = idx + 1;
                    for (i, schema) in schemas.into_iter().enumerate() {
                        self.state.tree.insert(
                            insert_pos + i,
                            TreeNode::Schema {
                                name: schema.name,
                                expanded: false,
                            },
                        );
                    }
                }
                self.state.status_message =
                    format!("Schemas loaded for {conn_name} - F to filter");
                self.state.loading = false;
            }
            AppMessage::TablesLoaded { schema, items } => {
                self.insert_leaves(&schema, CategoryKind::Tables, items, LeafKind::Table);
                self.state.loading = false;
            }
            AppMessage::ViewsLoaded { schema, items } => {
                self.insert_leaves(&schema, CategoryKind::Views, items, LeafKind::View);
                self.state.loading = false;
            }
            AppMessage::PackagesLoaded { schema, items } => {
                self.insert_package_leaves(&schema, items);
                self.state.loading = false;
            }
            AppMessage::ProceduresLoaded { schema, items } => {
                self.insert_leaves(
                    &schema,
                    CategoryKind::Procedures,
                    items,
                    LeafKind::Procedure,
                );
                self.state.loading = false;
            }
            AppMessage::FunctionsLoaded { schema, items } => {
                self.insert_leaves(
                    &schema,
                    CategoryKind::Functions,
                    items,
                    LeafKind::Function,
                );
                self.state.loading = false;
            }
            AppMessage::TableDataLoaded { tab_id, result } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.query_result = Some(result);
                    tab.grid_selected_row = 0;
                    tab.grid_scroll_row = 0;
                }
                self.state.status_message = "Data loaded".to_string();
                self.state.loading = false;
            }
            AppMessage::ColumnsLoaded { tab_id, columns } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.columns = columns;
                }
                self.state.loading = false;
            }
            AppMessage::PackageContentLoaded { tab_id, content } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    // Extract function and procedure names from declaration
                    tab.package_functions = extract_names(&content.declaration, "FUNCTION");
                    tab.package_procedures = extract_names(&content.declaration, "PROCEDURE");
                    tab.package_list_cursor = 0;

                    if let Some(editor) = tab.decl_editor.as_mut() {
                        editor.set_content(&content.declaration);
                    }
                    if let Some(editor) = tab.body_editor.as_mut() {
                        editor.set_content(content.body.as_deref().unwrap_or(""));
                    }
                    tab.package_content = Some(content);
                }
                self.state.loading = false;
            }
            AppMessage::QueryExecuted { tab_id, result } => {
                let row_count = result.rows.len();
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.query_result = Some(result);
                    tab.grid_selected_row = 0;
                    tab.grid_scroll_row = 0;
                }
                self.state.status_message = format!("{row_count} rows returned");
                self.state.loading = false;
            }
            AppMessage::TableDDLLoaded { tab_id, ddl } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    if let Some(editor) = tab.ddl_editor.as_mut() {
                        editor.set_content(&ddl);
                    }
                }
                self.state.loading = false;
            }
            AppMessage::SourceCodeLoaded { tab_id, source } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    if let Some(editor) = tab.editor.as_mut() {
                        editor.set_content(&source);
                    }
                }
                self.state.loading = false;
            }
            AppMessage::Connected { adapter, name } => {
                if self.state.overlay.is_some() {
                    let config = self.state.connection_form.to_connection_config();
                    self.save_connection_config(&config);
                    self.state.overlay = None;
                    self.state.connection_form.connecting = false;
                }

                self.set_conn_status(&name, crate::ui::state::ConnStatus::Connected);

                let already_in_tree = self.state.tree.iter().any(|n| {
                    matches!(n, TreeNode::Connection { name: n, .. } if n == &name)
                });

                if already_in_tree {
                    self.adapters
                        .insert(name.clone(), Arc::clone(&adapter));
                    self.state.connected = true;
                    self.state.connection_name = Some(name.clone());
                    self.state.db_type = Some(adapter.db_type());

                    let tx = self.msg_tx.clone();
                    let conn_name = name.clone();
                    tokio::spawn(async move {
                        match adapter.get_schemas().await {
                            Ok(schemas) => {
                                let _ = tx
                                    .send(AppMessage::SchemasLoaded {
                                        conn_name,
                                        schemas,
                                    })
                                    .await;
                            }
                            Err(e) => {
                                let _ =
                                    tx.send(AppMessage::Error(e.to_string())).await;
                            }
                        }
                    });
                } else {
                    self.add_connection(adapter, &name);
                }

                self.state.status_message = format!("Connected to {name}");
                self.state.loading = false;
            }
            AppMessage::Error(msg) => {
                if matches!(
                    self.state.overlay,
                    Some(crate::ui::state::Overlay::ConnectionDialog)
                ) {
                    self.state.connection_form.error_message = msg.clone();
                    self.state.connection_form.connecting = false;

                    let config = self.state.connection_form.to_connection_config();
                    if !config.name.is_empty() {
                        self.save_connection_config(&config);
                        let exists = self.state.tree.iter().any(|n| {
                            matches!(n, TreeNode::Connection { name, .. } if name == &config.name)
                        });
                        if !exists {
                            self.state.tree.push(TreeNode::Connection {
                                name: config.name.clone(),
                                expanded: false,
                                status: crate::ui::state::ConnStatus::Failed,
                            });
                        } else {
                            self.set_conn_status(
                                &config.name,
                                crate::ui::state::ConnStatus::Failed,
                            );
                        }
                    }
                }
                for node in &mut self.state.tree {
                    if let TreeNode::Connection { status, .. } = node {
                        if *status == crate::ui::state::ConnStatus::Connecting {
                            *status = crate::ui::state::ConnStatus::Failed;
                        }
                    }
                }
                self.state.status_message = format!("Error: {msg}");
                self.state.loading = false;
            }
        }
    }

    fn insert_leaves<T: HasName>(
        &mut self,
        schema: &str,
        category: CategoryKind,
        items: Vec<T>,
        leaf_kind: LeafKind,
    ) {
        let cat_idx = self.state.tree.iter().position(|n| {
            matches!(n, TreeNode::Category { schema: s, kind, .. } if s == schema && *kind == category)
        });
        if let Some(idx) = cat_idx {
            self.remove_children_of(idx);

            let insert_pos = idx + 1;
            for (i, item) in items.iter().enumerate() {
                self.state.tree.insert(
                    insert_pos + i,
                    TreeNode::Leaf {
                        name: item.get_name(),
                        schema: schema.to_string(),
                        kind: leaf_kind.clone(),
                        valid: item.is_valid(),
                    },
                );
            }
        }
    }

    fn insert_package_leaves(&mut self, schema: &str, items: Vec<Package>) {
        let cat_idx = self.state.tree.iter().position(|n| {
            matches!(n, TreeNode::Category { schema: s, kind: CategoryKind::Packages, .. } if s == schema)
        });
        if let Some(idx) = cat_idx {
            self.remove_children_of(idx);

            let insert_pos = idx + 1;
            for (i, pkg) in items.into_iter().enumerate() {
                self.state.tree.insert(
                    insert_pos + i,
                    TreeNode::Leaf {
                        name: pkg.name.clone(),
                        schema: schema.to_string(),
                        kind: LeafKind::Package,
                        valid: pkg.valid,
                    },
                );
            }
        }
    }

    fn remove_children_of(&mut self, parent_idx: usize) {
        let parent_depth = self.state.tree[parent_idx].depth();
        let start = parent_idx + 1;
        let mut end = start;
        while end < self.state.tree.len() && self.state.tree[end].depth() > parent_depth {
            end += 1;
        }
        if end > start {
            self.state.tree.drain(start..end);
        }
    }

    fn spawn_load_schemas(&mut self, conn_name: &str) {
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

            tokio::spawn(async move {
                match crate::drivers::create_adapter(&config).await {
                    Ok(adapter) => {
                        let adapter: Arc<dyn crate::core::DatabaseAdapter> = adapter.into();
                        let _ = tx
                            .send(AppMessage::Connected {
                                adapter,
                                name,
                            })
                            .await;
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

    fn spawn_load_children(&self, schema: &str, kind: &str) {
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

    fn spawn_load_table_data(&self, tab_id: TabId, schema: &str, table: &str) {
        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let query = format!("SELECT * FROM {schema}.{table}");
        let schema_owned = schema.to_string();
        let table_owned = table.to_string();

        tokio::spawn(async move {
            let (data_result, cols_result) = tokio::join!(
                adapter.execute(&query),
                adapter.get_columns(&schema_owned, &table_owned)
            );
            match data_result {
                Ok(result) => {
                    let _ = tx.send(AppMessage::TableDataLoaded { tab_id, result }).await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(e.to_string())).await;
                }
            }
            match cols_result {
                Ok(columns) => {
                    let _ = tx.send(AppMessage::ColumnsLoaded { tab_id, columns }).await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(e.to_string())).await;
                }
            }
        });
    }

    fn spawn_load_columns(&self, tab_id: TabId, schema: &str, table: &str) {
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

    fn spawn_load_package_content(&self, tab_id: TabId, schema: &str, name: &str) {
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
                    let _ = tx.send(AppMessage::PackageContentLoaded { tab_id, content }).await;
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

    fn spawn_execute_query(&self, tab_id: TabId, query: &str) {
        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let query = query.to_string();

        tokio::spawn(async move {
            match adapter.execute(&query).await {
                Ok(result) => {
                    let _ = tx.send(AppMessage::QueryExecuted { tab_id, result }).await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(e.to_string())).await;
                }
            }
        });
    }

    fn spawn_load_table_ddl(&self, tab_id: TabId, schema: &str, table: &str) {
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

    fn spawn_load_source_code(&self, tab_id: TabId, schema: &str, name: &str, obj_type: &str) {
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
                    let _ = tx.send(AppMessage::SourceCodeLoaded { tab_id, source }).await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(e.to_string())).await;
                }
            }
        });
    }

    fn spawn_connect(&mut self) {
        let config = self.state.connection_form.to_connection_config();
        let tx = self.msg_tx.clone();
        let conn_name = config.name.clone();

        self.state.status_message = format!("Connecting to {}...", conn_name);
        self.state.loading = true;

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

    fn set_conn_status(&mut self, conn_name: &str, status: crate::ui::state::ConnStatus) {
        for node in &mut self.state.tree {
            if let TreeNode::Connection { name, status: s, .. } = node {
                if name == conn_name {
                    *s = status;
                    break;
                }
            }
        }
    }

    fn connect_by_name(&mut self, name: &str) {
        self.adapters.remove(name);
        self.set_conn_status(name, crate::ui::state::ConnStatus::Connecting);

        let config = self
            .state
            .saved_connections
            .iter()
            .find(|c| c.name == name)
            .cloned();

        if let Some(config) = config {
            let tx = self.msg_tx.clone();
            let conn_name = name.to_string();
            self.state.status_message = format!("Connecting to {conn_name}...");
            self.state.loading = true;

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
        } else {
            self.state.status_message = format!("No saved config for '{name}'");
        }
    }

    fn disconnect_by_name(&mut self, name: &str) {
        self.adapters.remove(name);
        self.set_conn_status(name, crate::ui::state::ConnStatus::Disconnected);

        if let Some(conn_idx) = self.state.tree.iter().position(|n| {
            matches!(n, TreeNode::Connection { name: n, .. } if n == name)
        }) {
            if let TreeNode::Connection { expanded, .. } = &mut self.state.tree[conn_idx] {
                *expanded = false;
            }
            let next_conn = self.state.tree[conn_idx + 1..]
                .iter()
                .position(|n| matches!(n, TreeNode::Connection { .. }))
                .map(|p| conn_idx + 1 + p)
                .unwrap_or(self.state.tree.len());
            self.state.tree.drain(conn_idx + 1..next_conn);
        }

        if self.state.connection_name.as_deref() == Some(name) {
            self.state.connected = false;
            self.state.connection_name = None;
        }

        self.state.status_message = format!("Disconnected from '{name}'");
    }

    fn delete_connection(&mut self, name: &str) {
        self.adapters.remove(name);

        if let Some(conn_idx) = self.state.tree.iter().position(|n| {
            matches!(n, TreeNode::Connection { name: n, .. } if n == name)
        }) {
            let next_conn = self.state.tree[conn_idx + 1..]
                .iter()
                .position(|n| matches!(n, TreeNode::Connection { .. }))
                .map(|p| conn_idx + 1 + p)
                .unwrap_or(self.state.tree.len());
            self.state.tree.drain(conn_idx..next_conn);
        }

        self.state.saved_connections.retain(|c| c.name != name);
        self.persist_connections();

        if self.adapters.is_empty() {
            self.state.connected = false;
            self.state.connection_name = None;
            self.state.db_type = None;
        }

        self.state.tree_state.cursor = 0;
        self.state.tree_state.offset = 0;
        self.state.status_message = format!("Connection '{name}' deleted");
    }

    fn save_connection_config(&mut self, config: &ConnectionConfig) {
        if let Some(old_name) = self.state.connection_form.editing_name.take() {
            self.state.saved_connections.retain(|c| c.name != old_name);
            if old_name != config.name {
                if let Some(adapter) = self.adapters.remove(&old_name) {
                    self.adapters.insert(config.name.clone(), adapter);
                }
                for node in &mut self.state.tree {
                    if let TreeNode::Connection { name, .. } = node {
                        if *name == old_name {
                            *name = config.name.clone();
                        }
                    }
                }
                if self.state.connection_name.as_deref() == Some(&old_name) {
                    self.state.connection_name = Some(config.name.clone());
                }
            }
        }

        self.state.saved_connections.retain(|c| c.name != config.name);
        self.state.saved_connections.push(config.clone());
        self.persist_connections();
    }

    fn persist_connections(&self) {
        if let Ok(store) = crate::core::storage::ConnectionStore::new() {
            let _ = store.save(&self.state.saved_connections, "");
        }
    }

    fn save_current_connection(&mut self) {
        let config = self.state.connection_form.to_connection_config();
        if config.name.is_empty() {
            self.state.connection_form.error_message = "Name is required to save".to_string();
            return;
        }
        self.save_connection_config(&config);
        self.state.status_message = format!("Connection '{}' saved", config.name);
    }

    pub fn load_saved_connections(&mut self) {
        if let Ok(store) = crate::core::storage::ConnectionStore::new() {
            if let Ok(configs) = store.load("") {
                self.state.saved_connections = configs.clone();
                for config in &configs {
                    self.state.tree.push(TreeNode::Connection {
                        name: config.name.clone(),
                        expanded: false,
                        status: crate::ui::state::ConnStatus::Disconnected,
                    });
                }
                if !configs.is_empty() {
                    self.state.status_message = format!(
                        "{} connection(s) loaded - expand to connect",
                        configs.len()
                    );
                }
            }
        }
        self.load_object_filter();
    }

    fn load_object_filter(&mut self) {
        if let Ok(dir) = crate::core::storage::ConnectionStore::new() {
            let filter_path = dir.dir_path().join("object_filters.json");
            if let Ok(data) = std::fs::read_to_string(&filter_path) {
                if let Ok(filters) =
                    serde_json::from_str::<HashMap<String, Vec<String>>>(&data)
                {
                    for (key, names) in filters {
                        let set: HashSet<String> = names.into_iter().collect();
                        if !set.is_empty() {
                            self.state.object_filter.filters.insert(key, set);
                        }
                    }
                }
            }
        }
    }

    pub fn save_object_filter(&mut self) {
        if let Ok(dir) = crate::core::storage::ConnectionStore::new() {
            let filter_path = dir.dir_path().join("object_filters.json");
            let serializable: HashMap<&String, Vec<&String>> = self
                .state
                .object_filter
                .filters
                .iter()
                .filter(|(_, set)| !set.is_empty())
                .map(|(k, set)| (k, set.iter().collect()))
                .collect();
            match std::fs::write(
                &filter_path,
                serde_json::to_string_pretty(&serializable).unwrap_or_default(),
            ) {
                Ok(()) => {
                    let total: usize = self.state.object_filter.filters.values().map(|s| s.len()).sum();
                    if total > 0 {
                        self.state.status_message = format!("Filters saved ({total} rules)");
                    }
                }
                Err(e) => {
                    self.state.status_message = format!("Error saving filter: {e}");
                }
            }
        }
    }

    fn handle_close_tab(&mut self) {
        // Check if active tab has a modified editor (script)
        let is_modified = if let Some(tab) = self.state.active_tab() {
            match &tab.kind {
                TabKind::Script { .. } => {
                    tab.editor.as_ref().map(|e| e.modified).unwrap_or(false)
                }
                _ => false, // Non-script tabs close without confirmation
            }
        } else {
            false
        };

        if is_modified {
            self.state.overlay = Some(crate::ui::state::Overlay::ConfirmClose);
        } else {
            self.state.close_active_tab();
        }
    }

    fn save_active_script(&mut self) {
        if let Some(tab) = self.state.active_tab_mut() {
            if let TabKind::Script { ref name, ref mut file_path } = tab.kind {
                let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
                if let Ok(store) = crate::core::storage::ScriptStore::new() {
                    match store.save(name, &content) {
                        Ok(()) => {
                            *file_path = Some(format!("{}.sql", name));
                            if let Some(editor) = tab.editor.as_mut() {
                                editor.modified = false;
                            }
                            self.state.status_message = format!("Script '{}' saved", name);
                        }
                        Err(e) => {
                            self.state.status_message = format!("Error saving script: {e}");
                        }
                    }
                }
            }
        }
    }
}

trait HasName {
    fn get_name(&self) -> String;
    fn is_valid(&self) -> bool;
}
impl HasName for Table {
    fn get_name(&self) -> String { self.name.clone() }
    fn is_valid(&self) -> bool { true }
}
impl HasName for View {
    fn get_name(&self) -> String { self.name.clone() }
    fn is_valid(&self) -> bool { self.valid }
}
impl HasName for Procedure {
    fn get_name(&self) -> String { self.name.clone() }
    fn is_valid(&self) -> bool { self.valid }
}
impl HasName for Function {
    fn get_name(&self) -> String { self.name.clone() }
    fn is_valid(&self) -> bool { self.valid }
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
        if let Some(rest_upper) = trimmed_upper.strip_prefix(&kind_upper) {
            if rest_upper.starts_with(|c: char| c.is_whitespace()) {
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
    }
    names
}
