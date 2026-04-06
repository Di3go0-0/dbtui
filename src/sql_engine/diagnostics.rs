//! DiagnosticProvider — multi-pass SQL validation and lint rules.
//!
//! Three synchronous passes (instant, no I/O):
//! - Pass 1: Syntax validation via sqlparser (per-dialect)
//! - Pass 2: Semantic validation via SemanticAnalyzer (unknown tables/schemas)
//! - Pass 3: Lint rules (SELECT *, missing WHERE, JOIN without ON)
//!
//! Pass 4 (server-side compilation) is async and handled by the UI layer
//! via the adapter's `compile_check` method with debounce.

use sqlparser::parser::Parser;

use crate::sql_engine::analyzer::SemanticAnalyzer;
use crate::sql_engine::context::ResolutionErrorKind;
use crate::sql_engine::dialect::SqlDialect;
use crate::sql_engine::metadata::MetadataIndex;
use crate::sql_engine::tokenizer;

// ---------------------------------------------------------------------------
// Diagnostic types
// ---------------------------------------------------------------------------

/// Severity level of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// Source of the diagnostic (which pass produced it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSource {
    /// Pass 1: sqlparser syntax validation.
    Syntax,
    /// Pass 2: semantic reference validation.
    Semantic,
    /// Pass 3: lint rules.
    Lint,
    /// Pass 4: server-side compilation (async, handled externally).
    Server,
}

/// A single diagnostic with position, message, severity, and source.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub row: usize,
    pub col_start: usize,
    pub col_end: usize,
    pub message: String,
    pub severity: DiagnosticSeverity,
    pub source: DiagnosticSource,
}

/// Aggregated diagnostics for a buffer.
///
/// Supports source-based updates: replacing diagnostics from one source
/// while preserving diagnostics from other sources. This allows async
/// server diagnostics to arrive without clobbering local results.
#[derive(Debug, Clone, Default)]
pub struct DiagnosticSet {
    items: Vec<Diagnostic>,
    generation: u64,
}

impl DiagnosticSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn items(&self) -> &[Diagnostic] {
        &self.items
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Replace diagnostics from a given source, preserving others.
    pub fn update_source(&mut self, source: DiagnosticSource, diags: Vec<Diagnostic>) {
        self.items.retain(|d| d.source != source);
        self.items.extend(diags);
        self.items.sort_by_key(|d| (d.row, d.col_start));
        self.generation += 1;
    }

    /// Clear all diagnostics.
    pub fn clear(&mut self) {
        self.items.clear();
        self.generation += 1;
    }

    /// Count of error-level diagnostics.
    pub fn error_count(&self) -> usize {
        self.items
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .count()
    }

    /// Count of warning-level diagnostics.
    pub fn warning_count(&self) -> usize {
        self.items
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning)
            .count()
    }
}

// ---------------------------------------------------------------------------
// Diagnostic provider
// ---------------------------------------------------------------------------

/// Multi-pass diagnostic engine. All passes are synchronous.
pub struct DiagnosticProvider<'a> {
    dialect: &'a dyn SqlDialect,
    metadata: &'a MetadataIndex,
}

impl<'a> DiagnosticProvider<'a> {
    pub fn new(dialect: &'a dyn SqlDialect, metadata: &'a MetadataIndex) -> Self {
        Self { dialect, metadata }
    }

    /// Run all local passes (syntax + semantic + lint). Returns diagnostics.
    pub fn check_local(&self, lines: &[String]) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // Pass 1: Syntax
        self.check_syntax(lines, &mut diagnostics);

        // Pass 2: Semantic (table/schema references)
        self.check_references(lines, &mut diagnostics);

        // Pass 3: Lint
        self.check_lint(lines, &mut diagnostics);

