//! UI diagnostic type for rendering error underlines.
//! The diagnostic engine itself lives in `sql_engine::diagnostics`.

/// Severity level — mirrors `sql_engine::diagnostics::DiagnosticSeverity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

/// Source of the diagnostic — which pass produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Syntax,
    Semantic,
    Lint,
    Server,
}

impl Source {
    pub fn label(&self) -> &str {
        match self {
            Source::Syntax => "syntax",
            Source::Semantic => "semantic",
            Source::Lint => "lint",
            Source::Server => "server",
        }
    }
}

/// A single diagnostic (error/warning on a specific range).
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub row: usize,
    pub col_start: usize,
    pub col_end: usize,
    pub message: String,
    pub severity: Severity,
    pub source: Source,
}

impl Diagnostic {
    /// Convert from engine diagnostic to UI diagnostic.
    pub fn from_engine(d: crate::sql_engine::diagnostics::Diagnostic) -> Self {
        use crate::sql_engine::diagnostics::{DiagnosticSeverity, DiagnosticSource};
        Self {
            row: d.row,
            col_start: d.col_start,
            col_end: d.col_end,
            message: d.message,
            severity: match d.severity {
                DiagnosticSeverity::Error => Severity::Error,
                DiagnosticSeverity::Warning => Severity::Warning,
                DiagnosticSeverity::Info => Severity::Info,
                DiagnosticSeverity::Hint => Severity::Hint,
            },
            source: match d.source {
                DiagnosticSource::Syntax => Source::Syntax,
                DiagnosticSource::Semantic => Source::Semantic,
                DiagnosticSource::Lint => Source::Lint,
                DiagnosticSource::Server => Source::Server,
            },
        }
    }
}
