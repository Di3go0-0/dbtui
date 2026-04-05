use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::core::DatabaseAdapter;
use crate::core::models::*;
use crate::core::validator::SqlValidator;
use crate::core::virtual_fs::{FileType, SyncState, VirtualFileSystem};
use crate::ui::events::{self, Action};
use crate::ui::layout;
use crate::ui::state::{AppState, CategoryKind, LeafKind, Overlay, TreeNode};
use crate::ui::tabs::{SubView, TabId, TabKind, WorkspaceTab};
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
    MaterializedViewsLoaded {
        schema: String,
        items: Vec<MaterializedView>,
    },
    IndexesLoaded {
        schema: String,
        items: Vec<Index>,
    },
    SequencesLoaded {
        schema: String,
        items: Vec<Sequence>,
    },
    TypesLoaded {
        schema: String,
        items: Vec<DbType>,
    },
    TriggersLoaded {
        schema: String,
        items: Vec<Trigger>,
    },
    EventsLoaded {
        schema: String,
        items: Vec<DbEvent>,
    },
    TableDataLoaded {
        tab_id: TabId,
        result: QueryResult,
    },
    TableDataBatch {
        tab_id: TabId,
        rows: Vec<Vec<String>>,
        done: bool,
    },
    ColumnsLoaded {
        tab_id: TabId,
        columns: Vec<Column>,
    },
    PackageContentLoaded {
        tab_id: TabId,
        content: PackageContent,
    },
    QueryBatch {
        tab_id: TabId,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        done: bool,
        new_tab: bool,
        elapsed: Option<std::time::Duration>,
    },
    QueryFailed {
        tab_id: TabId,
        error: String,
        query: String,
        new_tab: bool,
        start_line: usize,
    },
    TableDDLLoaded {
        tab_id: TabId,
        ddl: String,
    },
    TypeInfoLoaded {
        tab_id: TabId,
        attributes: QueryResult,
        methods: QueryResult,
        declaration: String,
        body: String,
    },
    TriggerInfoLoaded {
        tab_id: TabId,
        columns: QueryResult,
        declaration: String,
    },
    GridChangesSaved {
        tab_id: TabId,
        count: usize,
    },
    GridChangesError {
        tab_id: TabId,
        error_text: String,
        sql_text: String,
    },
    SourceCodeLoaded {
        tab_id: TabId,
        source: String,
    },
    ObjectDropped {
        schema: String,
        name: String,
        obj_type: String,
    },
    ObjectRenamed {
        schema: String,
        old_name: String,
        new_name: String,
        obj_type: String,
    },
    ObjectError {
        error: String,
        sql: String,
    },
    DdlExecuted {
        query: String,
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
        failed_sql: String,
        /// "DECLARATION", "BODY", "SOURCE" — which part failed
        failed_part: String,
    },
    ColumnsCached {
        key: String,
        columns: Vec<Column>,
    },
    Error(String),
}