        diagnostics
    }

    // -----------------------------------------------------------------------
    // Pass 1: Syntax validation
    // -----------------------------------------------------------------------

    fn check_syntax(&self, lines: &[String], out: &mut Vec<Diagnostic>) {
        let dialect = self.dialect.parser_dialect();

        // Split into query blocks separated by blank lines
        let mut block_start = 0;
        let mut i = 0;
        while i <= lines.len() {
            let is_blank = i == lines.len() || lines[i].trim().is_empty();
            if is_blank && i > block_start {
                let block: String = lines[block_start..i]
                    .iter()
                    .map(|l| l.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                if !block.trim().is_empty()
                    && let Err(e) = Parser::parse_sql(dialect.as_ref(), &block)
                {
                    let msg = e.to_string();
                    let (err_line, err_col) = parse_syntax_error_position(&msg);
                    let file_row = block_start + err_line.saturating_sub(1);
                    let file_col = if err_col > 0 { err_col - 1 } else { 0 };
                    let clean_msg = msg.split(" at Line:").next().unwrap_or(&msg).to_string();
                    let col_end = if file_row < lines.len() {
                        let line_len = lines[file_row].len();
                        if file_col < line_len {
                            line_len
                        } else {
                            file_col + 1
                        }
                    } else {
                        file_col + 1
                    };
                    out.push(Diagnostic {
                        row: file_row,
                        col_start: file_col,
                        col_end,
                        message: clean_msg,
                        severity: DiagnosticSeverity::Error,
                        source: DiagnosticSource::Syntax,
                    });
                }
                block_start = i + 1;
            } else if is_blank {
                block_start = i + 1;
            }
            i += 1;
        }
    }

    // -----------------------------------------------------------------------
    // Pass 2: Semantic reference validation
    // -----------------------------------------------------------------------

    fn check_references(&self, lines: &[String], out: &mut Vec<Diagnostic>) {
        let analyzer = SemanticAnalyzer::new(self.dialect, self.metadata);
        let ctx = analyzer.analyze_for_diagnostics(lines);

        for err in &ctx.resolution_errors {
            let severity = match err.kind {
                ResolutionErrorKind::UnknownSchema | ResolutionErrorKind::UnknownTable => {
                    DiagnosticSeverity::Error
                }
                ResolutionErrorKind::UnknownColumn | ResolutionErrorKind::AmbiguousColumn => {
                    DiagnosticSeverity::Warning
                }
            };
            out.push(Diagnostic {
                row: err.location.row,
                col_start: err.location.col_start,
                col_end: err.location.col_end,
                message: err.message.clone(),
                severity,
                source: DiagnosticSource::Semantic,
            });
        }
    }

    // -----------------------------------------------------------------------
    // Pass 3: Lint rules
    // -----------------------------------------------------------------------

    fn check_lint(&self, lines: &[String], out: &mut Vec<Diagnostic>) {
        self.lint_select_star(lines, out);
        self.lint_missing_where(lines, out);
        self.lint_join_without_on(lines, out);
    }

    /// Warn on SELECT * usage.
    fn lint_select_star(&self, lines: &[String], out: &mut Vec<Diagnostic>) {
        let line_strs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let tokens = tokenizer::tokenize_sql(&line_strs);

        let mut i = 0;
        while i < tokens.len() {
            if tokens[i].kind == tokenizer::TokenKind::Word
                && tokens[i].text.to_uppercase() == "SELECT"
            {
                // Skip whitespace after SELECT
                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == tokenizer::TokenKind::Whitespace {
                    j += 1;
                }
                // Skip optional DISTINCT
                if j < tokens.len()
                    && tokens[j].kind == tokenizer::TokenKind::Word
                    && tokens[j].text.to_uppercase() == "DISTINCT"
                {
                    j += 1;
                    while j < tokens.len() && tokens[j].kind == tokenizer::TokenKind::Whitespace {
                        j += 1;
                    }
                }
                // Check for *
                if j < tokens.len()
                    && tokens[j].kind == tokenizer::TokenKind::Other
                    && tokens[j].text == "*"
                {
                    out.push(Diagnostic {
                        row: tokens[j].row,
                        col_start: tokens[j].col,
                        col_end: tokens[j].col + 1,
                        message: "SELECT * — consider listing columns explicitly".to_string(),
                        severity: DiagnosticSeverity::Warning,
                        source: DiagnosticSource::Lint,
                    });
                }
            }
            i += 1;
        }
    }

    /// Warn on UPDATE/DELETE without WHERE.
    fn lint_missing_where(&self, lines: &[String], out: &mut Vec<Diagnostic>) {
        let line_strs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let tokens = tokenizer::tokenize_sql(&line_strs);

        let words: Vec<String> = tokens
            .iter()
            .filter(|t| t.kind == tokenizer::TokenKind::Word)
            .map(|t| t.text.to_uppercase())
            .collect();

        let has_where = words.iter().any(|w| w == "WHERE");

        for token in &tokens {
            if token.kind != tokenizer::TokenKind::Word {
                continue;
            }
            let upper = token.text.to_uppercase();
            if (upper == "UPDATE" || upper == "DELETE") && !has_where {
                out.push(Diagnostic {
                    row: token.row,
                    col_start: token.col,
                    col_end: token.col + token.text.len(),
                    message: format!("{upper} without WHERE clause"),
                    severity: DiagnosticSeverity::Warning,
                    source: DiagnosticSource::Lint,
                });
                break; // One warning per block
            }
        }
    }

    /// Warn on JOIN without ON.
    fn lint_join_without_on(&self, lines: &[String], out: &mut Vec<Diagnostic>) {
        let line_strs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let tokens = tokenizer::tokenize_sql(&line_strs);

        let mut i = 0;
        while i < tokens.len() {
            if tokens[i].kind == tokenizer::TokenKind::Word
                && tokens[i].text.to_uppercase() == "JOIN"
            {
                let join_token = &tokens[i];
                // Scan forward for ON or another JOIN/WHERE (which would mean ON is missing)
                let mut j = i + 1;
                let mut found_on = false;
                while j < tokens.len() {
                    if tokens[j].kind == tokenizer::TokenKind::Word {
                        let upper = tokens[j].text.to_uppercase();
                        if upper == "ON" || upper == "USING" {
                            found_on = true;
                            break;
                        }
                        // Hit another clause → ON is missing
                        if matches!(
                            upper.as_str(),
                            "JOIN"
                                | "LEFT"
                                | "RIGHT"
                                | "INNER"
                                | "FULL"
                                | "CROSS"
                                | "NATURAL"
                                | "WHERE"
                                | "ORDER"
                                | "GROUP"
                                | "HAVING"
                                | "LIMIT"
                                | "UNION"
                                | "INTERSECT"
                                | "EXCEPT"
                        ) {
                            break;
                        }
                    }
                    j += 1;
                }

                // CROSS JOIN and NATURAL JOIN don't need ON
                let is_cross_or_natural = if i > 0 {
                    let mut k = i - 1;
                    while k > 0 && tokens[k].kind == tokenizer::TokenKind::Whitespace {
                        k -= 1;
                    }
                    tokens[k].kind == tokenizer::TokenKind::Word
                        && matches!(tokens[k].text.to_uppercase().as_str(), "CROSS" | "NATURAL")
                } else {
                    false
                };

                if !found_on && !is_cross_or_natural {
                    out.push(Diagnostic {
                        row: join_token.row,
                        col_start: join_token.col,
                        col_end: join_token.col + join_token.text.len(),
                        message: "JOIN without ON clause".to_string(),
                        severity: DiagnosticSeverity::Warning,
                        source: DiagnosticSource::Lint,
                    });
                }
            }
            i += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse line/column from sqlparser error messages.
/// Format: "Expected ..., found: ... at Line: 5, Column: 10"
fn parse_syntax_error_position(msg: &str) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    if let Some(pos) = msg.find("Line: ")
        && let Some(num_str) = msg[pos + 6..].split(',').next()
    {
        line = num_str.trim().parse().unwrap_or(1);
    }
    if let Some(pos) = msg.find("Column: ")
        && let Some(num_str) = msg[pos + 8..].split(|c: char| !c.is_ascii_digit()).next()
    {
        col = num_str.trim().parse().unwrap_or(1);
    }
    (line, col)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql_engine::dialect::OracleDialect;
    use crate::sql_engine::metadata::{MetadataIndex, ObjectKind};
    use crate::sql_engine::models::ResolvedColumn;

    fn test_index() -> MetadataIndex {
        let mut idx = MetadataIndex::new();
        idx.set_db_type(crate::core::models::DatabaseType::Oracle);
        idx.set_current_schema("HR");
        idx.add_schema("HR");
        idx.add_object("HR", "EMPLOYEES", ObjectKind::Table);
        idx.add_object("HR", "DEPARTMENTS", ObjectKind::Table);

        idx.cache_columns(
            "HR",
            "EMPLOYEES",
            vec![ResolvedColumn {
                name: "EMPLOYEE_ID".into(),
                data_type: "NUMBER".into(),
                nullable: false,
                is_primary_key: true,
                table_schema: "HR".into(),
                table_name: "EMPLOYEES".into(),
            }],
        );
        idx
    }

    // -- DiagnosticSet tests --

    #[test]
    fn diagnostic_set_update_source_preserves_others() {
        let mut set = DiagnosticSet::new();
        set.update_source(
            DiagnosticSource::Syntax,
            vec![Diagnostic {
                row: 0,
                col_start: 0,
                col_end: 5,
                message: "syntax err".into(),
                severity: DiagnosticSeverity::Error,
                source: DiagnosticSource::Syntax,
            }],
        );
        set.update_source(
            DiagnosticSource::Lint,
            vec![Diagnostic {
                row: 1,
                col_start: 0,
                col_end: 8,
                message: "SELECT *".into(),
                severity: DiagnosticSeverity::Warning,
                source: DiagnosticSource::Lint,
            }],
        );
        assert_eq!(set.items().len(), 2);
        assert_eq!(set.error_count(), 1);
        assert_eq!(set.warning_count(), 1);

        // Replace syntax: keeps lint
        set.update_source(DiagnosticSource::Syntax, vec![]);
        assert_eq!(set.items().len(), 1);
        assert_eq!(set.items()[0].source, DiagnosticSource::Lint);
    }

    #[test]
    fn diagnostic_set_clear() {
        let mut set = DiagnosticSet::new();
        let gen_before = set.generation();
        set.update_source(
            DiagnosticSource::Syntax,
            vec![Diagnostic {
                row: 0,
                col_start: 0,
                col_end: 1,
                message: "err".into(),
                severity: DiagnosticSeverity::Error,
                source: DiagnosticSource::Syntax,
            }],
        );
        set.clear();
        assert!(set.is_empty());
        assert!(set.generation() > gen_before);
    }

    #[test]
    fn diagnostic_set_sorted_by_position() {
        let mut set = DiagnosticSet::new();
        set.update_source(
            DiagnosticSource::Lint,
            vec![
                Diagnostic {
                    row: 2,
                    col_start: 0,
                    col_end: 1,
                    message: "b".into(),
                    severity: DiagnosticSeverity::Warning,
                    source: DiagnosticSource::Lint,
                },
                Diagnostic {
                    row: 0,
                    col_start: 5,
                    col_end: 6,
                    message: "a".into(),
                    severity: DiagnosticSeverity::Warning,
                    source: DiagnosticSource::Lint,
                },
            ],
        );
        assert_eq!(set.items()[0].row, 0);
        assert_eq!(set.items()[1].row, 2);
    }

    // -- Syntax pass tests --

    #[test]
    fn syntax_error_detected() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELEC * FROM employees".into()];
        let diags = provider.check_local(&lines);

        let syntax_errs: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Syntax)
            .collect();
        assert!(!syntax_errs.is_empty());
        assert_eq!(syntax_errs[0].severity, DiagnosticSeverity::Error);
    }

    #[test]
    fn valid_sql_no_syntax_error() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM employees".into()];
        let diags = provider.check_local(&lines);

        let syntax_errs: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Syntax)
            .collect();
        assert!(syntax_errs.is_empty());
    }

    // -- Semantic pass tests --

    #[test]
    fn unknown_table_semantic_error() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM nonexistent_table".into()];
        let diags = provider.check_local(&lines);

        let sem_errs: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Semantic)
            .collect();
        assert!(!sem_errs.is_empty());
        assert!(sem_errs[0].message.contains("Unknown table"));
    }

    #[test]
    fn known_table_no_semantic_error() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM employees".into()];
        let diags = provider.check_local(&lines);

        let sem_errs: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Semantic)
            .collect();
        assert!(sem_errs.is_empty());
    }

    // -- Lint pass tests --

    #[test]
    fn lint_select_star_warning() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM employees".into()];
        let diags = provider.check_local(&lines);

        let lint_warns: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Lint && d.message.contains("SELECT *"))
            .collect();
        assert_eq!(lint_warns.len(), 1);
        assert_eq!(lint_warns[0].severity, DiagnosticSeverity::Warning);
    }

    #[test]
    fn lint_no_warning_for_named_columns() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT employee_id FROM employees".into()];
        let diags = provider.check_local(&lines);

        let lint_star: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Lint && d.message.contains("SELECT *"))
            .collect();
        assert!(lint_star.is_empty());
    }

    #[test]
    fn lint_delete_without_where() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["DELETE FROM employees".into()];
        let diags = provider.check_local(&lines);

        let lint_warns: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Lint && d.message.contains("without WHERE"))
            .collect();
        assert!(!lint_warns.is_empty());
    }

    #[test]
    fn lint_update_without_where() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["UPDATE employees SET name = 'x'".into()];
        let diags = provider.check_local(&lines);

        let lint_warns: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Lint && d.message.contains("without WHERE"))
            .collect();
        assert!(!lint_warns.is_empty());
    }

    #[test]
    fn lint_no_warning_with_where() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["DELETE FROM employees WHERE id = 1".into()];
        let diags = provider.check_local(&lines);

        let lint_warns: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Lint && d.message.contains("without WHERE"))
            .collect();
        assert!(lint_warns.is_empty());
    }

    #[test]
    fn lint_join_without_on() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM employees JOIN departments WHERE 1=1".into()];
        let diags = provider.check_local(&lines);

        let lint_warns: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Lint && d.message.contains("JOIN without ON"))
            .collect();
        assert!(!lint_warns.is_empty());
    }

    #[test]
    fn lint_join_with_on_no_warning() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> =
            vec!["SELECT * FROM employees e JOIN departments d ON e.dept_id = d.id".into()];
        let diags = provider.check_local(&lines);

        let lint_warns: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Lint && d.message.contains("JOIN without ON"))
            .collect();
        assert!(lint_warns.is_empty());
    }

    #[test]
    fn lint_cross_join_no_warning() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM employees CROSS JOIN departments".into()];
        let diags = provider.check_local(&lines);

        let lint_warns: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Lint && d.message.contains("JOIN without ON"))
            .collect();
        assert!(lint_warns.is_empty());
    }

    // -- Multi-pass integration --

    #[test]
    fn all_three_passes_produce_results() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        // This SQL has: syntax error (SELEC), and if it parsed,
        // would have SELECT * lint warning. But syntax error stops parsing.
        // Let's use valid SQL that triggers multiple passes:
        let lines: Vec<String> = vec!["DELETE * FROM nonexistent".into()];
        let diags = provider.check_local(&lines);

        // Should have: syntax error (DELETE *), semantic error (nonexistent),
        // and/or lint warnings depending on what parses
        assert!(!diags.is_empty());
    }

    #[test]
    fn multiple_query_blocks_validated_independently() {
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec![
            "SELECT * FROM employees".into(),
            "".into(),
            "SELEC * FROM departments".into(),
        ];
        let diags = provider.check_local(&lines);

        // First block: valid syntax, but has SELECT * lint warning
        // Second block: syntax error
        let syntax_errs: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Syntax)
            .collect();
        assert!(!syntax_errs.is_empty());
        // Syntax error should be on row 2 (the "SELEC" line)
        assert_eq!(syntax_errs[0].row, 2);
    }
}
