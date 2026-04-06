//! UI diagnostic type for rendering error underlines.
//! The diagnostic engine itself lives in `sql_engine::diagnostics`.

/// A single diagnostic (error/warning on a specific range).
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub row: usize,
    pub col_start: usize,
    pub col_end: usize,
    pub message: String,
}