/// Extract source code info (conn_name, schema, content, obj_type) from a source tab.
/// Returns None for non-source tabs (scripts, tables).
fn extract_source_info(tab: &WorkspaceTab) -> Option<(String, String, String, String)> {
    match &tab.kind {
        TabKind::Package {
            conn_name, schema, ..
        } => {
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
            let content = format!("{}\n{}", decl, body);
            Some((
                conn_name.clone(),
                schema.clone(),
                content,
                "PACKAGE".to_string(),
            ))
        }
        TabKind::Function {
            conn_name, schema, ..
        } => {
            let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
            Some((
                conn_name.clone(),
                schema.clone(),
                content,
                "FUNCTION".to_string(),
            ))
        }
        TabKind::Procedure {
            conn_name, schema, ..
        } => {
            let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
            Some((
                conn_name.clone(),
                schema.clone(),
                content,
                "PROCEDURE".to_string(),
            ))
        }
        _ => None,
    }
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

        // Determine which group this connection belongs to
        let group = self
            .state
            .saved_connections
            .iter()
            .find(|c| c.name == conn_name)
            .map(|c| c.group.clone())
            .unwrap_or_else(|| "Default".to_string());

        // Find the group node or create it, then insert connection after group's last child
        let insert_idx = self.find_or_create_group_insert_idx(&group);
        self.state.tree.insert(
            insert_idx,
            TreeNode::Connection {
                name: conn_name.to_string(),
                expanded: true,
                status: crate::ui::state::ConnStatus::Connected,
            },
        );

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

            // Set cursor shape based on editor mode (beam for Insert, block otherwise)
            {
                use crossterm::cursor::SetCursorStyle;
                let grid_editing = self
                    .state
                    .active_tab()
                    .is_some_and(|t| t.grid_editing.is_some());
                let in_insert = grid_editing
                    || self
                        .state
                        .active_tab()
                        .and_then(|t| t.active_editor())
                        .is_some_and(|e| {
                            matches!(e.mode, vimltui::VimMode::Insert | vimltui::VimMode::Replace)
                        });
                let style = if in_insert {
                    SetCursorStyle::SteadyBar
                } else {
                    SetCursorStyle::SteadyBlock
                };
                let _ = crossterm::execute!(terminal.backend_mut(), style);
            }

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
                        // Check if already confirmed (overlay just closed)
                        if self.state.compile_confirmed {
                            self.state.compile_confirmed = false;
                            self.handle_compile_to_db(tab_id);
                        } else {
                            self.state.overlay =
                                Some(crate::ui::state::Overlay::ConfirmCompile);
                        }
                    }
                    Action::CloseResultTab => {
                        if let Some(tab) = self.state.active_tab_mut()
                            && !tab.result_tabs.is_empty()
                        {
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
                        if !self.state.column_cache.contains_key(&key) {
                            self.spawn_cache_columns(&schema, &table, key);
                        }
                    }
                    Action::ScriptOp { op } => {
                        self.handle_script_op(op);
                    }
                    Action::ReloadTableData => {
                        if let Some(tab) = self.state.active_tab_mut() {
                            tab.grid_changes.clear();
                        }
                        // Re-trigger table data load from the tab kind
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
                            self.spawn_rename_object(
                                &conn_name, &schema, &old_name, &new_name, &obj_type,
                            );
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
                }
            }
        }
        Ok(())
    }

    fn handle_paste(&mut self, text: &str) {
        use crate::ui::state::Focus;
        use vimltui::VimMode;

        // Paste into connection dialog fields
        if matches!(self.state.overlay, Some(Overlay::ConnectionDialog)) {
            if !self.state.connection_form.read_only
                && self.state.connection_form.selected_field != 1
                && self.state.connection_form.selected_field != 7
            {
                let clean: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                self.state.connection_form.active_field_mut().push_str(&clean);
                self.state.connection_form.error_message.clear();
            }
            return;
        }

        if self.state.focus != Focus::TabContent {
            return;
        }
        if let Some(tab) = self.state.active_tab_mut()
            && let Some(editor) = tab.active_editor_mut()
        {
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

    fn handle_message(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::SchemasLoaded { conn_name, schemas } => {
                let conn_idx = self.state.tree.iter().position(
                    |n| matches!(n, TreeNode::Connection { name, .. } if name == &conn_name),
                );
                if let Some(idx) = conn_idx {
                    let d = self.state.tree[idx].depth();
                    let mut end = idx + 1;
                    while end < self.state.tree.len() && self.state.tree[end].depth() > d {
                        end += 1;
                    }
                    self.state.tree.drain(idx + 1..end);

                    // Build all nodes in a batch (avoids O(n²) insert shifts)
                    let cats_template: Vec<(&str, CategoryKind)> = match self.state.db_type {
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
                    self.state.tree.splice(insert_pos..insert_pos, batch);

                    // Determine the user's own schema for priority loading
                    let user_schema = self
                        .state
                        .saved_connections
                        .iter()
                        .find(|c| c.name == conn_name)
                        .map(|c| match c.db_type {
                            DatabaseType::Oracle => c.username.to_uppercase(),
                            DatabaseType::MySQL => c.database.clone().unwrap_or_default(),
                            DatabaseType::PostgreSQL => "public".to_string(),
                        });

                    // Set current_schema to user's schema
                    if let Some(ref us) = user_schema {
                        self.state.current_schema = Some(us.clone());
                    }

                    // Warm-up: core categories for user's schema; new metadata categories stay lazy
                    if let Some(ref us) = user_schema {
                        self.spawn_load_children(us, "Tables");
                        self.spawn_load_children(us, "Views");
                        self.spawn_load_children(us, "Procedures");
                        self.spawn_load_children(us, "Functions");
                        if matches!(self.state.db_type, Some(DatabaseType::Oracle)) {
                            self.spawn_load_children(us, "Packages");
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
                        let mut labels = vec![
                            "Tables".to_string(),
                            "Views".to_string(),
                            "Procedures".to_string(),
                            "Functions".to_string(),
                        ];
                        if matches!(self.state.db_type, Some(DatabaseType::Oracle)) {
                            labels.push("Packages".to_string());
                        }
                        self.spawn_load_remaining_schemas(other_schemas, labels);
                    }
                }
                self.state.status_message = format!("Schemas loaded for {conn_name} - F to filter");
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::TablesLoaded { schema, items } => {
                self.insert_leaves(&schema, CategoryKind::Tables, items, LeafKind::Table);
                // Mark metadata ready once the primary schema's tables are loaded
                if !self.state.metadata_ready
                    && self
                        .state
                        .current_schema
                        .as_ref()
                        .is_some_and(|cs| cs.eq_ignore_ascii_case(&schema))
                {
                    self.state.metadata_ready = true;
                    self.state.status_message = "Context ready".to_string();
                    self.refresh_active_diagnostics();
                }
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::ViewsLoaded { schema, items } => {
                self.insert_leaves(&schema, CategoryKind::Views, items, LeafKind::View);
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::PackagesLoaded { schema, items } => {
                self.insert_package_leaves(&schema, items);
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::ProceduresLoaded { schema, items } => {
                self.insert_leaves(
                    &schema,
                    CategoryKind::Procedures,
                    items,
                    LeafKind::Procedure,
                );
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::FunctionsLoaded { schema, items } => {
                self.insert_leaves(&schema, CategoryKind::Functions, items, LeafKind::Function);
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::MaterializedViewsLoaded { schema, items } => {
                self.insert_leaves(
                    &schema,
                    CategoryKind::MaterializedViews,
                    items,
                    LeafKind::MaterializedView,
                );
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::IndexesLoaded { schema, items } => {
                self.insert_leaves(&schema, CategoryKind::Indexes, items, LeafKind::Index);
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::SequencesLoaded { schema, items } => {
                self.insert_leaves(&schema, CategoryKind::Sequences, items, LeafKind::Sequence);
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::TypesLoaded { schema, items } => {
                self.insert_leaves(&schema, CategoryKind::Types, items, LeafKind::Type);
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::TriggersLoaded { schema, items } => {
                self.insert_leaves(&schema, CategoryKind::Triggers, items, LeafKind::Trigger);
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::EventsLoaded { schema, items } => {
                self.insert_leaves(&schema, CategoryKind::Events, items, LeafKind::Event);
                self.state.loading = false;
                self.state.loading_since = None;
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
                self.state.loading = false;
                self.state.loading_since = None;
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
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::TableDataLoaded { tab_id, result } => {
                let row_count = result.rows.len();
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    let had_data = tab.query_result.is_some();
                    tab.query_result = Some(result);
                    if !had_data {
                        tab.grid_selected_row = 0;
                        tab.grid_scroll_row = 0;
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
                    self.state.loading = false;
                    self.state.loading_since = None;
                } else {
                    self.state.status_message =
                        format!("Loading... {total_rows} rows (+{batch_len})");
                }
            }
            AppMessage::ColumnsLoaded { tab_id, columns } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.columns = columns;
                }
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::PackageContentLoaded { tab_id, content } => {
                // Get connection name before mutating state
                let conn_name = self.state.find_tab(tab_id).and_then(|t| match &t.kind {
                    TabKind::Package { conn_name, .. } => Some(conn_name.clone()),
                    _ => None,
                });

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

                // Register in VFS
                if let Some(cn) = conn_name {
                    self.register_in_vfs(tab_id, &cn);
                }
                self.state.loading = false;
                self.state.loading_since = None;
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
                    self.state.loading = false;
                    self.state.loading_since = None;
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
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::TableDDLLoaded { tab_id, ddl } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.streaming_since = None;
                    if let Some(editor) = tab.ddl_editor.as_mut() {
                        editor.set_content(&ddl);
                    }
                }
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::GridChangesSaved { tab_id, count } => {
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    tab.grid_changes.clear();
                    tab.grid_error_editor = None;
                    tab.grid_query_editor = None;
                    tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
                }
                self.state.status_message = format!("{count} changes saved");
                self.state.loading = false;
                self.state.loading_since = None;
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
                self.state.loading = false;
                self.state.loading_since = None;
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
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::Connected { adapter, name } => {
                if self.state.overlay.is_some() {
                    let config = self.state.connection_form.to_connection_config();
                    self.save_connection_config(&config);
                    self.state.overlay = None;
                    self.state.connection_form.connecting = false;
                }

                self.set_conn_status(&name, crate::ui::state::ConnStatus::Connected);

                let already_in_tree = self
                    .state
                    .tree
                    .iter()
                    .any(|n| matches!(n, TreeNode::Connection { name: n, .. } if n == &name));

                if already_in_tree {
                    self.adapters.insert(name.clone(), Arc::clone(&adapter));
                    self.state.connected = true;
                    self.state.connection_name = Some(name.clone());
                    self.state.db_type = Some(adapter.db_type());

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
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::ObjectDropped {
                schema,
                name,
                obj_type,
            } => {
                // Remove from tree
                if let Some(idx) = self.state.tree.iter().position(|n| {
                    matches!(n, TreeNode::Leaf { name: n, schema: s, .. } if n == &name && s == &schema)
                }) {
                    self.state.tree.remove(idx);
                }
                self.state.status_message = format!("{obj_type} {schema}.{name} dropped");
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::ObjectRenamed {
                schema,
                old_name,
                new_name,
                obj_type,
            } => {
                // Update name in tree
                for node in &mut self.state.tree {
                    if let TreeNode::Leaf { name, schema: s, .. } = node
                        && *name == old_name
                        && *s == schema
                    {
                        *name = new_name.clone();
                        break;
                    }
                }
                self.state.status_message =
                    format!("{obj_type} {schema}.{old_name} → {new_name}");
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::ObjectError { error, sql } => {
                // Show error in active tab if it has an editor, or in status bar
                if let Some(tab) = self.state.active_tab_mut() {
                    use vimltui::VimEditor;
                    let formatted = format!("-- Error --\n\n{error}");
                    let mut err_editor =
                        VimEditor::new(&formatted, vimltui::VimModeConfig::read_only());
                    err_editor.mode = vimltui::VimMode::Normal;
                    let mut q_editor =
                        VimEditor::new(&sql, vimltui::VimModeConfig::read_only());
                    q_editor.mode = vimltui::VimMode::Normal;
                    tab.grid_error_editor = Some(err_editor);
                    tab.grid_query_editor = Some(q_editor);
                }
                self.state.status_message = format!("Error: {error}");
                self.state.loading = false;
                self.state.loading_since = None;
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
                    && let Some(schema) = self.state.current_schema.clone()
                {
                    self.spawn_load_children(&schema, kind);
                }
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
                            let insert_idx = self.find_or_create_group_insert_idx(&config.group);
                            self.state.tree.insert(
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
                for node in &mut self.state.tree {
                    if let TreeNode::Connection { status, .. } = node
                        && *status == crate::ui::state::ConnStatus::Connecting
                    {
                        *status = crate::ui::state::ConnStatus::Failed;
                    }
                }
                self.state.status_message = format!("Error: {msg}");
                self.state.loading = false;
                self.state.loading_since = None;
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
                self.state.loading_since = None;
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
                    self.state.status_message = "Compiled to database".to_string();
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
                        let mut err_editor = VimEditor::new(
                            &err_header,
                            vimltui::VimModeConfig::read_only(),
                        );
                        err_editor.mode = vimltui::VimMode::Normal;

                        let mut q_editor = VimEditor::new(
                            &failed_sql,
                            vimltui::VimModeConfig::read_only(),
                        );
                        q_editor.mode = vimltui::VimMode::Normal;

                        tab.grid_error_editor = Some(err_editor);
                        tab.grid_query_editor = Some(q_editor);
                        tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
                    }

                    self.state.status_message = format!("Compilation failed: {message}");
                }
                self.state.loading = false;
                self.state.loading_since = None;
            }
            AppMessage::ColumnsCached { key, columns } => {
                self.state.column_cache.insert(key, columns);
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

            if items.is_empty() {
                self.state.tree.insert(idx + 1, TreeNode::Empty);
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
            self.state.tree.splice(insert_pos..insert_pos, batch);
        }
    }

    fn insert_package_leaves(&mut self, schema: &str, items: Vec<Package>) {
        let cat_idx = self.state.tree.iter().position(|n| {
            matches!(n, TreeNode::Category { schema: s, kind: CategoryKind::Packages, .. } if s == schema)
        });
        if let Some(idx) = cat_idx {
            self.remove_children_of(idx);

            if items.is_empty() {
                self.state.tree.insert(idx + 1, TreeNode::Empty);
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
            self.state.tree.splice(insert_pos..insert_pos, batch);
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
    fn spawn_load_remaining_schemas(&self, schemas: Vec<String>, category_labels: Vec<String>) {
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

    fn spawn_load_table_data(&self, tab_id: TabId, schema: &str, table: &str) {
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

    fn spawn_cache_columns(&self, schema: &str, table: &str, key: String) {
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

    fn execute_grid_changes(&mut self) {
        use crate::ui::tabs::RowChange;

        let tab_idx = self.state.active_tab_idx;
        let tab = &self.state.tabs[tab_idx];

        // Extract table info
        let (schema, table) = match &tab.kind {
            TabKind::Table { schema, table, .. } => (schema.clone(), table.clone()),
            _ => return,
        };

        // Check we have PK columns
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

        // Build SQL statements
        let mut statements: Vec<String> = Vec::new();

        // Collect changes sorted by row
        let mut changes: Vec<(usize, &RowChange)> =
            tab.grid_changes.iter().map(|(k, v)| (*k, v)).collect();
        changes.sort_by_key(|(k, _)| *k);

        for (row_idx, change) in &changes {
            match change {
                RowChange::Modified { edits } => {
                    if pk_cols.is_empty() {
                        self.state.status_message =
                            "Cannot UPDATE: table has no primary key".to_string();
                        return;
                    }
                    let row_data = tab.query_result.as_ref().and_then(|r| r.rows.get(*row_idx));
                    if let Some(row_data) = row_data {
                        let set_clause: String = edits
                            .iter()
                            .map(|e| {
                                let col_name =
                                    all_col_names.get(e.col).cloned().unwrap_or_default();
                                format!("{} = '{}'", col_name, e.value.replace('\'', "''"))
                            })
                            .collect::<Vec<_>>()
                            .join(",\n       ");
                        let where_clause: String = pk_cols
                            .iter()
                            .map(|(i, name)| {
                                let val = row_data.get(*i).cloned().unwrap_or_default();
                                format!("{} = '{}'", name, val.replace('\'', "''"))
                            })
                            .collect::<Vec<_>>()
                            .join(" AND ");
                        statements.push(format!(
                            "UPDATE {schema}.{table}\n  SET {set_clause}\n  WHERE {where_clause}"
                        ));
                    }
                }
                RowChange::New { values } => {
                    let cols = all_col_names.join(", ");
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
                        "INSERT INTO {schema}.{table}\n  ({cols})\n  VALUES ({vals})"
                    ));
                }
                RowChange::Deleted => {
                    if pk_cols.is_empty() {
                        self.state.status_message =
                            "Cannot DELETE: table has no primary key".to_string();
                        return;
                    }
                    let row_data = tab.query_result.as_ref().and_then(|r| r.rows.get(*row_idx));
                    if let Some(row_data) = row_data {
                        let where_clause: String = pk_cols
                            .iter()
                            .map(|(i, name)| {
                                let val = row_data.get(*i).cloned().unwrap_or_default();
                                format!("{} = '{}'", name, val.replace('\'', "''"))
                            })
                            .collect::<Vec<_>>()
                            .join(" AND ");
                        statements.push(format!(
                            "DELETE FROM {schema}.{table}\n  WHERE {where_clause}"
                        ));
                    }
                }
            }
        }

        if statements.is_empty() {
            self.state.status_message = "No changes to save".to_string();
            return;
        }

        // Get adapter
        let (_, adapter) = match self.active_adapter() {
            Some(a) => a,
            None => {
                self.state.status_message = "No active connection".to_string();
                return;
            }
        };

        let tx = self.msg_tx.clone();
        let tab_id = self.state.tabs[tab_idx].id;
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

    fn spawn_execute_query_at(&self, tab_id: TabId, query: &str, new_tab: bool, start_line: usize) {
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

        tokio::spawn(async move {
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
                        let _ = tx
                            .send(AppMessage::QueryBatch {
                                tab_id,
                                columns: batch.columns,
                                rows: batch.rows,
                                done,
                                new_tab,
                                elapsed: if done { Some(start.elapsed()) } else { None },
                            })
                            .await;
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
                    let _ = tx.send(AppMessage::DdlExecuted { query: query.clone() }).await;
                }
            }

            // Check if the streaming task itself failed
            if !had_error && let Ok(Err(e)) = stream_handle.await {
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
        });
    }

    fn check_leader_help_timeout(&mut self) {
        // Sub-menus appear immediately
        if self.state.leader_b_pending
            || self.state.leader_w_pending
            || self.state.leader_s_pending
            || self.state.leader_leader_pending
        {
            self.state.leader_help_visible = true;
            return;
        }
        // Root leader menu appears immediately
        if self.state.leader_pending {
            self.state.leader_help_visible = true;
            return;
        }
        // No leader pending → hide
        if self.state.leader_help_visible {
            self.state.leader_help_visible = false;
        }
    }

    fn open_script_conn_picker(&mut self) {
        let connected: std::collections::HashSet<String> = self.adapters.keys().cloned().collect();

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
        if let Some(tab) = self.state.active_tab()
            && let TabKind::Script {
                conn_name: Some(cn),
                ..
            } = &tab.kind
        {
            let items = picker.visible_items();
            if let Some(pos) = items.iter().position(|item| match item {
                crate::ui::state::PickerItem::Active(n) => n == cn,
                _ => false,
            }) {
                picker.cursor = pos;
            }
        }

        self.state.script_conn_picker = Some(picker);
        self.state.overlay = Some(Overlay::ScriptConnection);
    }

    fn set_script_connection(&mut self, conn_name: &str) {
        if !self.adapters.contains_key(conn_name) {
            self.connect_by_name(conn_name);
        }
        if let Some(tab) = self.state.active_tab_mut()
            && let TabKind::Script {
                conn_name: ref mut cn,
                ref file_path,
                ref name,
                ..
            } = tab.kind
        {
            *cn = Some(conn_name.to_string());
            // Use file_path (with collection prefix) as key, fallback to name
            let key = file_path
                .as_ref()
                .map(|fp| fp.strip_suffix(".sql").unwrap_or(fp))
                .unwrap_or(name);
            save_script_connection(key, conn_name);
        }
        self.state.status_message = format!("Script → {conn_name}");
        self.refresh_active_diagnostics();
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

    fn spawn_load_type_info(&self, tab_id: TabId, schema: &str, name: &str) {
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

    fn spawn_load_trigger_info(&self, tab_id: TabId, schema: &str, name: &str) {
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

    fn spawn_drop_object(&self, conn_name: &str, schema: &str, name: &str, obj_type: &str) {
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

    fn spawn_rename_object(
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

    fn open_template_script(&mut self, conn_name: &str, schema: &str, obj_type: &str) {
        let db_type = self.state.db_type;
        let template = match (obj_type, db_type) {
            ("TABLE", Some(DatabaseType::Oracle)) => format!(
                "CREATE TABLE {schema}.new_table (\n\
                 \x20   id NUMBER PRIMARY KEY,\n\
                 \x20   name VARCHAR2(100) NOT NULL,\n\
                 \x20   created_at DATE DEFAULT SYSDATE\n\
                 );"
            ),
            ("TABLE", Some(DatabaseType::PostgreSQL)) => format!(
                "CREATE TABLE {schema}.new_table (\n\
                 \x20   id SERIAL PRIMARY KEY,\n\
                 \x20   name VARCHAR(100) NOT NULL,\n\
                 \x20   created_at TIMESTAMP DEFAULT NOW()\n\
                 );"
            ),
            ("TABLE", Some(DatabaseType::MySQL)) => format!(
                "CREATE TABLE `{schema}`.`new_table` (\n\
                 \x20   id INT AUTO_INCREMENT PRIMARY KEY,\n\
                 \x20   name VARCHAR(100) NOT NULL,\n\
                 \x20   created_at DATETIME DEFAULT CURRENT_TIMESTAMP\n\
                 );"
            ),
            ("VIEW", Some(DatabaseType::Oracle)) => format!(
                "CREATE OR REPLACE VIEW {schema}.new_view AS\n\
                 SELECT * FROM {schema}.table_name;"
            ),
            ("VIEW", _) => format!(
                "CREATE VIEW {schema}.new_view AS\n\
                 SELECT * FROM {schema}.table_name;"
            ),
            ("PACKAGE", _) => format!(
                "CREATE OR REPLACE PACKAGE {schema}.new_package AS\n\
                 \x20   -- declarations\n\
                 END;\n\
                 /"
            ),
            _ => format!("-- CREATE {obj_type} {schema}.new_object"),
        };

        let script_num = self
            .state
            .tabs
            .iter()
            .filter(|t| matches!(t.kind, TabKind::Script { .. }))
            .count()
            + 1;
        let name = format!("Script {script_num}");
        let tab_id = self.state.open_or_focus_tab(TabKind::Script {
            file_path: None,
            name,
            conn_name: Some(conn_name.to_string()),
        });
        if let Some(tab) = self.state.find_tab_mut(tab_id)
            && let Some(editor) = tab.editor.as_mut()
        {
            editor.set_content(&template);
            editor.mode = vimltui::VimMode::Normal;
        }
    }

    fn rename_connection(&mut self, old_name: &str, new_name: &str) {
        // Check for name collision
        if self.state.saved_connections.iter().any(|c| c.name == new_name) {
            self.state.status_message = format!("Connection '{new_name}' already exists");
            return;
        }

        // Update saved config
        if let Some(config) = self
            .state
            .saved_connections
            .iter_mut()
            .find(|c| c.name == old_name)
        {
            config.name = new_name.to_string();
        }

        // Update tree node
        for node in &mut self.state.tree {
            if let TreeNode::Connection { name, .. } = node
                && *name == old_name
            {
                *name = new_name.to_string();
                break;
            }
        }

        // Update adapter key
        if let Some(adapter) = self.adapters.remove(old_name) {
            self.adapters.insert(new_name.to_string(), adapter);
        }

        // Update active connection name
        if self.state.connection_name.as_deref() == Some(old_name) {
            self.state.connection_name = Some(new_name.to_string());
        }

        // Update tab connection references
        for tab in &mut self.state.tabs {
            match &mut tab.kind {
                TabKind::Table { conn_name, .. }
                | TabKind::Package { conn_name, .. }
                | TabKind::Function { conn_name, .. }
                | TabKind::Procedure { conn_name, .. }
                | TabKind::DbType { conn_name, .. }
                | TabKind::Trigger { conn_name, .. } => {
                    if *conn_name == old_name {
                        *conn_name = new_name.to_string();
                    }
                }
                TabKind::Script { conn_name, .. } => {
                    if conn_name.as_deref() == Some(old_name) {
                        *conn_name = Some(new_name.to_string());
                    }
                }
            }
        }

        self.persist_connections();
        self.state.status_message = format!("Connection renamed: {old_name} → {new_name}");
    }

    fn duplicate_connection(&mut self, source_name: &str, target_group: &str) {
        if let Some(config) = self
            .state
            .saved_connections
            .iter()
            .find(|c| c.name == source_name)
            .cloned()
        {
            let mut new_config = config;
            new_config.name = format!("{source_name} (copy)");
            new_config.group = target_group.to_string();
            // Avoid name collisions
            let mut n = 1;
            while self
                .state
                .saved_connections
                .iter()
                .any(|c| c.name == new_config.name)
            {
                n += 1;
                new_config.name = format!("{source_name} (copy {n})");
            }
            self.save_connection_config(&new_config);
            let insert_idx = self.find_or_create_group_insert_idx(&new_config.group);
            self.state.tree.insert(
                insert_idx,
                TreeNode::Connection {
                    name: new_config.name.clone(),
                    expanded: false,
                    status: crate::ui::state::ConnStatus::Disconnected,
                },
            );
            self.state.status_message = format!("Connection duplicated: {}", new_config.name);
        }
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

    fn spawn_connect(&mut self) {
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

    /// Re-run diagnostics on the active editor to clear stale results.
    fn refresh_active_diagnostics(&mut self) {
        let tab_data = self.state.active_tab().map(|t| {
            let conn = t.kind.conn_name().map(|s| s.to_string());
            let lines = t.active_editor().map(|e| e.lines.clone());
            (conn, lines)
        });
        if let Some((script_conn, Some(lines))) = tab_data {
            self.state.diagnostics =
                crate::ui::diagnostics::check_sql(&self.state, &lines, script_conn.as_deref());
        }
    }

    fn set_conn_status(&mut self, conn_name: &str, status: crate::ui::state::ConnStatus) {
        for node in &mut self.state.tree {
            if let TreeNode::Connection {
                name, status: s, ..
            } = node
                && name == conn_name
            {
                *s = status;
                break;
            }
        }
    }

    fn connect_by_name(&mut self, name: &str) {
        self.adapters.remove(name);
        self.state.metadata_ready = false;
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
        } else {
            self.state.status_message = format!("No saved config for '{name}'");
        }
    }

    fn disconnect_by_name(&mut self, name: &str) {
        self.adapters.remove(name);
        self.state.metadata_ready = false;
        self.set_conn_status(name, crate::ui::state::ConnStatus::Disconnected);

        if let Some(conn_idx) = self
            .state
            .tree
            .iter()
            .position(|n| matches!(n, TreeNode::Connection { name: n, .. } if n == name))
        {
            if let TreeNode::Connection { expanded, .. } = &mut self.state.tree[conn_idx] {
                *expanded = false;
            }
            let d = self.state.tree[conn_idx].depth();
            let mut end = conn_idx + 1;
            while end < self.state.tree.len() && self.state.tree[end].depth() > d {
                end += 1;
            }
            self.state.tree.drain(conn_idx + 1..end);
        }

        if self.state.connection_name.as_deref() == Some(name) {
            self.state.connected = false;
            self.state.connection_name = None;
        }

        self.state.status_message = format!("Disconnected from '{name}'");
    }

    fn delete_connection(&mut self, name: &str) {
        self.adapters.remove(name);

        if let Some(conn_idx) = self
            .state
            .tree
            .iter()
            .position(|n| matches!(n, TreeNode::Connection { name: n, .. } if n == name))
        {
            let d = self.state.tree[conn_idx].depth();
            let mut end = conn_idx + 1;
            while end < self.state.tree.len() && self.state.tree[end].depth() > d {
                end += 1;
            }
            self.state.tree.drain(conn_idx..end);
        }

        self.state.saved_connections.retain(|c| c.name != name);
        self.persist_connections();
        self.remove_empty_groups();
        self.persist_groups();

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
        // Track old group for potential tree move
        let old_group = self
            .state
            .connection_form
            .editing_name
            .as_ref()
            .and_then(|old| {
                self.state
                    .saved_connections
                    .iter()
                    .find(|c| c.name == *old)
                    .map(|c| c.group.clone())
            });

        if let Some(old_name) = self.state.connection_form.editing_name.take() {
            self.state.saved_connections.retain(|c| c.name != old_name);
            if old_name != config.name {
                if let Some(adapter) = self.adapters.remove(&old_name) {
                    self.adapters.insert(config.name.clone(), adapter);
                }
                for node in &mut self.state.tree {
                    if let TreeNode::Connection { name, .. } = node
                        && *name == old_name
                    {
                        *name = config.name.clone();
                    }
                }
                if self.state.connection_name.as_deref() == Some(&old_name) {
                    self.state.connection_name = Some(config.name.clone());
                }
            }

            // If group changed, move the connection node in the tree
            if old_group.as_deref() != Some(&config.group) {
                // Remove connection + its children from old position
                if let Some(conn_idx) = self.state.tree.iter().position(
                    |n| matches!(n, TreeNode::Connection { name, .. } if name == &config.name),
                ) {
                    let d = self.state.tree[conn_idx].depth();
                    let mut end = conn_idx + 1;
                    while end < self.state.tree.len() && self.state.tree[end].depth() > d {
                        end += 1;
                    }
                    let nodes: Vec<_> = self.state.tree.drain(conn_idx..end).collect();
                    // Insert into new group
                    let insert_idx = self.find_or_create_group_insert_idx(&config.group);
                    for (i, node) in nodes.into_iter().enumerate() {
                        self.state.tree.insert(insert_idx + i, node);
                    }
                }
                // Remove old group if now empty
                self.remove_empty_groups();
            }
        }

        self.state
            .saved_connections
            .retain(|c| c.name != config.name);
        self.state.saved_connections.push(config.clone());
        self.persist_connections();
        self.persist_groups();
    }

    /// Remove group nodes that have no children
    fn remove_empty_groups(&mut self) {
        let mut i = 0;
        while i < self.state.tree.len() {
            if let TreeNode::Group { .. } = &self.state.tree[i] {
                let next_is_child = i + 1 < self.state.tree.len()
                    && self.state.tree[i + 1].depth() > self.state.tree[i].depth();
                if !next_is_child {
                    self.state.tree.remove(i);
                    continue;
                }
            }
            i += 1;
        }
    }

    /// Find the insert index for a connection within a group.
    /// If the group doesn't exist yet, creates it and returns the index after it.
    fn find_or_create_group_insert_idx(&mut self, group_name: &str) -> usize {
        // Find existing group node
        for i in 0..self.state.tree.len() {
            if let TreeNode::Group { name, .. } = &self.state.tree[i]
                && name == group_name
            {
                let d = self.state.tree[i].depth();
                let mut end = i + 1;
                while end < self.state.tree.len() && self.state.tree[end].depth() > d {
                    end += 1;
                }
                return end;
            }
        }
        // Group doesn't exist — create it at the end
        self.state.tree.push(TreeNode::Group {
            name: group_name.to_string(),
            expanded: true,
        });
        self.state.tree.len()
    }

    fn persist_connections(&self) {
        if let Ok(store) = crate::core::storage::ConnectionStore::new() {
            let _ = store.save(&self.state.saved_connections, "");
        }
    }

    /// Persist the current list of group names (so empty groups survive restarts)
    fn persist_groups(&self) {
        if let Ok(store) = crate::core::storage::ConnectionStore::new() {
            let groups: Vec<String> = self
                .state
                .tree
                .iter()
                .filter_map(|n| {
                    if let TreeNode::Group { name, .. } = n {
                        return Some(name.clone());
                    }
                    None
                })
                .collect();
            let _ = store.save_groups(&groups);
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
        if let Ok(store) = crate::core::storage::ConnectionStore::new()
            && let Ok(configs) = store.load("")
        {
            self.state.saved_connections = configs.clone();

            // Build grouped tree: collect unique groups from persisted + connections
            let mut seen = std::collections::HashSet::new();
            let mut groups_order: Vec<String> = Vec::new();
            // Include persisted groups (preserves order, includes "Default" if it was saved)
            if let Ok(persisted_groups) = store.load_groups() {
                for g in persisted_groups {
                    if seen.insert(g.clone()) {
                        groups_order.push(g);
                    }
                }
            }
            // Include groups from connections
            for config in &configs {
                if seen.insert(config.group.clone()) {
                    groups_order.push(config.group.clone());
                }
            }
            // If no groups at all, add "Default" as fallback
            if groups_order.is_empty() && !configs.is_empty() {
                groups_order.push("Default".to_string());
            }

            for group in &groups_order {
                let group_conns: Vec<_> = configs.iter().filter(|c| &c.group == group).collect();
                if group_conns.is_empty() && !seen.contains(group) {
                    continue;
                }
                self.state.tree.push(TreeNode::Group {
                    name: group.clone(),
                    expanded: false,
                });
                for config in group_conns {
                    self.state.tree.push(TreeNode::Connection {
                        name: config.name.clone(),
                        expanded: false,
                        status: crate::ui::state::ConnStatus::Disconnected,
                    });
                }
            }
            if !configs.is_empty() {
                self.state.status_message =
                    format!("{} connection(s) loaded - expand to connect", configs.len());
            }
        }
        self.load_object_filter();
        self.refresh_scripts_list();
    }

    fn load_object_filter(&mut self) {
        if let Ok(dir) = crate::core::storage::ConnectionStore::new()
            && let Ok(data) = std::fs::read_to_string(dir.dir_path().join("object_filters.json"))
            && let Ok(filters) = serde_json::from_str::<HashMap<String, Vec<String>>>(&data)
        {
            for (key, names) in filters {
                let set: HashSet<String> = names.into_iter().collect();
                if !set.is_empty() {
                    self.state.object_filter.filters.insert(key, set);
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
                    let total: usize = self
                        .state
                        .object_filter
                        .filters
                        .values()
                        .map(|s| s.len())
                        .sum();
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
                TabKind::Script { .. } => tab.editor.as_ref().map(|e| e.modified).unwrap_or(false),
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
        if let Some(tab) = self.state.active_tab()
            && let TabKind::Script {
                ref file_path,
                ref name,
                ..
            } = tab.kind
            && file_path.is_none()
        {
            // New script: prompt for name
            self.state.scripts_save_name = Some(name.clone());
            self.state.overlay = Some(Overlay::SaveScriptName);
            return;
        }
        self.do_save_script(None);
    }

    fn do_save_script(&mut self, new_name: Option<&str>) {
        if let Some(tab) = self.state.active_tab_mut()
            && let TabKind::Script {
                ref mut name,
                ref mut file_path,
                ..
            } = tab.kind
        {
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
        self.refresh_scripts_list();
    }
    fn refresh_scripts_list(&mut self) {
        use crate::ui::state::ScriptNode;
        if let Ok(store) = crate::core::storage::ScriptStore::new()
            && let Ok(tree) = store.list_tree()
        {
            let mut nodes = Vec::new();
            for coll in &tree.collections {
                let was_expanded = self.state.scripts_tree.iter().any(|n| {
                    matches!(n, ScriptNode::Collection { name, expanded: true }
                        if *name == coll.name)
                });
                nodes.push(ScriptNode::Collection {
                    name: coll.name.clone(),
                    expanded: was_expanded,
                });
                for script in &coll.scripts {
                    let base = script.strip_suffix(".sql").unwrap_or(script).to_string();
                    nodes.push(ScriptNode::Script {
                        name: base,
                        collection: Some(coll.name.clone()),
                        file_path: format!("{}/{script}", coll.name),
                    });
                }
            }
            for script in &tree.root_scripts {
                let base = script.strip_suffix(".sql").unwrap_or(script).to_string();
                nodes.push(ScriptNode::Script {
                    name: base,
                    collection: None,
                    file_path: script.clone(),
                });
            }
            self.state.scripts_tree = nodes;
            let visible_count = self.state.visible_scripts().len();
            if self.state.scripts_cursor >= visible_count && visible_count > 0 {
                self.state.scripts_cursor = visible_count - 1;
            }
        }
    }

    fn open_script(&mut self, name: &str) {
        if let Ok(store) = crate::core::storage::ScriptStore::new() {
            if let Ok(content) = store.read(&format!("{name}.sql")) {
                // Display name = last segment (without collection prefix)
                let display_name = name.rsplit('/').next().unwrap_or(name).to_string();

                // Load saved connection: try full path first, then base name (backward compat)
                let saved_conn =
                    load_script_connection(name).or_else(|| load_script_connection(&display_name));

                // Auto-connect if saved connection exists but isn't active
                let needs_connect = saved_conn
                    .as_ref()
                    .is_some_and(|cn| !self.adapters.contains_key(cn.as_str()));
                if let Some(ref cn) = saved_conn
                    && needs_connect
                {
                    self.connect_by_name(cn);
                }

                let tab_id = self.state.open_or_focus_tab(TabKind::Script {
                    file_path: Some(format!("{name}.sql")),
                    name: display_name,
                    conn_name: saved_conn,
                });
                if let Some(tab) = self.state.find_tab_mut(tab_id)
                    && let Some(editor) = tab.editor.as_mut()
                {
                    editor.set_content(&content);
                }
                if needs_connect {
                    self.state.status_message = "Loading context...".to_string();
                    self.state.loading = true;
                    self.state.loading_since = Some(std::time::Instant::now());
                } else {
                    self.state.status_message = format!("Opened script '{name}'");
                }
            } else {
                self.state.status_message = format!("Error reading script '{name}'");
            }
        }
    }

    fn handle_script_op(&mut self, op: crate::ui::events::ScriptOperation) {
        use crate::ui::events::ScriptOperation;
        if let Ok(store) = crate::core::storage::ScriptStore::new() {
            match op {
                ScriptOperation::Create {
                    name,
                    in_collection,
                } => {
                    if name.ends_with('/') {
                        let dir_name = name.trim_end_matches('/');
                        if let Err(e) = store.create_collection(dir_name) {
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
                    // Compute new path preserving collection prefix
                    let prefix = old_path.rfind('/').map(|i| &old_path[..=i]).unwrap_or("");
                    let new_path = format!("{prefix}{new_name}.sql");
                    if let Ok(content) = store.read(&old_path) {
                        let _ = store.save(&new_path, &content);
                        let _ = store.delete(&old_path);
                        // Update open tabs
                        for tab in &mut self.state.tabs {
                            if let TabKind::Script {
                                ref mut name,
                                ref mut file_path,
                                ..
                            } = tab.kind
                                && file_path.as_deref() == Some(old_path.as_str())
                            {
                                *name = new_name.clone();
                                *file_path = Some(new_path.clone());
                            }
                        }
                    }
                }
                ScriptOperation::RenameCollection { old_name, new_name } => {
                    if let Err(e) = store.rename_collection(&old_name, &new_name) {
                        self.state.status_message = format!("Error: {e}");
                    } else {
                        for tab in &mut self.state.tabs {
                            if let TabKind::Script {
                                ref mut file_path, ..
                            } = tab.kind
                                && let Some(fp) = file_path
                                && fp.starts_with(&format!("{old_name}/"))
                            {
                                *fp = fp.replacen(&old_name, &new_name, 1);
                            }
                        }
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
                            for tab in &mut self.state.tabs {
                                if let TabKind::Script {
                                    ref mut file_path, ..
                                } = tab.kind
                                    && file_path.as_deref() == Some(from.as_str())
                                {
                                    *file_path = Some(to.clone());
                                }
                            }
                            self.state.status_message =
                                format!("Moved to {}", to_collection.as_deref().unwrap_or("root"));
                        }
                    }
                }
            }
        }
        self.refresh_scripts_list();
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
                let schema = schema.clone();
                let name = name.clone();
                let conn = conn_name.to_string();

                let vfs = self.vfs_for(&conn);
                vfs.get_or_create(
                    FileType::PackageDeclaration {
                        schema: schema.clone(),
                        package: name.clone(),
                    },
                    decl,
                );
                vfs.get_or_create(
                    FileType::PackageBody {
                        schema,
                        package: name,
                    },
                    body,
                );
            }
            TabKind::Function { schema, name, .. } => {
                let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
                let schema = schema.clone();
                let name = name.clone();
                let conn = conn_name.to_string();

                let vfs = self.vfs_for(&conn);
                vfs.get_or_create(FileType::Function { schema, name }, content);
            }
            TabKind::Procedure { schema, name, .. } => {
                let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
                let schema = schema.clone();
                let name = name.clone();
                let conn = conn_name.to_string();

                let vfs = self.vfs_for(&conn);
                vfs.get_or_create(FileType::Procedure { schema, name }, content);
            }
            _ => {}
        }
    }

    /// Get VFS path for a tab
    fn vfs_path_for_tab(&self, tab_id: TabId) -> Option<(String, String)> {
        let tab = self.state.find_tab(tab_id)?;
        match &tab.kind {
            TabKind::Package {
                conn_name,
                schema,
                name,
            } => {
                let sub = tab.active_sub_view.as_ref();
                let path = match sub {
                    Some(SubView::PackageBody) => {
                        VirtualFileSystem::path_for_package_body(schema, name)
                    }
                    _ => VirtualFileSystem::path_for_package_decl(schema, name),
                };
                Some((conn_name.clone(), path))
            }
            TabKind::Function {
                conn_name,
                schema,
                name,
            } => Some((
                conn_name.clone(),
                VirtualFileSystem::path_for_function(schema, name),
            )),
            TabKind::Procedure {
                conn_name,
                schema,
                name,
            } => Some((
                conn_name.clone(),
                VirtualFileSystem::path_for_procedure(schema, name),
            )),
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
        if let Some(tab) = self.state.find_tab(tab_id)
            && let TabKind::Package { schema, name, .. } = &tab.kind
        {
            let other_path = match tab.active_sub_view.as_ref() {
                Some(SubView::PackageBody) => {
                    VirtualFileSystem::path_for_package_decl(schema, name)
                }
                _ => VirtualFileSystem::path_for_package_body(schema, name),
            };

            let other_content = match tab.active_sub_view.as_ref() {
                Some(SubView::PackageBody) => tab
                    .decl_editor
                    .as_ref()
                    .map(|e| e.content())
                    .unwrap_or_default(),
                _ => tab
                    .body_editor
                    .as_ref()
                    .map(|e| e.content())
                    .unwrap_or_default(),
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
            TabKind::Package {
                conn_name,
                schema,
                name,
            } => {
                let path = match tab.active_sub_view.as_ref() {
                    Some(SubView::PackageBody) => {
                        VirtualFileSystem::path_for_package_body(schema, name)
                    }
                    _ => VirtualFileSystem::path_for_package_decl(schema, name),
                };
                (conn_name, path)
            }
            TabKind::Function {
                conn_name,
                schema,
                name,
            } => (
                conn_name,
                VirtualFileSystem::path_for_function(schema, name),
            ),
            TabKind::Procedure {
                conn_name,
                schema,
                name,
            } => (
                conn_name,
                VirtualFileSystem::path_for_procedure(schema, name),
            ),
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

        let (conn_name, schema, content, obj_type) = match extract_source_info(tab) {
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
        self.state.status_message = format!("Validating {obj_type}...");
        self.state.loading = true;
        self.state.loading_since = Some(std::time::Instant::now());

        tokio::spawn(async move {
            let validator = SqlValidator::new(db_type);
            let report = validator
                .validate_thorough(&schema, &content, &adapter)
                .await;
            let _ = tx
                .send(AppMessage::ValidationResult { tab_id, report })
                .await;
        });
    }

    /// Handle <leader><leader>s: quick syntax + compile to DB
    fn handle_compile_to_db(&mut self, tab_id: TabId) {
        let tab = match self.state.find_tab(tab_id) {
            Some(t) => t,
            None => return,
        };

        let (conn_name, _obj_type) = match extract_source_info(tab) {
            Some((cn, _, _, ot)) => (cn, ot),
            None => return,
        };

        // Build per-statement list: Package has separate decl/body, others use single content
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
                stmts.push(decl);
            }
            if !body.trim().is_empty() {
                stmts.push(body);
            }
            stmts
        } else {
            vec![tab.editor.as_ref().map(|e| e.content()).unwrap_or_default()]
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
            let validator = SqlValidator::new(db_type);

            let part_names: Vec<&str> = if sql_statements.len() > 1 {
                vec!["DECLARATION", "BODY"]
            } else {
                vec!["SOURCE"]
            };

            // Quick syntax check first
            for (idx, sql) in sql_statements.iter().enumerate() {
                let syntax = validator.validate_syntax(sql);
                if !syntax.is_valid {
                    let _ = tx
                        .send(AppMessage::CompileResult {
                            tab_id,
                            success: false,
                            message: syntax.error_summary(),
                            failed_sql: sql.clone(),
                            failed_part: part_names.get(idx).unwrap_or(&"SOURCE").to_string(),
                        })
                        .await;
                    return;
                }
            }

            // Compile each statement
            for (idx, sql) in sql_statements.iter().enumerate() {
                if let Err(e) = validator.compile_to_db(sql, &adapter).await {
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

    fn save_theme_preference(&self, name: &str) {
        if let Ok(dir) = crate::core::storage::ConnectionStore::new() {
            let path = dir.dir_path().join("theme.txt");
            let _ = std::fs::write(path, name);
        }
    }

    pub fn load_theme_preference(&mut self) {
        if let Ok(dir) = crate::core::storage::ConnectionStore::new() {
            let path = dir.dir_path().join("theme.txt");
            if let Ok(name) = std::fs::read_to_string(path) {
                let name = name.trim();
                if !name.is_empty() {
                    self.theme = crate::ui::theme::Theme::by_name(name);
                }
            }
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

/// Load the saved connection name for a script
fn load_script_connection(script_name: &str) -> Option<String> {
    let dir = crate::core::storage::ConnectionStore::new().ok()?;
    let path = dir.dir_path().join("script_connections.json");
    let data = std::fs::read_to_string(&path).ok()?;
    let map: std::collections::HashMap<String, String> = serde_json::from_str(&data).ok()?;
    map.get(script_name).cloned()
}

/// Load saved bind variable values (all variables across all scripts)
pub fn load_bind_variable_values() -> std::collections::HashMap<String, String> {
    let dir = match crate::core::storage::ConnectionStore::new() {
        Ok(d) => d,
        Err(_) => return std::collections::HashMap::new(),
    };
    let path = dir.dir_path().join("bind_variables.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

/// Save bind variable values to disk
pub fn save_bind_variable_values(vars: &[(String, String)]) {
    let dir = match crate::core::storage::ConnectionStore::new() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = dir.dir_path().join("bind_variables.json");

    let mut map: std::collections::HashMap<String, String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default();

    for (name, value) in vars {
        if !value.is_empty() {
            map.insert(name.clone(), value.clone());
        }
    }

    if let Ok(json) = serde_json::to_string_pretty(&map) {
        let _ = std::fs::write(&path, json);
    }
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
