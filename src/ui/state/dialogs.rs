use crate::core::models::*;

use super::connection::ConnectionFormState;
use super::scripts::{BindVariablesState, ScriptConnPicker, ThemePickerState};

// ---------------------------------------------------------------------------
// Export / Import dialog state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportField {
    Path,
    IncludeCredentials,
    ShowPassword,
    Password,
    Confirm,
}

#[derive(Debug, Clone)]
pub struct ExportDialogState {
    pub path: String,
    pub include_credentials: bool,
    pub show_password: bool,
    pub password: String,
    pub confirm: String,
    pub focused: ExportField,
    pub error: Option<String>,
    pub path_completions: Vec<String>,
    pub completion_idx: usize,
}

impl ExportDialogState {
    pub fn new() -> Self {
        // Default path: ~/dbtui_export_{date}.dbx
        let date = {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let days = secs / 86400;
            // Simple date approximation (good enough for filename)
            let y = 1970 + (days / 365);
            let rem = days % 365;
            let m = rem / 30 + 1;
            let d = rem % 30 + 1;
            format!("{y}-{m:02}-{d:02}")
        };
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Self {
            path: format!("{home}/dbtui_export_{date}.dbx"),
            include_credentials: true,
            show_password: false,
            password: String::new(),
            confirm: String::new(),
            focused: ExportField::Path,
            error: None,
            path_completions: Vec::new(),
            completion_idx: 0,
        }
    }

    pub fn complete_path(&mut self) {
        complete_path_field(
            &mut self.path,
            &mut self.path_completions,
            &mut self.completion_idx,
        );
    }

    pub fn reset_completions(&mut self) {
        self.path_completions.clear();
        self.completion_idx = 0;
    }

    pub fn next_field(&mut self) {
        self.focused = match self.focused {
            ExportField::Path => ExportField::IncludeCredentials,
            ExportField::IncludeCredentials => ExportField::ShowPassword,
            ExportField::ShowPassword => ExportField::Password,
            ExportField::Password => ExportField::Confirm,
            ExportField::Confirm => ExportField::Path,
        };
    }

    pub fn prev_field(&mut self) {
        self.focused = match self.focused {
            ExportField::Path => ExportField::Confirm,
            ExportField::IncludeCredentials => ExportField::Path,
            ExportField::ShowPassword => ExportField::IncludeCredentials,
            ExportField::Password => ExportField::ShowPassword,
            ExportField::Confirm => ExportField::Password,
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportField {
    Path,
    ShowPassword,
    Password,
}

#[derive(Debug, Clone)]
pub struct ImportDialogState {
    pub path: String,
    pub show_password: bool,
    pub password: String,
    pub focused: ImportField,
    pub error: Option<String>,
    pub path_completions: Vec<String>,
    pub completion_idx: usize,
}

impl ImportDialogState {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Self {
            path: format!("{home}/"),
            show_password: false,
            password: String::new(),
            focused: ImportField::Path,
            error: None,
            path_completions: Vec::new(),
            completion_idx: 0,
        }
    }

    pub fn next_field(&mut self) {
        self.focused = match self.focused {
            ImportField::Path => ImportField::ShowPassword,
            ImportField::ShowPassword => ImportField::Password,
            ImportField::Password => ImportField::Path,
        };
    }

    pub fn complete_path(&mut self) {
        complete_path_field(
            &mut self.path,
            &mut self.path_completions,
            &mut self.completion_idx,
        );
    }

    pub fn reset_completions(&mut self) {
        self.path_completions.clear();
        self.completion_idx = 0;
    }
}

/// Expand a leading `~` or `~/` to the user's home directory. Paths without
/// a leading tilde are returned unchanged. If `$HOME` isn't set the input is
/// returned verbatim (unusual but the safest fallback). Used by export/import
/// so the user can type `~/foo.dbx` and have it resolve correctly.
pub fn expand_user_path(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::PathBuf::from(home).join(rest);
        }
    } else if path == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return std::path::PathBuf::from(home);
    }
    std::path::PathBuf::from(path)
}

