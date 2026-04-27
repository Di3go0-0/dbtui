mod actions;
mod connections;
mod message_helpers;
mod messages;
mod persistence;
mod schema_handlers;
mod spawns;
mod tabs;
mod vfs;
pub(crate) use persistence::*;

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
        conn_name: String,
        schema: String,
        items: Vec<Table>,
    },
    ViewsLoaded {
        conn_name: String,
        schema: String,
        items: Vec<View>,
    },
    PackagesLoaded {
        conn_name: String,
        schema: String,
        items: Vec<Package>,
    },
    ProceduresLoaded {
        conn_name: String,
        schema: String,
        items: Vec<Procedure>,
    },
    FunctionsLoaded {
        conn_name: String,
        schema: String,
        items: Vec<Function>,
    },
    MaterializedViewsLoaded {
        conn_name: String,
        schema: String,
        items: Vec<MaterializedView>,
    },
    IndexesLoaded {
        conn_name: String,
        schema: String,
        items: Vec<Index>,
    },
    SequencesLoaded {
        conn_name: String,
        schema: String,
        items: Vec<Sequence>,
    },
    TypesLoaded {
        conn_name: String,
        schema: String,
        items: Vec<DbType>,
    },
    TriggersLoaded {
        conn_name: String,
        schema: String,
        items: Vec<Trigger>,
    },
    EventsLoaded {
        conn_name: String,
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
    /// Result of an on-demand package-member load triggered by completion.
    /// Carries just the declaration text — the messages handler extracts
    /// function/procedure names from it and stashes them in the connection's
    /// MetadataIndex without creating a tab.
    PackageMembersLoaded {
        conn_name: String,
        schema: String,
        package: String,
        declaration: String,
    },
    /// Result of an on-demand function return-type load triggered by the
    /// completion engine when the user types `alias.<cursor>` and `alias`
    /// resolves to a `TABLE(pkg.fn()) alias` ref.
    FunctionReturnColumnsLoaded {
        conn_name: String,
        schema: Option<String>,
        package: Option<String>,
        function: String,
        columns: Vec<Column>,
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
        conn_name: String,
        key: String,
        columns: Vec<Column>,
    },
    /// Server-side compile-check diagnostics (Pass 4) arrived.
    ServerDiagnosticsResult {
        diagnostics: Vec<crate::core::models::CompileDiagnostic>,
        generation: u64,
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
            .dialogs
            .saved_connections
            .iter()
            .find(|c| c.name == conn_name)
            .map(|c| c.group.clone())
            .unwrap_or_else(|| "Default".to_string());

        // Find the group node or create it, then insert connection after group's last child
        let insert_idx = self.find_or_create_group_insert_idx(&group);
        self.state.sidebar.tree.insert(
            insert_idx,
            TreeNode::Connection {
                name: conn_name.to_string(),
                expanded: true,
                status: crate::ui::state::ConnStatus::Connected,
            },
        );

        self.state.conn.connected = true;
        self.state.conn.name = Some(conn_name.to_string());
        self.state.conn.db_type = Some(adapter.db_type());
        {
            let idx = self
                .state
                .engine
                .metadata_indexes
                .entry(conn_name.to_string())
                .or_default();
            idx.clear();
            idx.set_db_type(adapter.db_type());
        }

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
            match &self.state.sidebar.tree[idx] {
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
        let mut needs_render = true;
        loop {
            if needs_render {
                terminal.draw(|frame| {
                    layout::render(frame, &mut self.state, &self.theme);
                })?;
                needs_render = false;
            }

            // Set cursor shape based on editor mode (beam for Insert, block otherwise)
            {
                use crossterm::cursor::SetCursorStyle;
                let grid_editing = self
                    .state
                    .active_tab()
                    .is_some_and(|t| t.grid_editing.is_some());
                let style = if let Some(tab) = self.state.active_tab()
                    && let Some(editor) = tab.active_editor()
                {
                    if editor.pending_replace {
                        SetCursorStyle::SteadyUnderScore
                    } else {
                        match editor.mode {
                            vimltui::VimMode::Replace => SetCursorStyle::SteadyUnderScore,
                            vimltui::VimMode::Insert => SetCursorStyle::SteadyBar,
                            _ => {
                                if grid_editing {
                                    SetCursorStyle::SteadyBar
                                } else {
                                    SetCursorStyle::SteadyBlock
                                }
                            }
                        }
                    }
                } else if grid_editing {
                    SetCursorStyle::SteadyBar
                } else {
                    SetCursorStyle::SteadyBlock
                };
                let _ = crossterm::execute!(terminal.backend_mut(), style);
            }

            while let Ok(msg) = self.msg_rx.try_recv() {
                self.handle_message(msg);
                needs_render = true;
            }

            // Auto-refresh tick: scan every tab's active result_tab and
            // dispatch a re-execute if its auto_refresh interval has
            // elapsed and no other query is currently in flight on that
            // tab. Done after message processing so a refresh that just
            // landed extends `next_at` based on the new instant.
            self.tick_auto_refresh();

            // Check if leader key has been pending for >1s → show help popup
            self.check_leader_help_timeout();

            // Keep rendering while loading spinner is active or initial connect
            if self.state.loading || self.state.tabs.iter().any(|t| t.streaming) {
                needs_render = true;
            }

            if let Some(input) = events::poll_event(Duration::from_millis(50)) {
                needs_render = true;
                // Any key press hides the leader help popup
                if self.state.leader.help_visible {
                    self.state.leader.help_visible = false;
                }

                let action = match input {
                    events::InputEvent::Key(key) => events::handle_key(&mut self.state, key),
                    events::InputEvent::Paste(text) => {
                        self.handle_paste(&text);
                        events::Action::Render
                    }
                };
                if matches!(action, Action::Quit) {
                    break;
                }
                self.dispatch_action(action);

                // Drain any pending server diagnostic request set by the
                // editor handler (which can't return two actions at once).
                if let Some((sql, conn_name)) = self.state.engine.pending_server_diag.take() {
                    self.spawn_server_diagnostics(&conn_name, sql);
                }
            }
        }
        Ok(())
    }

    fn check_leader_help_timeout(&mut self) {
        // Sub-menus appear immediately
        if self.state.leader.b_pending
            || self.state.leader.w_pending
            || self.state.leader.s_pending
            || self.state.leader.f_pending
            || self.state.leader.q_pending
            || self.state.leader.leader_pending
        {
            self.state.leader.help_visible = true;
            return;
        }
        // Root leader menu appears immediately
        if self.state.leader.pending {
            self.state.leader.help_visible = true;
            return;
        }
        // No leader pending → hide
        if self.state.leader.help_visible {
            self.state.leader.help_visible = false;
        }
    }

    fn open_script_conn_picker(&mut self) {
        let connected: std::collections::HashSet<String> = self.adapters.keys().cloned().collect();

        let active: Vec<String> = connected.iter().cloned().collect();
        let others: Vec<String> = self
            .state
            .dialogs
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

        self.state.dialogs.script_conn_picker = Some(picker);
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

        // If this connection has no metadata loaded yet, trigger schema loading
        // so completion works immediately without manually expanding the sidebar
        let needs_metadata = self
            .state
            .engine
            .metadata_indexes
            .get(conn_name)
            .is_none_or(|idx| idx.all_schemas().is_empty());
        if needs_metadata && self.adapters.contains_key(conn_name) {
            self.spawn_load_schemas(conn_name);
        }

        self.state.status_message = format!("Script → {conn_name}");
        self.refresh_active_diagnostics();
    }
    fn open_template_script(&mut self, conn_name: &str, schema: &str, obj_type: &str) {
        let db_type = self.state.conn.db_type;
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
    /// Re-run diagnostics on the active editor to clear stale results.
    fn refresh_active_diagnostics(&mut self) {
        // Skip diagnostics for source tabs (PL/SQL) — sqlparser doesn't understand them
        if let Some(tab) = self.state.active_tab()
            && matches!(
                tab.kind,
                TabKind::Package { .. }
                    | TabKind::Function { .. }
                    | TabKind::Procedure { .. }
                    | TabKind::DbType { .. }
                    | TabKind::Trigger { .. }
            )
        {
            self.state.engine.diagnostics.clear();
            return;
        }

        let lines = self
            .state
            .active_tab()
            .and_then(|t| t.active_editor().map(|e| e.lines.clone()));
        if let Some(lines) = lines {
            let eff_conn = self
                .state
                .active_tab()
                .and_then(|t| t.kind.conn_name().map(|s| s.to_string()))
                .or_else(|| self.state.conn.name.clone());
            let empty_idx = crate::sql_engine::metadata::MetadataIndex::new();
            let metadata_idx = eff_conn
                .as_ref()
                .and_then(|cn| self.state.engine.metadata_indexes.get(cn))
                .unwrap_or(&empty_idx);
            let db_type = metadata_idx.db_type();
            let dialect_box = db_type
                .map(crate::sql_engine::dialect::dialect_for)
                .unwrap_or_else(|| Box::new(crate::sql_engine::dialect::OracleDialect));
            let provider = crate::sql_engine::diagnostics::DiagnosticProvider::new(
                dialect_box.as_ref(),
                metadata_idx,
            );
            let engine_diags = provider.check_local(&lines);
            self.state.engine.diagnostics = engine_diags
                .into_iter()
                .map(crate::ui::diagnostics::Diagnostic::from_engine)
                .collect();

            // Also trigger server-side compile check (Pass 4).
            if let Some(conn_name) = eff_conn {
                let sql = lines.join("\n");
                if !sql.trim().is_empty() {
                    self.spawn_server_diagnostics(&conn_name, sql);
                }
            }
        }
    }

    /// Cancel any active streaming on the current tab.
    /// Walk every tab and re-execute any active result_tab whose
    /// `auto_refresh` interval has elapsed. Skips tabs that are still
    /// streaming the previous run, so a slow query can't pile up
    /// concurrent refreshes.
    fn tick_auto_refresh(&mut self) {
        let now = std::time::Instant::now();
        // Collect (tab_id, query, start_line) tuples first to avoid
        // borrowing issues — we need a mutable App below to dispatch.
        let mut to_run: Vec<(crate::ui::tabs::TabId, String, usize)> = Vec::new();
        for tab in &mut self.state.tabs {
            if tab.streaming {
                continue;
            }
            let idx = tab.active_result_idx;
            let Some(rt) = tab.result_tabs.get_mut(idx) else {
                continue;
            };
            let Some(ar) = rt.auto_refresh.as_mut() else {
                continue;
            };
            if ar.in_flight || now < ar.next_at {
                continue;
            }
            ar.in_flight = true;
            ar.next_at = now + ar.interval;
            to_run.push((tab.id, rt.source_query.clone(), rt.source_start_line));
        }
        for (tab_id, query, start_line) in to_run {
            if let Some(tab) = self.state.find_tab_mut(tab_id) {
                tab.streaming = true;
                tab.streaming_since = Some(now);
                tab.first_batch_pending = true;
                tab.pending_query = Some((query.clone(), start_line));
            }
            self.spawn_execute_query_at(tab_id, &query, false, start_line);
        }
    }

    fn abort_active_streaming(&mut self) {
        if let Some(tab) = self.state.active_tab_mut()
            && tab.streaming
        {
            if let Some(handle) = tab.streaming_abort.take() {
                handle.abort();
            }
            tab.streaming = false;
            tab.streaming_since = None;
            tab.first_batch_pending = false;
            tab.pending_query = None;
        }
    }
}
