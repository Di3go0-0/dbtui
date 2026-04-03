use thiserror::Error;

#[allow(dead_code)]
#[derive(Error, Debug, Clone)]
pub enum DbError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Query failed: {0}")]
    QueryFailed(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Not supported: {0}")]
    NotSupported(String),

    #[error("Timeout")]
    Timeout,

    #[error("Unknown error: {0}")]
    Unknown(String),
}

#[allow(dead_code)]
#[derive(Error, Debug, Clone)]
pub enum UiError {
    #[error("Render failed: {0}")]
    RenderFailed(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("State error: {0}")]
    StateError(String),
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Db(#[from] DbError),

    #[error("UI error: {0}")]
    Ui(#[from] UiError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Storage error: {0}")]
    Storage(String),
}

pub type DbResult<T> = std::result::Result<T, DbError>;
pub type AppResult<T> = std::result::Result<T, AppError>;