/// Shared path completion logic for export/import dialogs.
/// Scans the filesystem and cycles through matches on repeated Tab.
fn complete_path_field(path: &mut String, completions: &mut Vec<String>, idx: &mut usize) {
    // Expand `~` before doing anything with the path so tab-completion and
    // the eventual fs::read also work when the user types a home-relative
    // path. The expanded form replaces the editable string so the dialog
    // shows an absolute path after expansion (matches shell behavior).
    let expanded = expand_user_path(path);
    let expanded_str = expanded.to_string_lossy().into_owned();
    if expanded_str != *path {
        *path = expanded_str;
    }
    let p = std::path::Path::new(path.as_str());

    let (dir, prefix) = if path.ends_with('/') {
        (std::path::PathBuf::from(path.as_str()), String::new())
    } else {
        let parent = p.parent().unwrap_or(std::path::Path::new("/"));
        let file_prefix = p
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        (parent.to_path_buf(), file_prefix)
    };

    // If we already have completions, cycle through them
    if !completions.is_empty() {
        *idx = (*idx + 1) % completions.len();
        *path = completions[*idx].clone();
        return;
    }

    // Scan directory
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let prefix_lower = prefix.to_lowercase();
    let mut matches: Vec<String> = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !prefix.is_empty() && !name.to_lowercase().starts_with(&prefix_lower) {
            continue;
        }
        if name.starts_with('.') {
            continue;
        }
        let full = dir.join(&name);
        let display = if full.is_dir() {
            format!("{}/", full.display())
        } else {
            full.display().to_string()
        };
        matches.push(display);
    }

    matches.sort();

    if matches.is_empty() {
        return;
    }

    if matches.len() == 1 {
        *path = matches[0].clone();
        completions.clear();
        *idx = 0;
        return;
    }

    *path = matches[0].clone();
    *completions = matches;
    *idx = 0;
}

// --- Dialog State ---

pub struct DialogState {
    pub connection_form: ConnectionFormState,
    pub conn_menu: super::connection::ConnMenuState,
    pub script_conn_picker: Option<ScriptConnPicker>,
    pub theme_picker: ThemePickerState,
    pub saved_connections: Vec<ConnectionConfig>,

    // Connection group state
    pub group_menu: super::connection::GroupMenuState,
    pub group_renaming: Option<String>, // Some(original_name) when renaming a group
    pub group_rename_buf: String,
    pub group_creating: bool, // true when creating a new group

    // Inline connection rename (oil-style, no modal)
    pub conn_renaming: Option<String>, // Some(original_name) when renaming a connection
    pub conn_rename_buf: String,

    // Bind variables prompt state
    pub bind_variables: Option<BindVariablesState>,

    // Export/Import dialog state
    pub export_dialog: Option<ExportDialogState>,
    pub import_dialog: Option<ImportDialogState>,

    /// Experimental oil-style inline connection editor (Proposal D).
    /// When `Some`, takes precedence over the sidebar event handler and
    /// renders a floating buffer-like editor for connection fields.
    pub inline_conn_editor: Option<InlineConnEditor>,
}

impl DialogState {
    pub fn new() -> Self {
        Self {
            connection_form: ConnectionFormState::new(),
            conn_menu: super::connection::ConnMenuState {
                conn_name: String::new(),
                cursor: 0,
                is_connected: false,
            },
            script_conn_picker: None,
            theme_picker: ThemePickerState { cursor: 0 },
            saved_connections: vec![],
            group_menu: super::connection::GroupMenuState {
                group_name: String::new(),
                cursor: 0,
                is_empty: false,
            },
            group_renaming: None,
            group_rename_buf: String::new(),
            group_creating: false,
            conn_renaming: None,
            conn_rename_buf: String::new(),
            bind_variables: None,
            export_dialog: None,
            import_dialog: None,
            inline_conn_editor: None,
        }
    }
}

// --- Inline connection editor (Proposal D — experimental) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineConnMode {
    Normal,
    Insert,
}

/// In-buffer connection editor — the oil-style alternative to the
/// full-screen `ConnectionDialog` modal. Each field is rendered as one
/// line; `j/k` moves between lines in Normal mode; `i` enters Insert mode
/// on the current field; `Enter` in Normal saves + connects; `Esc` in
/// Normal cancels. Marked experimental — bound under a dedicated
/// `<leader>I` action so it can be toggled on/off independently of the
/// regular ConnectionDialog flow.
pub struct InlineConnEditor {
    pub mode: InlineConnMode,
    pub cursor_row: usize,
    pub db_type_idx: usize,
    pub name: String,
    pub host: String,
    pub port: String,
    pub username: String,
    pub password: String,
    pub database: String,
    pub group: String,
    pub group_options: Vec<String>,
    pub password_visible: bool,
    pub error_message: String,
    pub connecting: bool,
    pub connecting_since: Option<std::time::Instant>,
}

