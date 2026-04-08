mod connections;
mod messages;
mod persistence;
mod spawns;
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
            }

            // Check if leader key has been pending for >1s → show help popup
            self.check_leader_help_timeout();

            if let Some(input) = events::poll_event(Duration::from_millis(50)) {
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
                match action {
                    Action::Quit => break,
                    Action::Render | Action::None => {}
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
                        // Check if already confirmed (overlay just closed)
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
                        // On-demand load tables and views for a schema
                        // (triggered when typing "schema." in the editor)
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
                    Action::ExportBundle => {
                        self.handle_export();
                    }
                    Action::ImportBundle => {
                        self.handle_import();
                    }
                }
            }
        }
        Ok(())
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
        }
    }

    /// Cancel any active streaming on the current tab.
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

    fn handle_close_tab(&mut self) {
        // Context-aware close:
        //   - if focus is on a result tab → close that result tab
        //   - if a query is currently streaming (even from Editor focus) →
        //     cancel it and clear the loading placeholder / partial result tab
        //   - otherwise fall through to closing the workspace tab
        let (on_results, is_streaming) = self
            .state
            .active_tab()
            .map(|t| {
                (
                    matches!(t.sub_focus, crate::ui::tabs::SubFocus::Results)
                        && !t.result_tabs.is_empty(),
                    t.streaming,
                )
            })
            .unwrap_or((false, false));
        let close_result = on_results || is_streaming;

        if close_result {
            self.abort_active_streaming();
            if let Some(tab) = self.state.active_tab_mut() {
                if !tab.result_tabs.is_empty() {
                    let idx = tab.active_result_idx;
                    tab.result_tabs.remove(idx);
                    if tab.result_tabs.is_empty() {
                        tab.active_result_idx = 0;
                        tab.query_result = None;
                        tab.grid_focused = false;
                        tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
                    } else if idx >= tab.result_tabs.len() {
                        tab.active_result_idx = tab.result_tabs.len() - 1;
                    }
                } else {
                    // Pure loading placeholder (streaming with no batches yet).
                    tab.query_result = None;
                    tab.grid_focused = false;
                    tab.sub_focus = crate::ui::tabs::SubFocus::Editor;
                }
            }
            self.state.status_message = "Query cancelled".to_string();
            return;
        }

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
            self.abort_active_streaming();
            self.state.close_active_tab();
        }
    }

    /// Create a vertical split. Clones the active tab as an independent instance
    /// (new TabId, separate state) into the new (right) group.
    fn handle_create_split(&mut self) {
        use crate::ui::state::TabGroup;

        if self.state.groups.is_some() {
            return; // Already split — max 2 groups
        }
        if self.state.tabs.is_empty() {
            return;
        }

        let all_ids: Vec<_> = self.state.tabs.iter().map(|t| t.id).collect();
        let active_idx = self.state.active_tab_idx;

        // Clone the active tab as an independent instance with a new TabId
        let new_id = self.state.alloc_tab_id();
        let cloned_tab = self.state.tabs[active_idx].clone_for_split(new_id);
        self.state.tabs.push(cloned_tab);

        let g0 = TabGroup::new(all_ids, active_idx);
        let g1 = TabGroup::new(vec![new_id], 0);

        self.state.groups = Some([g0, g1]);
        self.state.active_group = 1;
        self.state.sync_active_tab_idx();
        self.state.status_message = "Split created".to_string();
    }

    /// Close the focused group: kill the active tab in the focused group and move
    /// all OTHER tabs from the focused group into the surviving group. Then exit
    /// split mode. When there's no split, falls back to closing the active tab.
    fn handle_close_group(&mut self) {
        let groups = match self.state.groups.take() {
            Some(g) => g,
            None => {
                // No split — just close the active tab
                self.handle_close_tab();
                return;
            }
        };

        let closed = self.state.active_group;
        let mut surviving = groups[1 - closed].clone();
        let closed_group = &groups[closed];
        let active_id = closed_group.active_tab_id();

        // Move all non-active tabs from the closed group into the surviving group
        // (skip ones already in surviving to avoid duplicates).
        for id in &closed_group.tab_ids {
            if Some(*id) == active_id {
                continue; // skip the active tab — it gets killed
            }
            if !surviving.tab_ids.contains(id) {
                surviving.tab_ids.push(*id);
            }
        }

        // Kill the active tab from state.tabs (only if no other group still references it)
        if let Some(id) = active_id
            && !surviving.tab_ids.contains(&id)
            && let Some(idx) = self.state.tabs.iter().position(|t| t.id == id)
        {
            self.state.tabs.remove(idx);
        }

        // Reorder state.tabs to match surviving group order
        let mut new_tabs = Vec::with_capacity(surviving.tab_ids.len());
        for id in &surviving.tab_ids {
            if let Some(pos) = self.state.tabs.iter().position(|t| t.id == *id) {
                new_tabs.push(self.state.tabs.remove(pos));
            }
        }
        new_tabs.append(&mut self.state.tabs);
        self.state.tabs = new_tabs;
        self.state.active_tab_idx = surviving
            .active_idx
            .min(self.state.tabs.len().saturating_sub(1));
        self.state.active_group = 0;

        if self.state.tabs.is_empty() {
            self.state.focus = crate::ui::state::Focus::Sidebar;
        }
        self.state.status_message = "Group closed".to_string();
    }

    /// Move the focused group's active tab to the other group.
    fn handle_move_tab_to_other(&mut self) {
        let groups = match self.state.groups.as_mut() {
            Some(g) => g,
            None => return,
        };

        let from = self.state.active_group;
        let to = 1 - from;

        // Get tab to move
        let moving_id = match groups[from].active_tab_id() {
            Some(id) => id,
            None => return,
        };

        // Remove from source group
        if let Some(pos) = groups[from].tab_ids.iter().position(|id| *id == moving_id) {
            groups[from].tab_ids.remove(pos);
            if groups[from].active_idx >= groups[from].tab_ids.len()
                && !groups[from].tab_ids.is_empty()
            {
                groups[from].active_idx = groups[from].tab_ids.len() - 1;
            }
        }

        // Add to destination if not already there
        if !groups[to].tab_ids.contains(&moving_id) {
            groups[to].tab_ids.push(moving_id);
            groups[to].active_idx = groups[to].tab_ids.len() - 1;
        }

        // If source group is now empty, destroy split
        if groups[from].tab_ids.is_empty() {
            let surviving = groups[to].clone();
            self.state.groups = None;
            self.state.active_group = 0;
            // Reorder tabs to match surviving group order
            let mut new_tabs = Vec::with_capacity(surviving.tab_ids.len());
            for id in &surviving.tab_ids {
                if let Some(pos) = self.state.tabs.iter().position(|t| t.id == *id) {
                    new_tabs.push(self.state.tabs.remove(pos));
                }
            }
            new_tabs.append(&mut self.state.tabs);
            self.state.tabs = new_tabs;
            self.state.active_tab_idx = surviving
                .active_idx
                .min(self.state.tabs.len().saturating_sub(1));
            self.state.status_message = "Tab moved (split closed)".to_string();
        } else {
            // Switch focus to destination group
            self.state.active_group = to;
            self.state.sync_active_tab_idx();
            self.state.status_message = "Tab moved".to_string();
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
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    if let Some(editor) = tab.editor.as_mut() {
                        editor.set_content(&content);
                    }
                    tab.mark_saved();
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
                        // Creating a collection. If the cursor is inside an
                        // existing collection, the new one is nested —
                        // `parent/child` on disk, which `list_tree()` now
                        // walks recursively.
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

        let (conn_name, obj_schema, obj_name, obj_type) = match extract_source_info(tab) {
            Some((cn, schema, _content, ot)) => {
                let name = tab.kind.display_name().to_string();
                (cn, schema, name, ot)
            }
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

        let obj_schema = obj_schema.clone();
        let obj_name = obj_name.clone();
        let obj_type = obj_type.clone();

        tokio::spawn(async move {
            let part_names: Vec<&str> = if sql_statements.len() > 1 {
                vec!["DECLARATION", "BODY"]
            } else {
                vec!["SOURCE"]
            };

            // Compile each statement directly (bypass validator for PL/SQL)
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
                if matches!(db_type, DatabaseType::Oracle) {
                    let oracle_type = match part_names.get(idx) {
                        Some(&"BODY") => format!("{obj_type} BODY"),
                        _ => obj_type.clone(),
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
                        let _ = tx
                            .send(AppMessage::CompileResult {
                                tab_id,
                                success: false,
                                message: error_text.trim().to_string(),
                                failed_sql: sql.clone(),
                                failed_part: part_names.get(idx).unwrap_or(&"SOURCE").to_string(),
                            })
                            .await;
                        return;
                    }
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
