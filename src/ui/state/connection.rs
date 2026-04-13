use crate::core::models::*;

/// Info about an object pending drop/rename
#[derive(Debug, Clone)]
pub struct PendingObjectAction {
    pub schema: String,
    pub name: String,
    pub obj_type: String, // "TABLE", "VIEW", "PACKAGE"
    pub conn_name: String,
}

pub struct GroupMenuState {
    pub group_name: String,
    pub cursor: usize,
    pub is_empty: bool, // true if the group has no connections
}

#[derive(Clone)]
pub enum GroupMenuAction {
    Rename,
    Delete,
    NewGroup,
}

impl GroupMenuAction {
    pub fn all() -> Vec<Self> {
        vec![Self::Rename, Self::Delete, Self::NewGroup]
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Rename => "Rename group",
            Self::Delete => "Delete group",
            Self::NewGroup => "New group",
        }
    }

    pub fn icon(&self) -> &str {
        match self {
            Self::Rename => "✎",
            Self::Delete => "✗",
            Self::NewGroup => "+",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnMenuAction {
    View,
    Edit,
    Connect,
    Disconnect,
    Restart,
    Delete,
}

impl ConnMenuAction {
    pub fn all() -> Vec<Self> {
        vec![
            Self::View,
            Self::Edit,
            Self::Connect,
            Self::Disconnect,
            Self::Restart,
            Self::Delete,
        ]
    }

    pub fn label(&self) -> &str {
        match self {
            Self::View => "View connection info",
            Self::Edit => "Edit connection",
            Self::Connect => "Connect",
            Self::Disconnect => "Disconnect",
            Self::Restart => "Restart connection",
            Self::Delete => "Delete connection",
        }
    }

    pub fn icon(&self) -> &str {
        match self {
            Self::View => "👁",
            Self::Edit => "✎",
            Self::Connect => "●",
            Self::Disconnect => "○",
            Self::Restart => "↻",
            Self::Delete => "✗",
        }
    }
}

pub struct ConnMenuState {
    pub conn_name: String,
    pub cursor: usize,
    pub is_connected: bool,
}

// --- Connection Form State ---

/// Visual order used by the Connection dialog (Proposal B). The
/// underlying field indices 0..=7 stay stable — this array just defines
/// the order the user sees and tabs through:
///   Name → Type → Group → Host → Port → Database → Username → Password
pub const CONN_FIELD_VISUAL_ORDER: [usize; 8] = [0, 1, 7, 2, 3, 6, 4, 5];

pub struct ConnectionFormState {
    pub name: String,
    pub db_type_idx: usize,
    pub host: String,
    pub port: String,
    pub username: String,
    pub password: String,
    pub database: String,
    pub group: String,
    pub group_options: Vec<String>, // available groups for cycling
    pub selected_field: usize,
    pub error_message: String,
    pub password_visible: bool,
    pub connecting: bool,
    /// When `connecting` is set, track the instant so the dialog can show
    /// an elapsed-time spinner while the adapter negotiates the handshake.
    pub connecting_since: Option<std::time::Instant>,
    pub show_saved_list: bool,
    pub saved_cursor: usize,
    pub editing_name: Option<String>,
    pub read_only: bool,
}

impl ConnectionFormState {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            db_type_idx: 0,
            host: "localhost".to_string(),
            port: "5432".to_string(),
            username: String::new(),
            password: String::new(),
            database: String::new(),
            group: "Default".to_string(),
            group_options: vec!["Default".to_string()],
            selected_field: 0,
            error_message: String::new(),
            password_visible: false,
            connecting: false,
            connecting_since: None,
            show_saved_list: false,
            saved_cursor: 0,
            editing_name: None,
            read_only: false,
        }
    }

    pub fn db_type_label(&self) -> &str {
        match self.db_type_idx {
            0 => "PostgreSQL",
            1 => "MySQL",
            2 => "Oracle",
            _ => "PostgreSQL",
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

    pub fn to_connection_config(&self) -> ConnectionConfig {
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

    pub fn from_config(config: &ConnectionConfig) -> Self {
        let db_type_idx = match config.db_type {
            DatabaseType::PostgreSQL => 0,
            DatabaseType::MySQL => 1,
            DatabaseType::Oracle => 2,
        };
        Self {
            name: config.name.clone(),
            db_type_idx,
            host: config.host.clone(),
            port: config.port.to_string(),
            username: config.username.clone(),
            password: config.password.clone(),
            database: config.database.clone().unwrap_or_default(),
            group: config.group.clone(),
            group_options: vec!["Default".to_string()],
            selected_field: 0,
            error_message: String::new(),
            password_visible: false,
            connecting: false,
            connecting_since: None,
            show_saved_list: false,
            saved_cursor: 0,
            editing_name: None,
            read_only: false,
        }
    }

    pub fn for_edit(config: &ConnectionConfig) -> Self {
        let mut form = Self::from_config(config);
        form.editing_name = Some(config.name.clone());
        form
    }

    pub fn active_field_mut(&mut self) -> &mut String {
        match self.selected_field {
            0 => &mut self.name,
            1 => &mut self.name, // db_type (handled separately via Ctrl+T)
            2 => &mut self.host,
            3 => &mut self.port,
            4 => &mut self.username,
            5 => &mut self.password,
            6 => &mut self.database,
            7 => &mut self.group, // group (handled separately via Ctrl+G)
            _ => &mut self.name,
        }
    }

    /// Cycle through available groups (Ctrl+G)
    pub fn cycle_group(&mut self) {
        if self.group_options.is_empty() {
            return;
        }
        let current_idx = self
            .group_options
            .iter()
            .position(|g| g == &self.group)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % self.group_options.len();
        self.group = self.group_options[next_idx].clone();
    }

    pub fn next_field(&mut self) {
        let cur = CONN_FIELD_VISUAL_ORDER
            .iter()
            .position(|&f| f == self.selected_field)
            .unwrap_or(0);
        let next = (cur + 1) % CONN_FIELD_VISUAL_ORDER.len();
        self.selected_field = CONN_FIELD_VISUAL_ORDER[next];
    }

    pub fn prev_field(&mut self) {
        let cur = CONN_FIELD_VISUAL_ORDER
            .iter()
            .position(|&f| f == self.selected_field)
            .unwrap_or(0);
        let prev = if cur == 0 {
            CONN_FIELD_VISUAL_ORDER.len() - 1
        } else {
            cur - 1
        };
        self.selected_field = CONN_FIELD_VISUAL_ORDER[prev];
    }
}

impl Default for ConnectionFormState {
    fn default() -> Self {
        Self::new()
    }
}

// --- Connection State ---

pub struct ConnectionState {
    pub name: Option<String>,
    pub db_type: Option<DatabaseType>,
    pub current_schema: Option<String>,
    pub connected: bool,
}

impl ConnectionState {
    pub fn new() -> Self {
        Self {
            name: None,
            db_type: None,
            current_schema: None,
            connected: false,
        }
    }
}