/// Visual row order (used by both render and navigation). Each entry is
/// a logical field id that maps onto one `InlineConnEditor` field.
pub const INLINE_CONN_ROWS: [InlineConnField; 8] = [
    InlineConnField::Type,
    InlineConnField::Name,
    InlineConnField::Host,
    InlineConnField::Port,
    InlineConnField::Username,
    InlineConnField::Password,
    InlineConnField::Database,
    InlineConnField::Group,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineConnField {
    Type,
    Name,
    Host,
    Port,
    Username,
    Password,
    Database,
    Group,
}

impl InlineConnField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Name => "name",
            Self::Host => "host",
            Self::Port => "port",
            Self::Username => "user",
            Self::Password => "pass",
            Self::Database => "db",
            Self::Group => "group",
        }
    }

    /// True if this row is a free-text field (accepts typed chars in Insert).
    pub fn is_text(self) -> bool {
        matches!(
            self,
            Self::Name | Self::Host | Self::Port | Self::Username | Self::Password | Self::Database
        )
    }
}

impl InlineConnEditor {
    pub fn new(group_options: Vec<String>) -> Self {
        let group = group_options
            .first()
            .cloned()
            .unwrap_or_else(|| "Default".to_string());
        Self {
            mode: InlineConnMode::Normal,
            cursor_row: 0,
            db_type_idx: 0,
            name: String::new(),
            host: "localhost".to_string(),
            port: "5432".to_string(),
            username: String::new(),
            password: String::new(),
            database: String::new(),
            group,
            group_options,
            password_visible: false,
            error_message: String::new(),
            connecting: false,
            connecting_since: None,
        }
    }

    pub fn current_field(&self) -> InlineConnField {
        INLINE_CONN_ROWS[self.cursor_row.min(INLINE_CONN_ROWS.len() - 1)]
    }

    pub fn move_down(&mut self) {
        if self.cursor_row + 1 < INLINE_CONN_ROWS.len() {
            self.cursor_row += 1;
        }
    }

    pub fn move_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
        }
    }

    pub fn field_value_mut(&mut self, field: InlineConnField) -> Option<&mut String> {
        match field {
            InlineConnField::Name => Some(&mut self.name),
            InlineConnField::Host => Some(&mut self.host),
            InlineConnField::Port => Some(&mut self.port),
            InlineConnField::Username => Some(&mut self.username),
            InlineConnField::Password => Some(&mut self.password),
            InlineConnField::Database => Some(&mut self.database),
            _ => None,
        }
    }

    pub fn db_type_label(&self) -> &'static str {
        match self.db_type_idx {
            0 => "postgres",
            1 => "mysql",
            2 => "oracle",
            _ => "postgres",
        }
    }

    pub fn cycle_db_type(&mut self) {
        self.db_type_idx = (self.db_type_idx + 1) % 3;
        self.port = match self.db_type_idx {
            0 => "5432".to_string(),
            1 => "3306".to_string(),
            2 => "1521".to_string(),
            _ => "5432".to_string(),
        };
    }

    pub fn cycle_group(&mut self) {
        if self.group_options.is_empty() {
            return;
        }
        let cur = self
            .group_options
            .iter()
            .position(|g| g == &self.group)
            .unwrap_or(0);
        self.group = self.group_options[(cur + 1) % self.group_options.len()].clone();
    }

    pub fn to_config(&self) -> ConnectionConfig {
        let db_type = match self.db_type_idx {
            1 => DatabaseType::MySQL,
            2 => DatabaseType::Oracle,
            _ => DatabaseType::PostgreSQL,
        };
        ConnectionConfig {
            name: self.name.clone(),
            db_type,
            host: self.host.clone(),
            port: self.port.parse().unwrap_or(5432),
            username: self.username.clone(),
            password: self.password.clone(),
            database: if self.database.is_empty() {
                None
            } else {
                Some(self.database.clone())
            },
            group: if self.group.is_empty() {
                "Default".to_string()
            } else {
                self.group.clone()
            },
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.trim().is_empty() {
            return Err("name is required");
        }
        if self.host.trim().is_empty() {
            return Err("host is required");
        }
        if self.username.trim().is_empty() {
            return Err("user is required");
        }
        if self.port.parse::<u16>().is_err() {
            return Err("port must be a number");
        }
        Ok(())
    }
}
