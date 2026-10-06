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

impl Diagnostic {
    /// A server-reported error on `row`, spanning `span` (byte offsets into
    /// that line).
    pub fn server_error(row: usize, span: (usize, usize), message: String) -> Self {
        Self {
            row,
            col_start: span.0,
            col_end: span.1,
            message,
            severity: Severity::Error,
            source: Source::Server,
        }
    }
}

/// Byte span to underline on `line` for an error the server placed at the
/// 1-based character column `col`, or on the whole line when it only gave a
/// line number. With a column the span covers the token starting there, which
/// is what engines point at ("syntax error at or near …").
pub fn error_span(line: &str, col: Option<usize>) -> (usize, usize) {
    let Some(col) = col else {
        let start = line.len() - line.trim_start().len();
        let end = line.trim_end().len().max(start + 1);
        return (start, end);
    };
    let start = line
        .char_indices()
        .nth(col.saturating_sub(1))
        .map_or(line.len(), |(i, _)| i);
    (
        start,
        crate::sql_engine::diagnostics::token_end(line, start),
    )
}

/// Recompute the diagnostics shown for the tab at `tab_idx`: the local passes
/// (syntax, semantic, lint) for script buffers, plus whatever the server
/// reported for the view being shown, then refresh the gutter signs.
pub fn refresh(state: &mut crate::ui::state::AppState, tab_idx: usize) {
    let Some(tab) = state.tabs.get(tab_idx) else {
        state.engine.diagnostics.clear();
        return;
    };
    let mut diagnostics = if tab.kind.is_source_object() {
        // sqlparser cannot parse PL/SQL sources; only compile errors apply.
        Vec::new()
    } else {
        local_diagnostics(state, tab)
    };
    diagnostics.extend(tab.visible_server_diagnostics().iter().cloned());
    diagnostics.sort_by_key(|d| (d.row, d.col_start));

    state.engine.diagnostic_list_cursor = state
        .engine
        .diagnostic_list_cursor
        .min(diagnostics.len().saturating_sub(1));
    state.engine.diagnostics = diagnostics;
    state.engine.last_diagnostic_run = Some(std::time::Instant::now());
    apply_gutter_signs(state, tab_idx);
}

/// Run the local passes over a script buffer. Without a connection there is
/// no dialect to check against, so nothing is reported; with one whose
/// metadata is still loading only syntax and lint run — the semantic pass
/// would flag every table as unknown.
fn local_diagnostics(
    state: &crate::ui::state::AppState,
    tab: &crate::ui::tabs::WorkspaceTab,
) -> Vec<Diagnostic> {
    use crate::sql_engine::diagnostics::DiagnosticProvider;
    use crate::sql_engine::metadata::MetadataIndex;

    let Some(editor) = tab.active_editor() else {
        return Vec::new();
    };
    let conn_name = tab.kind.conn_name().or(state.conn.name.as_deref());
    let Some(index) = conn_name.and_then(|cn| state.engine.metadata_indexes.get(cn)) else {
        return Vec::new();
    };
    let Some(db_type) = index.db_type() else {
        return Vec::new();
    };
    let ready = conn_name.is_some_and(|cn| state.metadata_ready.contains(cn));
    let empty = MetadataIndex::new();
    let metadata = if ready { index } else { &empty };
    let dialect = crate::sql_engine::dialect::dialect_for(db_type);

    DiagnosticProvider::new(dialect.as_ref(), metadata)
        .check_local(&editor.lines)
        .into_iter()
        .map(Diagnostic::from_engine)
        .collect()
}

/// Set diagnostic signs on the tab's active editor gutter (left of the line
/// numbers), keeping the diff signs that share the gutter config.
pub fn apply_gutter_signs(state: &mut crate::ui::state::AppState, tab_idx: usize) {
    use std::collections::HashMap;

    let mut signs: HashMap<usize, vimltui::Diagnostic> = HashMap::new();
    for d in &state.engine.diagnostics {
        let severity = match d.severity {
            Severity::Error => vimltui::DiagnosticSeverity::Error,
            Severity::Warning | Severity::Info | Severity::Hint => {
                vimltui::DiagnosticSeverity::Warning
            }
        };
        signs
            .entry(d.row)
            .and_modify(|existing| {
                if severity == vimltui::DiagnosticSeverity::Error {
                    existing.severity = vimltui::DiagnosticSeverity::Error;
                    existing.message = Some(d.message.clone());
                }
            })
            .or_insert(vimltui::Diagnostic {
                severity,
                message: Some(d.message.clone()),
            });
    }

    let Some(editor) = state
        .tabs
        .get_mut(tab_idx)
        .and_then(|tab| tab.active_editor_mut())
    else {
        return;
    };
    if signs.is_empty() && editor.gutter.is_none() {
        return;
    }
    let mut config = editor.gutter.take().unwrap_or_default();
    config.diagnostics = signs;
    if !(config.signs.is_empty() && config.diagnostics.is_empty()) {
        editor.gutter = Some(config);
    }
}

#[cfg(test)]
mod tests {
    use super::error_span;

    #[test]
    fn span_covers_the_token_at_the_column() {
        assert_eq!(error_span("SELECT * FORM users", Some(10)), (9, 13));
        // A lone symbol is one character wide.
        assert_eq!(error_span("SELECT 1 + ;", Some(12)), (11, 12));
    }

    #[test]
    fn span_counts_characters_not_bytes() {
        // "ñ" is two bytes, so the byte span starts one past the column.
        let line = "SELECT 'año' frm t";
        let (start, end) = error_span(line, Some(14));
        assert_eq!(&line[start..end], "frm");
    }

    #[test]
    fn span_without_a_column_is_the_trimmed_line() {
        assert_eq!(error_span("  UPDATE t  ", None), (2, 10));
        assert_eq!(error_span("", None), (0, 1));
    }

    #[test]
    fn span_past_the_end_of_the_line_stays_in_range() {
        assert_eq!(error_span("abc", Some(40)), (3, 4));
    }
}
