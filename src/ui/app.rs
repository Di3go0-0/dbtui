use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::core::models::*;
use crate::core::validator::SqlValidator;
use crate::core::virtual_fs::{FileType, SyncState, VirtualFileSystem};
use crate::core::DatabaseAdapter;
use crate::ui::events::{self, Action};
use crate::ui::layout;
use crate::ui::state::{AppState, CategoryKind, LeafKind, Overlay, TreeNode};
use crate::ui::tabs::{SubView, TabId, TabKind};
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
        new_tab: bool,
    },
    QueryFailed {
        tab_id: TabId,
        error: String,
        query: String,
        new_tab: bool,
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
    ValidationResult {
        tab_id: TabId,
        report: crate::core::validator::ValidationReport,
    },
    CompileResult {
        tab_id: TabId,
        success: bool,
        message: String,
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
    /// Virtual file system per connection
    pub vfs: HashMap<String, VirtualFileSystem>,
    /// Cache directory base path
    pub cache_dir: Option<PathBuf>,
}

impl App {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(64);
        let cache_dir = crate::core::storage::CacheStore::base_cache_dir();
        Self {
            state: AppState::new(),
            theme: Theme::default(),
            adapters: HashMap::new(),
            msg_tx: tx,
            msg_rx: rx,
            vfs: HashMap::new(),
            cache_dir,
        }
    }

    /// Add a connected adapter and its tree node
    pub fn add_connection(&mut self, adapter: Arc<dyn DatabaseAdapter>, conn_name: &str) {
        self.adapters
            .insert(conn_name.to_string(), Arc::clone(&adapter));

        // Add connection node to tree expanded, then auto-load schemas
        self.state.tree.push(TreeNode::Connection {
            name: conn_name.to_string(),
            expanded: true,
            status: crate::ui::state::ConnStatus::Connected,
        });

        self.state.connected = true;
        self.state.connection_name = Some(conn_name.to_string());
        self.state.db_type = Some(adapter.db_type());

        // Auto-load schemas so the sidebar populates immediately
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

            // Check if leader key has been pending for >1s → show help popup
            self.check_leader_help_timeout();

            if let Some(input) = events::poll_event(Duration::from_millis(50)) {
                // Any key press hides the leader help popup
                if self.state.leader_help_visible {
                    self.state.leader_help_visible = false;
                }

                let action = match input {
                    events::InputEvent::Key(key) => events::handle_key(&mut self.state, key),
                    events::InputEvent::Paste(text) => {
                        self.handle_paste(&text);
                        events::Action::Render
                    }
                };
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
                    Action::LoadPackageContent { tab_id, schema, name } => {
                        self.state.loading = true;
                        self.state.status_message = format!("Loading {name}...");
                        self.spawn_load_package_content(tab_id, &schema, &name);
                    }
                    Action::ExecuteQuery { tab_id, query } => {
                        self.spawn_execute_query(tab_id, &query, false);
                    }
                    Action::ExecuteQueryNewTab { tab_id, query } => {
                        self.spawn_execute_query(tab_id, &query, true);
                    }
                    Action::LoadSourceCode { tab_id, schema, name, obj_type } => {
                        self.spawn_load_source_code(tab_id, &schema, &name, &obj_type);
                    }
                    Action::OpenNewScript => {
                        let script_num = self.state.tabs.iter()
                            .filter(|t| matches!(t.kind, TabKind::Script { .. }))
                            .count() + 1;
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
                        self.state.close_active_tab();
                    }
                    Action::ConfirmCloseNo => {
                        self.state.close_active_tab();
                    }
                    Action::OpenScript { name } => {
                        self.open_script(&name);
                    }
                    Action::DeleteScript { name } => {
                        self.delete_script(&name);
                    }
                    Action::DuplicateScript { name } => {
                        self.duplicate_script(&name);
                    }
                    Action::RenameScript { old_name, new_name } => {
                        self.rename_script(&old_name, &new_name);
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
                    Action::SaveSchemaFilter => {
                        self.save_object_filter();
                    }
                    Action::ValidateAndSave { tab_id } => {
                        self.handle_validate_and_save(tab_id);
                    }
                    Action::CompileToDb { tab_id } => {
                        self.handle_compile_to_db(tab_id);
                    }
                    Action::CloseResultTab => {
                        if let Some(tab) = self.state.active_tab_mut() {
                            if !tab.result_tabs.is_empty() {
                                let idx = tab.active_result_idx;
                                tab.result_tabs.remove(idx);
                                if tab.result_tabs.is_empty() {
                                    tab.active_result_idx = 0;
                                    tab.query_result = None;
                                    tab.grid_focused = false;
                                } else if idx >= tab.result_tabs.len() {
                                    tab.active_result_idx = tab.result_tabs.len() - 1;
                                }
                            }
                        }
                    }
                    Action::OpenScriptConnPicker => {
                        self.open_script_conn_picker();
                    }
                    Action::SetScriptConnection { conn_name } => {
                        self.set_script_connection(&conn_name);
                    }
                }
            }
        }
        Ok(())
    }

    fn handle_paste(&mut self, text: &str) {
        use crate::ui::state::Focus;
        use crate::ui::vim::VimMode;

        if self.state.focus != Focus::TabContent {
            return;
        }
        if let Some(tab) = self.state.active_tab_mut() {
            if let Some(editor) = tab.active_editor_mut() {
                if !matches!(editor.mode, VimMode::Insert) {
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
                // Get connection name before mutating state
                let conn_name = self.state.find_tab(tab_id).and_then(|t| match &t.kind {
                    TabKind::Package { conn_name, .. } => Some(conn_name.clone()),
                    _ => None,
                });

                if let Some(tab) = self.state.find_tab_mut(tab_id) {
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

                // Register in VFS
                if let Some(cn) = conn_name {
                    self.register_in_vfs(tab_id, &cn);
                }
                self.state.loading = false;

            }
            AppMessage::QueryExecuted { tab_id, result, new_tab } => {
                let row_count = result.rows.len();
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    let is_script = matches!(tab.kind, TabKind::Script { .. });
                    if is_script {
                        use crate::ui::tabs::ResultTab;
                        let label = format!("Result {}", tab.result_tabs.len() + 1);
                        let rt = ResultTab {
                            label,
                            result,
                            error_editor: None,
                            query_editor: None,
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
                            // Replace active result tab
                            let idx = tab.active_result_idx;
                            if idx < tab.result_tabs.len() {
                                tab.result_tabs[idx] = rt;
                            } else {
                                tab.result_tabs.push(rt);
                                tab.active_result_idx = tab.result_tabs.len() - 1;
                            }
                        }
                        tab.grid_focused = true;
                    } else {
                        // Table/view: single result, no tabs
                        tab.query_result = Some(result);
                        tab.grid_selected_row = 0;
                        tab.grid_scroll_row = 0;
                    }
                }
                self.state.status_message = format!("{row_count} rows returned");
                self.state.loading = false;
            }
            AppMessage::QueryFailed { tab_id, error, query, new_tab } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    let is_script = matches!(tab.kind, TabKind::Script { .. });
                    if is_script {
                        use crate::ui::tabs::ResultTab;
                        use crate::ui::vim::buffer::VimEditor;

                        // Error editor (left pane)
                        let wrap_width = 40;
                        let formatted = wrap_error_text(&error, wrap_width);
                        let mut err_editor = VimEditor::new(
                            &formatted,
                            crate::ui::vim::VimModeConfig::read_only(),
                        );
                        err_editor.mode = crate::ui::vim::VimMode::Normal;

                        // Query editor (right pane) — the SQL that failed
                        let mut q_editor = VimEditor::new(
                            &query,
                            crate::ui::vim::VimModeConfig::read_only(),
                        );
                        q_editor.mode = crate::ui::vim::VimMode::Normal;

                        let label = format!("Error {}", tab.result_tabs.len() + 1);
                        let rt = ResultTab {
                            label,
                            result: QueryResult { columns: vec![], rows: vec![] },
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
                        tab.grid_focused = true;
                    }
                }
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
                let conn_name = self.state.find_tab(tab_id).and_then(|t| match &t.kind {
                    TabKind::Function { conn_name, .. }
                    | TabKind::Procedure { conn_name, .. } => Some(conn_name.clone()),
                    _ => None,
                });

                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    if let Some(editor) = tab.editor.as_mut() {
                        editor.set_content(&source);
                    }
                }


                // Register in VFS
                if let Some(cn) = conn_name {
                    self.register_in_vfs(tab_id, &cn);
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
                self.state.loading = false;
            }
            AppMessage::CompileResult { tab_id, success, message } => {
                if success {
                    self.sync_tab_to_vfs_compiled(tab_id);
                    if let Some(tab) = self.state.find_tab_mut(tab_id) {
                        if let Some(editor) = tab.active_editor_mut() {
                            editor.modified = false;
                        }
                    }
                    self.state.status_message = "Compiled to database".to_string();
                } else {
                    self.sync_tab_to_vfs_error(tab_id, message.clone());
                    self.state.status_message = format!("Compilation failed: {message}");
                }
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

    #[allow(dead_code)]
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

    fn spawn_execute_query(&self, tab_id: TabId, query: &str, new_tab: bool) {
        // If the tab is a script with an assigned connection, use that adapter
        let adapter = self
            .state
            .find_tab(tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::Script { conn_name: Some(cn), .. } => self.adapter_for(cn),
                _ => None,
            })
            .or_else(|| self.active_adapter().map(|(_, a)| a));

        let adapter = match adapter {
            Some(a) => a,
            None => return,
        };
        let tx = self.msg_tx.clone();
        let query = query.to_string();

        tokio::spawn(async move {
            match adapter.execute(&query).await {
                Ok(result) => {
                    let _ = tx.send(AppMessage::QueryExecuted { tab_id, result, new_tab }).await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::QueryFailed { tab_id, error: e.to_string(), query: query.clone(), new_tab }).await;
                }
            }
        });
    }

    fn check_leader_help_timeout(&mut self) {
        if let Some(tab) = self.state.active_tab() {
            if let Some(editor) = tab.active_editor() {
                // Sub-menus (b, <leader>) appear immediately
                if editor.pending_leader_b || editor.pending_leader_leader {
                    self.state.leader_help_visible = true;
                    return;
                }
                // Root leader menu appears immediately
                if editor.pending_leader {
                    self.state.leader_help_visible = true;
                    return;
                }
            }
        }
        // No leader pending → hide
        if self.state.leader_help_visible {
            self.state.leader_help_visible = false;
        }
    }

    fn open_script_conn_picker(&mut self) {
        let connected: std::collections::HashSet<String> =
            self.adapters.keys().cloned().collect();

        let active: Vec<String> = connected.iter().cloned().collect();
        let others: Vec<String> = self
            .state
            .saved_connections
            .iter()
            .filter(|c| !connected.contains(&c.name))
            .map(|c| c.name.clone())
            .collect();

        if active.is_empty() && others.is_empty() {
            self.state.status_message = "No connections available".to_string();
            return;
        }

        // Pre-select current script connection if set
        let mut picker = crate::ui::state::ScriptConnPicker::new(active, others);
        if let Some(tab) = self.state.active_tab() {
            if let TabKind::Script { conn_name: Some(cn), .. } = &tab.kind {
                let items = picker.visible_items();
                if let Some(pos) = items.iter().position(|item| match item {
                    crate::ui::state::PickerItem::Active(n) => n == cn,
                    _ => false,
                }) {
                    picker.cursor = pos;
                }
            }
        }

        self.state.script_conn_picker = Some(picker);
        self.state.overlay = Some(Overlay::ScriptConnection);
    }

    fn set_script_connection(&mut self, conn_name: &str) {
        if !self.adapters.contains_key(conn_name) {
            self.connect_by_name(conn_name);
        }
        if let Some(tab) = self.state.active_tab_mut() {
            if let TabKind::Script { conn_name: ref mut cn, ref name, .. } = tab.kind {
                *cn = Some(conn_name.to_string());
                save_script_connection(name, conn_name);
            }
        }
        self.state.status_message = format!("Script → {conn_name}");
    }

    #[allow(dead_code)]
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
        self.refresh_scripts_list();
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
        if let Some(tab) = self.state.active_tab() {
            if let TabKind::Script { ref file_path, ref name, .. } = tab.kind {
                if file_path.is_none() {
                    // New script: prompt for name
                    self.state.scripts_save_name = Some(name.clone());
                    self.state.overlay = Some(Overlay::SaveScriptName);
                    return;
                }
            }
        }
        self.do_save_script(None);
    }

    fn do_save_script(&mut self, new_name: Option<&str>) {
        if let Some(tab) = self.state.active_tab_mut() {
            if let TabKind::Script { ref mut name, ref mut file_path, .. } = tab.kind {
                let save_name = new_name.unwrap_or(name);
                let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
                if let Ok(store) = crate::core::storage::ScriptStore::new() {
                    match store.save(save_name, &content) {
                        Ok(()) => {
                            if let Some(new) = new_name {
                                *name = new.to_string();
                            }
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
        self.refresh_scripts_list();
    }
    fn refresh_scripts_list(&mut self) {
        if let Ok(store) = crate::core::storage::ScriptStore::new() {
            if let Ok(scripts) = store.list() {
                self.state.scripts_list = scripts;
                if self.state.scripts_cursor >= self.state.scripts_list.len() && !self.state.scripts_list.is_empty() {
                    self.state.scripts_cursor = self.state.scripts_list.len() - 1;
                }
            }
        }
    }

    fn open_script(&mut self, name: &str) {
        if let Ok(store) = crate::core::storage::ScriptStore::new() {
            if let Ok(content) = store.read(&format!("{name}.sql")) {
                // Load saved connection for this script
                let saved_conn = load_script_connection(name);
                let tab_id = self.state.open_or_focus_tab(TabKind::Script {
                    file_path: Some(format!("{name}.sql")),
                    name: name.to_string(),
                    conn_name: saved_conn,
                });
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    if let Some(editor) = tab.editor.as_mut() {
                        editor.set_content(&content);
                    }
                }
                self.state.status_message = format!("Opened script '{name}'");
            } else {
                self.state.status_message = format!("Error reading script '{name}'");
            }
        }
    }

    fn delete_script(&mut self, name: &str) {
        if let Ok(store) = crate::core::storage::ScriptStore::new() {
            let _ = store.delete(name);
            self.state.status_message = format!("Script '{name}' deleted");
            self.refresh_scripts_list();
        }
    }

    fn duplicate_script(&mut self, name: &str) {
        if let Ok(store) = crate::core::storage::ScriptStore::new() {
            if let Ok(content) = store.read(name) {
                let base = name.strip_suffix(".sql").unwrap_or(name);
                let new_name = format!("{base}_copy");
                let _ = store.save(&new_name, &content);
                self.state.status_message = format!("Duplicated as '{new_name}'");
                self.refresh_scripts_list();
            }
        }
    }

    fn rename_script(&mut self, old_name: &str, new_name: &str) {
        if let Ok(store) = crate::core::storage::ScriptStore::new() {
            // Read old content, save with new name, delete old
            if let Ok(content) = store.read(old_name) {
                let _ = store.save(new_name, &content);
                let _ = store.delete(old_name);
                self.state.status_message = format!("Renamed to '{new_name}'");
                self.refresh_scripts_list();

                // Update any open tab with the old name
                for tab in &mut self.state.tabs {
                    if let TabKind::Script { ref mut name, ref mut file_path, .. } = tab.kind {
                        let old_base = old_name.strip_suffix(".sql").unwrap_or(old_name);
                        if name == old_base {
                            *name = new_name.to_string();
                            *file_path = Some(format!("{new_name}.sql"));
                        }
                    }
                }
            }
        }
    }

    // ─── VFS Methods ───

    /// Get or create VFS for a connection
    fn vfs_for(&mut self, conn_name: &str) -> &mut VirtualFileSystem {
        let cache_dir = self.cache_dir.as_ref().map(|d| d.join(conn_name));
        self.vfs
            .entry(conn_name.to_string())
            .or_insert_with(|| VirtualFileSystem::new(conn_name.to_string(), cache_dir))
    }

    /// Register content in VFS when package/source content is loaded from DB
    fn register_in_vfs(&mut self, tab_id: TabId, conn_name: &str) {
        let tab = match self.state.find_tab(tab_id) {
            Some(t) => t,
            None => return,
        };

        match &tab.kind {
            TabKind::Package { schema, name, .. } => {
                let decl = tab.decl_editor.as_ref().map(|e| e.content()).unwrap_or_default();
                let body = tab.body_editor.as_ref().map(|e| e.content()).unwrap_or_default();
                let schema = schema.clone();
                let name = name.clone();
                let conn = conn_name.to_string();

                let vfs = self.vfs_for(&conn);
                vfs.get_or_create(
                    FileType::PackageDeclaration { schema: schema.clone(), package: name.clone() },
                    decl,
                );
                vfs.get_or_create(
                    FileType::PackageBody { schema, package: name },
                    body,
                );
            }
            TabKind::Function { schema, name, .. } => {
                let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
                let schema = schema.clone();
                let name = name.clone();
                let conn = conn_name.to_string();

                let vfs = self.vfs_for(&conn);
                vfs.get_or_create(
                    FileType::Function { schema, name },
                    content,
                );
            }
            TabKind::Procedure { schema, name, .. } => {
                let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
                let schema = schema.clone();
                let name = name.clone();
                let conn = conn_name.to_string();

                let vfs = self.vfs_for(&conn);
                vfs.get_or_create(
                    FileType::Procedure { schema, name },
                    content,
                );
            }
            _ => {}
        }
    }

    /// Get VFS path for a tab
    fn vfs_path_for_tab(&self, tab_id: TabId) -> Option<(String, String)> {
        let tab = self.state.find_tab(tab_id)?;
        match &tab.kind {
            TabKind::Package { conn_name, schema, name } => {
                let sub = tab.active_sub_view.as_ref();
                let path = match sub {
                    Some(SubView::PackageBody) => {
                        VirtualFileSystem::path_for_package_body(schema, name)
                    }
                    _ => VirtualFileSystem::path_for_package_decl(schema, name),
                };
                Some((conn_name.clone(), path))
            }
            TabKind::Function { conn_name, schema, name } => {
                Some((conn_name.clone(), VirtualFileSystem::path_for_function(schema, name)))
            }
            TabKind::Procedure { conn_name, schema, name } => {
                Some((conn_name.clone(), VirtualFileSystem::path_for_procedure(schema, name)))
            }
            _ => None,
        }
    }

    /// Sync tab content to VFS and mark as locally saved
    fn sync_tab_to_vfs(&mut self, tab_id: TabId, mark_saved: bool) {
        let (conn_name, vfs_path) = match self.vfs_path_for_tab(tab_id) {
            Some(p) => p,
            None => return,
        };

        // Get current content from editor
        let content = if let Some(tab) = self.state.find_tab(tab_id) {
            tab.active_editor().map(|e| e.content()).unwrap_or_default()
        } else {
            return;
        };

        let vfs = self.vfs_for(&conn_name);
        if let Some(file) = vfs.get_mut(&vfs_path) {
            file.update_content(content);
            if mark_saved {
                file.mark_local_saved();
                // Save to disk cache
                if let Some(ref cache_path) = file.cache_path {
                    if let Some(parent) = cache_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(cache_path, &file.local_saved);
                }
            }
        }

        // For packages, also sync the paired file (Declaration <-> Body)
        if let Some(tab) = self.state.find_tab(tab_id) {
            if let TabKind::Package { schema, name, .. } = &tab.kind {
                let other_path = match tab.active_sub_view.as_ref() {
                    Some(SubView::PackageBody) => {
                        VirtualFileSystem::path_for_package_decl(schema, name)
                    }
                    _ => VirtualFileSystem::path_for_package_body(schema, name),
                };

                let other_content = match tab.active_sub_view.as_ref() {
                    Some(SubView::PackageBody) => {
                        tab.decl_editor.as_ref().map(|e| e.content()).unwrap_or_default()
                    }
                    _ => tab.body_editor.as_ref().map(|e| e.content()).unwrap_or_default(),
                };

                let conn_name = conn_name.clone();
                let vfs = self.vfs_for(&conn_name);
                if let Some(file) = vfs.get_mut(&other_path) {
                    file.update_content(other_content);
                    if mark_saved {
                        file.mark_local_saved();
                        if let Some(ref cache_path) = file.cache_path {
                            if let Some(parent) = cache_path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            let _ = std::fs::write(cache_path, &file.local_saved);
                        }
                    }
                }
            }
        }
        self.update_tab_sync_state(tab_id);
    }

    /// Mark VFS file as having a validation/compilation error
    fn sync_tab_to_vfs_error(&mut self, tab_id: TabId, error: String) {
        let (conn_name, vfs_path) = match self.vfs_path_for_tab(tab_id) {
            Some(p) => p,
            None => return,
        };
        let vfs = self.vfs_for(&conn_name);
        if let Some(file) = vfs.get_mut(&vfs_path) {
            file.mark_error(error);
        }
        self.update_tab_sync_state(tab_id);
    }

    /// Mark VFS file as successfully compiled to DB
    fn sync_tab_to_vfs_compiled(&mut self, tab_id: TabId) {
        let (conn_name, vfs_path) = match self.vfs_path_for_tab(tab_id) {
            Some(p) => p,
            None => return,
        };
        let vfs = self.vfs_for(&conn_name);
        if let Some(file) = vfs.get_mut(&vfs_path) {
            file.mark_compiled();
        }
        self.update_tab_sync_state(tab_id);
    }

    /// Copy VFS sync state to the tab for rendering
    fn update_tab_sync_state(&mut self, tab_id: TabId) {
        let sync = self.vfs_sync_state(tab_id).cloned();
        if let Some(tab) = self.state.find_tab_mut(tab_id) {
            tab.sync_state = sync;
        }
    }

    /// Get VFS sync state for a tab
    pub fn vfs_sync_state(&self, tab_id: TabId) -> Option<&SyncState> {
        let tab = self.state.find_tab(tab_id)?;
        let (conn_name, vfs_path) = match &tab.kind {
            TabKind::Package { conn_name, schema, name } => {
                let path = match tab.active_sub_view.as_ref() {
                    Some(SubView::PackageBody) => {
                        VirtualFileSystem::path_for_package_body(schema, name)
                    }
                    _ => VirtualFileSystem::path_for_package_decl(schema, name),
                };
                (conn_name, path)
            }
            TabKind::Function { conn_name, schema, name } => {
                (conn_name, VirtualFileSystem::path_for_function(schema, name))
            }
            TabKind::Procedure { conn_name, schema, name } => {
                (conn_name, VirtualFileSystem::path_for_procedure(schema, name))
            }
            _ => return None,
        };
        let vfs = self.vfs.get(conn_name.as_str())?;
        vfs.sync_state(&vfs_path)
    }

    /// Handle Ctrl+S: thorough validation + local save
    fn handle_validate_and_save(&mut self, tab_id: TabId) {
        let tab = match self.state.find_tab(tab_id) {
            Some(t) => t,
            None => return,
        };

        let (conn_name, schema, content, obj_type) = match &tab.kind {
            TabKind::Package { conn_name, schema, .. } => {
                let decl = tab.decl_editor.as_ref().map(|e| e.content()).unwrap_or_default();
                let body = tab.body_editor.as_ref().map(|e| e.content()).unwrap_or_default();
                let content = format!("{}\n{}", decl, body);
                (conn_name.clone(), schema.clone(), content, "PACKAGE".to_string())
            }
            TabKind::Function { conn_name, schema, .. } => {
                let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
                (conn_name.clone(), schema.clone(), content, "FUNCTION".to_string())
            }
            TabKind::Procedure { conn_name, schema, .. } => {
                let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
                (conn_name.clone(), schema.clone(), content, "PROCEDURE".to_string())
            }
            _ => return,
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
        self.state.status_message = format!("Validating {obj_type}...");
        self.state.loading = true;

        tokio::spawn(async move {
            let validator = SqlValidator::new(db_type);
            let report = validator.validate_thorough(&schema, &content, &adapter).await;
            let _ = tx.send(AppMessage::ValidationResult { tab_id, report }).await;
        });
    }

    /// Handle <leader><leader>s: quick syntax + compile to DB
    fn handle_compile_to_db(&mut self, tab_id: TabId) {
        let tab = match self.state.find_tab(tab_id) {
            Some(t) => t,
            None => return,
        };

        let (conn_name, _schema, sql_statements) = match &tab.kind {
            TabKind::Package { conn_name, schema, .. } => {
                let decl = tab.decl_editor.as_ref().map(|e| e.content()).unwrap_or_default();
                let body = tab.body_editor.as_ref().map(|e| e.content()).unwrap_or_default();
                let mut stmts = Vec::new();
                if !decl.trim().is_empty() {
                    stmts.push(decl);
                }
                if !body.trim().is_empty() {
                    stmts.push(body);
                }
                (conn_name.clone(), schema.clone(), stmts)
            }
            TabKind::Function { conn_name, schema, .. } => {
                let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
                (conn_name.clone(), schema.clone(), vec![content])
            }
            TabKind::Procedure { conn_name, schema, .. } => {
                let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
                (conn_name.clone(), schema.clone(), vec![content])
            }
            _ => return,
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

        // First save locally, then compile
        self.sync_tab_to_vfs(tab_id, true);

        tokio::spawn(async move {
            let validator = SqlValidator::new(db_type);

            // Quick syntax check first
            for sql in &sql_statements {
                let syntax = validator.validate_syntax(sql);
                if !syntax.is_valid {
                    let _ = tx.send(AppMessage::CompileResult {
                        tab_id,
                        success: false,
                        message: syntax.error_summary(),
                    }).await;
                    return;
                }
            }

            // Compile each statement
            for sql in &sql_statements {
                if let Err(e) = validator.compile_to_db(sql, &adapter).await {
                    let _ = tx.send(AppMessage::CompileResult {
                        tab_id,
                        success: false,
                        message: e.to_string(),
                    }).await;
                    return;
                }
            }

            let _ = tx.send(AppMessage::CompileResult {
                tab_id,
                success: true,
                message: "OK".to_string(),
            }).await;
        });
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

/// Load the saved connection name for a script
fn load_script_connection(script_name: &str) -> Option<String> {
    let dir = crate::core::storage::ConnectionStore::new().ok()?;
    let path = dir.dir_path().join("script_connections.json");
    let data = std::fs::read_to_string(&path).ok()?;
    let map: std::collections::HashMap<String, String> = serde_json::from_str(&data).ok()?;
    map.get(script_name).cloned()
}

/// Save the connection name for a script
fn save_script_connection(script_name: &str, conn_name: &str) {
    let dir = match crate::core::storage::ConnectionStore::new() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = dir.dir_path().join("script_connections.json");

    let mut map: std::collections::HashMap<String, String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default();

    map.insert(script_name.to_string(), conn_name.to_string());

    if let Ok(json) = serde_json::to_string_pretty(&map) {
        let _ = std::fs::write(&path, json);
    }
}

/// Word-wrap error text to fit within `max_width` columns.
fn wrap_error_text(error: &str, max_width: usize) -> String {
    let mut lines = Vec::new();
    lines.push("-- Query Error --".to_string());
    lines.push(String::new());

    // Split on ": " to break long error chains into sections
    for section in error.split(": ") {
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
