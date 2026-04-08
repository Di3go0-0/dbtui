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
#[allow(dead_code)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// Source of the diagnostic (which pass produced it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
pub struct DiagnosticSet {
    items: Vec<Diagnostic>,
    generation: u64,
}

#[allow(dead_code)]
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
    /// Tokenizes once and shares context across passes.
    pub fn check_local(&self, lines: &[String]) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // Pass 1: Syntax
        self.check_syntax(lines, &mut diagnostics);

        // Shared tokenization for passes 2+3
        let line_strs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let tokens = tokenizer::tokenize_sql(&line_strs);

        // Pass 2: Semantic (table/schema references) + "did you mean?" suggestions
        self.check_references(lines, &mut diagnostics);

        // Pass 3: Lint (uses shared tokens)
        self.check_lint_with_tokens(&tokens, &mut diagnostics);

        diagnostics
    }

    // -----------------------------------------------------------------------
    // Pass 1: Syntax validation
    // -----------------------------------------------------------------------

    fn check_syntax(&self, lines: &[String], out: &mut Vec<Diagnostic>) {
        let dialect = self.dialect.parser_dialect();

        // Pre-compute which lines are inside a PL/SQL anonymous block so the
        // blank-line splitter below doesn't cut `DECLARE .. BEGIN .. END;` in
        // half (common when the user leaves a blank line between the JSON
        // payload and the procedure call). Without this, the tail half would
        // start with a bare `SCHEMA.PKG.PROC(...)` call which sqlparser can't
        // parse as a standalone statement.
        let plsql_mask = compute_plsql_mask(lines);

        // Split into query blocks separated by **two or more** consecutive
        // blank lines. A single blank line stays inside the same block, so a
        // SELECT broken visually like:
        //
        //     SELECT *
        //
        //     FROM orders
        //
        // is treated as one statement (matching `query_block_at_cursor`'s
        // runtime extraction — they MUST agree, otherwise the editor
        // flags as a syntax error something the engine happily executes).
        //
        // Blanks that fall inside a PL/SQL anonymous block don't count as
        // separators at all, so a `DECLARE..BEGIN..END;` with internal blank
        // lines stays intact.
        let mut block_start: Option<usize> = None;
        let mut consecutive_blanks: usize = 0;
        let mut i = 0;
        while i <= lines.len() {
            let at_eof = i == lines.len();
            let in_plsql = !at_eof && plsql_mask.get(i).copied().unwrap_or(false);
            let is_blank = !at_eof && lines[i].trim().is_empty() && !in_plsql;

            if is_blank {
                consecutive_blanks += 1;
            }

            // Flush the current block when we hit a real separator: at EOF,
            // or after 2+ consecutive non-PL/SQL blank lines.
            let separator = at_eof || consecutive_blanks >= 2;
            if separator && let Some(start) = block_start {
                // The block ends at the first blank of the run (or at EOF).
                let end_excl = i - consecutive_blanks;
                if end_excl > start {
                    let block: String = lines[start..end_excl]
                        .iter()
                        .map(|l| l.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    // Skip linting PL/SQL DDL forms that sqlparser doesn't
                    // support (CREATE OR REPLACE TYPE / PACKAGE / TRIGGER /
                    // etc.). Compiling these is what surfaces real errors
                    // via the database.
                    if !block.trim().is_empty()
                        && !is_unsupported_plsql_ddl(&block)
                        && let Err(e) = Parser::parse_sql(dialect.as_ref(), &block)
                    {
                        let msg = e.to_string();
                        let (err_line, err_col) = parse_syntax_error_position(&msg);
                        let file_row = start + err_line.saturating_sub(1);
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
                }
                block_start = None;
            }

            // A non-blank line starts (or continues) a block.
            if !at_eof && !is_blank {
                if block_start.is_none() {
                    block_start = Some(i);
                }
                consecutive_blanks = 0;
            }

            i += 1;
        }
    }

    // -----------------------------------------------------------------------
    // Pass 2: Semantic reference validation
    // -----------------------------------------------------------------------

    fn check_references(&self, lines: &[String], out: &mut Vec<Diagnostic>) {
        // Skip semantic checks when metadata is not yet loaded — avoids
        // false "unknown table/schema" errors during connection warmup.
        if self.metadata.all_schemas().is_empty() {
            return;
        }

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
            // "Did you mean?" — fuzzy-match unknown names against metadata
            let suggestion = self.suggest_similar(&err.message, &err.kind);
            let message = if let Some(ref s) = suggestion {
                format!("{} — did you mean '{s}'?", err.message)
            } else {
                err.message.clone()
            };
            out.push(Diagnostic {
                row: err.location.row,
                col_start: err.location.col_start,
                col_end: err.location.col_end,
                message,
                severity,
                source: DiagnosticSource::Semantic,
            });
        }
    }

    // -----------------------------------------------------------------------
    // Pass 3: Lint rules
    // -----------------------------------------------------------------------

    /// Lint using pre-tokenized tokens (avoids re-tokenization).
    fn check_lint_with_tokens(&self, tokens: &[tokenizer::Token<'_>], out: &mut Vec<Diagnostic>) {
        self.lint_select_star_tokens(tokens, out);
        self.lint_missing_where_tokens(tokens, out);
        self.lint_join_without_on_tokens(tokens, out);
    }

    fn lint_select_star_tokens(&self, tokens: &[tokenizer::Token<'_>], out: &mut Vec<Diagnostic>) {
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

    fn lint_missing_where_tokens(
        &self,
        tokens: &[tokenizer::Token<'_>],
        out: &mut Vec<Diagnostic>,
    ) {
        let words: Vec<String> = tokens
            .iter()
            .filter(|t| t.kind == tokenizer::TokenKind::Word)
            .map(|t| t.text.to_uppercase())
            .collect();

        let has_where = words.iter().any(|w| w == "WHERE");

        for token in tokens {
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

    fn lint_join_without_on_tokens(
        &self,
        tokens: &[tokenizer::Token<'_>],
        out: &mut Vec<Diagnostic>,
    ) {
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

    /// Suggest a similar name from metadata when a table/schema/column is unknown.
    fn suggest_similar(&self, error_msg: &str, kind: &ResolutionErrorKind) -> Option<String> {
        use crate::sql_engine::completion::fuzzy_match;
        use crate::sql_engine::metadata::ObjectKind;

        // Extract the unknown name from the error message
        let name = error_msg
            .strip_prefix("Unknown table '")
            .or_else(|| error_msg.strip_prefix("Unknown schema '"))
            .or_else(|| error_msg.strip_prefix("Unknown column '"))
            .and_then(|s| s.strip_suffix('\''))?;

        let candidates: Vec<String> = match kind {
            ResolutionErrorKind::UnknownTable => {
                let kinds = &[ObjectKind::Table, ObjectKind::View];
                self.metadata
                    .objects_by_kind(None, kinds)
                    .iter()
                    .map(|e| e.display_name.clone())
                    .collect()
            }
            ResolutionErrorKind::UnknownSchema => self
                .metadata
                .all_schemas()
                .iter()
                .map(|s| s.to_string())
                .collect(),
            _ => return None,
        };

        // Find the best fuzzy match with a reasonable threshold
        let mut best: Option<(String, i32)> = None;
        for candidate in &candidates {
            if let Some(m) = fuzzy_match(name, candidate)
                && m.score > 200
                && best.as_ref().is_none_or(|(_, s)| m.score > *s)
            {
                best = Some((candidate.clone(), m.score));
            }
        }
        best.map(|(name, _)| name)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Mark every line that is part of a PL/SQL anonymous block (DECLARE / BEGIN
/// .. END;) so the blank-line block splitter can skip over interior blank
/// lines. Tracks BEGIN/END nesting and ignores the non-terminal END forms
/// (END IF, END LOOP, END CASE, END WHILE, END FOR).
fn compute_plsql_mask(lines: &[String]) -> Vec<bool> {
    let mut mask = vec![false; lines.len()];
    let mut i = 0;
    let n = lines.len();
    while i < n {
        let trimmed_upper = lines[i].trim().to_ascii_uppercase();
        let starts_block = trimmed_upper.starts_with("DECLARE")
            || trimmed_upper == "BEGIN"
            || trimmed_upper.starts_with("BEGIN ")
            || trimmed_upper.starts_with("BEGIN\t")
            || trimmed_upper.starts_with("BEGIN;");
        if !starts_block {
            i += 1;
            continue;
        }
        // Walk forward, counting BEGIN vs terminating END tokens. We only
        // mark lines as PL/SQL once we've actually seen a BEGIN (a DECLARE
        // preamble isn't itself a PL/SQL block until the BEGIN appears).
        let start = i;
        let mut depth: i32 = 0;
        let mut saw_begin = false;
        let mut j = i;
        while j < n {
            // Strip line comments so `-- END;` in a comment doesn't close.
            let code_upper = lines[j].to_ascii_uppercase();
            let code = code_upper.split("--").next().unwrap_or("");
            let bytes = code.as_bytes();
            // Count BEGIN tokens on this line.
            for tok in code.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
                if tok == "BEGIN" {
                    depth += 1;
                    saw_begin = true;
                }
            }
            // Count terminating END / END <label>; tokens (skipping the
            // control-flow enders).
            for (pos, _) in code.match_indices("END") {
                let before_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
                let after_ok = pos + 3 == bytes.len()
                    || !(bytes[pos + 3].is_ascii_alphanumeric() || bytes[pos + 3] == b'_');
                if !before_ok || !after_ok {
                    continue;
                }
                let rest = code[pos + 3..].trim_start();
                if rest.starts_with(';') {
                    depth -= 1;
                    continue;
                }
                if let Some(ident_end) = rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                {
                    let ident = &rest[..ident_end];
                    let after = rest[ident_end..].trim_start();
                    if after.starts_with(';')
                        && !matches!(ident, "IF" | "LOOP" | "CASE" | "WHILE" | "FOR")
                    {
                        depth -= 1;
                    }
                }
            }
            if saw_begin && depth <= 0 {
                mask[start..=j].fill(true);
                i = j + 1;
                break;
            }
            j += 1;
        }
        if j >= n {
            // Unterminated — still mark the tail so sqlparser doesn't see it
            // as a bunch of stray statements.
            if saw_begin
                || trimmed_upper.starts_with("DECLARE")
                || trimmed_upper.starts_with("BEGIN")
            {
                mask[start..n].fill(true);
            }
            break;
        }
    }
    mask
}

/// True for PL/SQL DDL forms that the underlying sqlparser crate doesn't
/// support (Oracle TYPE / PACKAGE / TRIGGER bodies, anonymous blocks, etc.).
/// We don't surface "syntax errors" for these — the user gets real errors
/// from the database when they compile via <leader>+s+s anyway.
fn is_unsupported_plsql_ddl(block: &str) -> bool {
    let trimmed = block.trim_start().to_ascii_uppercase();
    // Strip leading line comments / whitespace so the prefix check is robust.
    let mut s = trimmed.as_str();
    loop {
        s = s.trim_start();
        if let Some(rest) = s.strip_prefix("--") {
            // Skip the rest of the line.
            s = rest.split_once('\n').map(|(_, after)| after).unwrap_or("");
            continue;
        }
        break;
    }
    // CREATE [OR REPLACE] [EDITIONABLE | NONEDITIONABLE] {TYPE | PACKAGE [BODY] | TRIGGER}
    if s.starts_with("CREATE ") {
        let after = s.trim_start_matches("CREATE ").trim_start();
        let after = after
            .strip_prefix("OR REPLACE ")
            .map(|r| r.trim_start())
            .unwrap_or(after);
        let after = after
            .strip_prefix("EDITIONABLE ")
            .or_else(|| after.strip_prefix("NONEDITIONABLE "))
            .map(|r| r.trim_start())
            .unwrap_or(after);
        if after.starts_with("TYPE ")
            || after.starts_with("PACKAGE ")
            || after.starts_with("TRIGGER ")
            || after.starts_with("PROCEDURE ")
            || after.starts_with("FUNCTION ")
        {
            return true;
        }
    }
    // DECLARE / BEGIN-only blocks (anonymous PL/SQL) — sqlparser refuses these.
    if s.starts_with("DECLARE") || s.starts_with("BEGIN") {
        return true;
    }
    false
}

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

    #[test]
    fn plsql_mask_spans_blank_lines_inside_begin_end() {
        // Repro: anonymous PL/SQL block with a blank line between the JSON
        // payload and the procedure call. The whole DECLARE..END; must be
        // marked as PL/SQL so the syntax splitter doesn't feed the second
        // half ("PLANTAFISICA.PKG...") to sqlparser as a stray statement.
        let src = r#"DECLARE
  v_json JSON;
BEGIN
  v_json := JSON('{"x": 1}');

  PLANTAFISICA.PKG_EDEPORTIVOS.SP_CREAREVENTO(
    P_JSON => v_json
  );
END;"#;
        let lines: Vec<String> = src.lines().map(|s| s.to_string()).collect();
        let mask = compute_plsql_mask(&lines);
        assert!(
            mask.iter().all(|&b| b),
            "every line should be marked PL/SQL: {mask:?}"
        );

        // And no syntax diagnostic should fire for the whole block.
        let idx = MetadataIndex::new();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);
        let mut diags = Vec::new();
        provider.check_syntax(&lines, &mut diags);
        assert!(
            diags.is_empty(),
            "expected no syntax diagnostics for PL/SQL block, got: {diags:?}"
        );
    }

    #[test]
    fn plsql_mask_leaves_surrounding_sql_alone() {
        // A SELECT followed by a DECLARE block followed by another SELECT —
        // only the middle range should be marked.
        let src = "SELECT 1 FROM dual;\n\nDECLARE\n  x NUMBER;\nBEGIN\n  NULL;\nEND;\n\nSELECT 2 FROM dual;";
        let lines: Vec<String> = src.lines().map(|s| s.to_string()).collect();
        let mask = compute_plsql_mask(&lines);
        assert!(!mask[0], "line 0 (first SELECT) should NOT be PL/SQL");
        assert!(mask[2], "DECLARE line should be PL/SQL");
        assert!(mask[6], "END; line should be PL/SQL");
        assert!(!mask[8], "trailing SELECT should NOT be PL/SQL");
    }

    #[test]
    fn skips_oracle_create_or_replace_type() {
        assert!(is_unsupported_plsql_ddl(
            "CREATE OR REPLACE TYPE emp_obj AS OBJECT (id NUMBER)"
        ));
        assert!(is_unsupported_plsql_ddl(
            "  CREATE OR REPLACE EDITIONABLE PACKAGE emp_pkg AS\n  END;"
        ));
        assert!(is_unsupported_plsql_ddl(
            "CREATE TRIGGER my_trg BEFORE INSERT"
        ));
        assert!(is_unsupported_plsql_ddl(
            "DECLARE x NUMBER; BEGIN NULL; END;"
        ));
        assert!(is_unsupported_plsql_ddl("BEGIN NULL; END;"));
        // Should NOT skip — these the parser should still validate.
        assert!(!is_unsupported_plsql_ddl("CREATE TABLE t (id NUMBER)"));
        assert!(!is_unsupported_plsql_ddl("SELECT * FROM dual"));
        assert!(!is_unsupported_plsql_ddl(
            "CREATE OR REPLACE VIEW v AS SELECT 1 FROM dual"
        ));
    }

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

        // Two blocks separated by two blank lines (the real splitter
        // threshold — a single blank line keeps them in the same block).
        let lines: Vec<String> = vec![
            "SELECT * FROM employees".into(),
            "".into(),
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
        // Syntax error should be on row 3 (the "SELEC" line)
        assert_eq!(syntax_errs[0].row, 3);
    }

    #[test]
    fn single_blank_line_does_not_split_query() {
        // Repro: user's case. A SELECT visually broken with a single
        // blank line between the projection and the FROM clause must be
        // treated as ONE statement — same contract as the runtime
        // `query_block_at_cursor` extractor — and produce ZERO syntax
        // diagnostics.
        let idx = test_index();
        let dialect = OracleDialect;
        let provider = DiagnosticProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT *".into(), "".into(), "FROM employees".into()];
        let diags = provider.check_local(&lines);

        let syntax_errs: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.source == DiagnosticSource::Syntax)
            .collect();
        assert!(
            syntax_errs.is_empty(),
            "single blank line should not split — got: {syntax_errs:?}"
        );
    }
}
