use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectPrivilege {
    Full,     // SELECT + DML, or EXECUTE
    ReadOnly, // SELECT only
    Execute,  // EXECUTE only (routines)
    Unknown,  // Not yet loaded / default
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatabaseType {
    Oracle,
    PostgreSQL,
    MySQL,
}

impl std::fmt::Display for DatabaseType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseType::Oracle => write!(f, "Oracle"),
            DatabaseType::PostgreSQL => write!(f, "PostgreSQL"),
            DatabaseType::MySQL => write!(f, "MySQL"),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

#[derive(Debug, Clone)]
pub struct Schema {
    pub name: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Table {
    pub name: String,
    pub schema: String,
    pub privilege: ObjectPrivilege,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct View {
    pub name: String,
    pub schema: String,
    pub valid: bool,
    pub privilege: ObjectPrivilege,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub schema: String,
    pub has_body: bool,
    pub valid: bool,
    pub privilege: ObjectPrivilege,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Procedure {
    pub name: String,
    pub schema: String,
    pub valid: bool,
    pub privilege: ObjectPrivilege,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub schema: String,
    pub valid: bool,
    pub privilege: ObjectPrivilege,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct MaterializedView {
    pub name: String,
    pub schema: String,
    pub valid: bool,
    pub privilege: ObjectPrivilege,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Index {
    pub name: String,
    pub schema: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Sequence {
    pub name: String,
    pub schema: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DbType {
    pub name: String,
    pub schema: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Trigger {
    pub name: String,
    pub schema: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DbEvent {
    pub name: String,
    pub schema: String,
}

#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub is_primary_key: bool,
}

#[derive(Debug, Clone)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub elapsed: Option<std::time::Duration>,
}

#[derive(Debug, Clone)]
pub struct PackageContent {
    pub declaration: String,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub name: String,
    pub db_type: DatabaseType,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: Option<String>,
    #[serde(default = "default_group")]
    pub group: String,
}

fn default_group() -> String {
    "Default".to_string()
}
