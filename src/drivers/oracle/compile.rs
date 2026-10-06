//! Compilation errors of stored PL/SQL units.
//!
//! `CREATE OR REPLACE PROCEDURE ...` succeeds even when the body does not
//! compile: Oracle stores the object as INVALID and answers with ORA-24344 as
//! a success-with-info, which the `oracle` crate records as the connection's
//! last warning instead of returning an error. The actual diagnostics live in
//! `ALL_ERRORS`. This module turns that into a failure the editor can show.

use oracle::Connection;

use crate::core::adapter::skip_leading_noise;
use crate::core::error::{DbError, ErrorPosition};

/// ORA-24344: success with compilation error.
const ORA_COMPILATION_ERROR: i32 = 24344;

/// The stored unit a `CREATE` statement defines.
#[derive(Debug, PartialEq, Eq)]
struct CompiledObject {
    /// Schema written in the statement, if any.
    owner: Option<String>,
    name: String,
    /// As `ALL_ERRORS.TYPE` spells it: `PROCEDURE`, `PACKAGE BODY`, ...
    object_type: String,
    /// Byte offset of the type keyword in the statement. Oracle stores the
    /// source from that keyword on, so line 1 of `ALL_ERRORS` starts there.
    source_offset: usize,
}

/// One row of `ALL_ERRORS`.
#[derive(Debug)]
struct CompileError {
    line: usize,
    col: usize,
    text: String,
    is_error: bool,
}

/// If the statement just executed on `conn` left a compilation warning,
/// build the failure for it. Must be called before anything else runs on the
/// connection, since every execution overwrites the last warning.
pub(super) fn compile_failure(conn: &Connection, sql: &str) -> Option<DbError> {
    let warning = conn.last_warning()?;
    let db_err = warning.db_error()?;
    if db_err.code() != ORA_COMPILATION_ERROR {
        return None;
    }
    let headline = db_err.message().trim().to_string();

    let Some(object) = parse_compiled_object(sql) else {
        return Some(DbError::QueryFailed(headline));
    };
    let errors = fetch_errors(conn, &object).unwrap_or_default();
    Some(DbError::query_failed(
        failure_message(&headline, &object, &errors),
        first_error_position(sql, &object, &errors),
    ))
}

/// Read the unit's rows from `ALL_ERRORS`, in the order Oracle reported them.
fn fetch_errors(conn: &Connection, object: &CompiledObject) -> Option<Vec<CompileError>> {
    let owner = match &object.owner {
        Some(owner) => owner.clone(),
        // An unqualified name is created in the session's current schema.
        None => conn
            .query_row_as::<String>(
                "SELECT SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA') FROM DUAL",
                &[],
            )
            .ok()?,
    };
    let rows = conn
        .query(
            "SELECT line, position, text, attribute FROM all_errors \
             WHERE owner = :1 AND name = :2 AND type = :3 \
             ORDER BY sequence",
            &[&owner, &object.name, &object.object_type],
        )
        .ok()?;

    let mut errors = Vec::new();
    for row in rows.flatten() {
        let attribute: String = row.get(3).unwrap_or_default();
        errors.push(CompileError {
            line: row
                .get::<usize, i64>(0)
                .ok()
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0),
            col: row
                .get::<usize, i64>(1)
                .ok()
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0),
            text: row.get::<usize, String>(2).unwrap_or_default(),
            is_error: attribute != "WARNING",
        });
    }
    Some(errors)
}

/// Headline plus one `LINE/COL: text` row per diagnostic, as SQL*Plus'
/// `SHOW ERRORS` lists them.
fn failure_message(headline: &str, object: &CompiledObject, errors: &[CompileError]) -> String {
    let qualified = match &object.owner {
        Some(owner) => format!("{owner}.{}", object.name),
        None => object.name.clone(),
    };
    let mut out = format!(
        "{headline}\n{} {qualified} was created with compilation errors",
        object.object_type
    );
    if errors.is_empty() {
        out.push_str(" (no detail available in ALL_ERRORS)");
        return out;
    }
    out.push(':');
    for e in errors {
        out.push_str(&format!("\n{}/{}: {}", e.line, e.col, e.text.trim()));
    }
    out
}

/// Position of the first real error, translated from the stored source's
/// coordinates to the submitted statement's.
///
/// Triggers are skipped: Oracle numbers their diagnostics from the start of
/// the trigger body rather than from the `TRIGGER` keyword, so the offset to
/// the statement cannot be derived reliably. Rows at line 0 (object-level
/// messages) carry no location either.
fn first_error_position(
    sql: &str,
    object: &CompiledObject,
    errors: &[CompileError],
) -> Option<ErrorPosition> {
    if object.object_type == "TRIGGER" {
        return None;
    }
    let first = errors
        .iter()
        .find(|e| e.is_error && e.line > 0)
        .or_else(|| errors.iter().find(|e| e.line > 0))?;

    let start = ErrorPosition::from_byte_offset(sql, object.source_offset);
    let start_col = start.col.unwrap_or(1);
    let col = first.col.max(1);
    Some(if first.line == 1 {
        // Same line as the type keyword: shift by what precedes it.
        ErrorPosition {
            line: start.line,
            col: Some(start_col + col - 1),
        }
    } else {
        ErrorPosition {
            line: start.line + first.line - 1,
            col: Some(col),
        }
    })
}

/// Work out which stored unit a statement creates:
/// `CREATE [OR REPLACE] [EDITIONABLE | NONEDITIONABLE | FORCE | NOFORCE]
///  {PROCEDURE | FUNCTION | PACKAGE [BODY] | TYPE [BODY] | TRIGGER | VIEW}
///  [schema.]name`.
fn parse_compiled_object(sql: &str) -> Option<CompiledObject> {
    let mut scan = Scanner { sql, pos: 0 };

    if scan.word()? != "CREATE" {
        return None;
    }
    let mut source_offset = scan.peek_offset();
    let mut keyword = scan.word()?;
    if keyword == "OR" {
        if scan.word()? != "REPLACE" {
            return None;
        }
        source_offset = scan.peek_offset();
        keyword = scan.word()?;
    }
    while matches!(
        keyword.as_str(),
        "EDITIONABLE" | "NONEDITIONABLE" | "FORCE" | "NOFORCE" | "NO"
    ) {
        source_offset = scan.peek_offset();
        keyword = scan.word()?;
    }

    let object_type = match keyword.as_str() {
        "PROCEDURE" | "FUNCTION" | "TRIGGER" | "VIEW" => keyword,
        "PACKAGE" | "TYPE" => {
            let before_body = scan.pos;
            if scan.word().as_deref() == Some("BODY") {
                format!("{keyword} BODY")
            } else {
                scan.pos = before_body;
                keyword
            }
        }
        _ => return None,
    };

    let first = scan.identifier()?;
    let (owner, name) = if scan.eat('.') {
        (Some(first), scan.identifier()?)
    } else {
        (None, first)
    };

    Some(CompiledObject {
        owner,
        name,
        object_type,
        source_offset,
    })
}

/// Minimal reader over the head of a statement.
struct Scanner<'a> {
    sql: &'a str,
    pos: usize,
}

impl Scanner<'_> {
    fn rest(&self) -> &str {
        &self.sql[self.pos..]
    }

    fn skip_noise(&mut self) {
        self.pos += skip_leading_noise(self.rest());
    }

    /// Offset of the next token, without consuming it.
    fn peek_offset(&mut self) -> usize {
        self.skip_noise();
        self.pos
    }

    /// Next bare word, upper-cased.
    fn word(&mut self) -> Option<String> {
        self.skip_noise();
        let len = self
            .rest()
            .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '$' | '#')))
            .unwrap_or(self.rest().len());
        if len == 0 {
            return None;
        }
        let word = self.rest()[..len].to_uppercase();
        self.pos += len;
        Some(word)
    }

    /// Next identifier the way the dictionary stores it: a quoted name keeps
    /// its case, a bare one is folded to upper case.
    fn identifier(&mut self) -> Option<String> {
        self.skip_noise();
        let Some(quoted) = self.rest().strip_prefix('"') else {
            return self.word();
        };
        let end = quoted.find('"')?;
        let name = quoted[..end].to_string();
        self.pos += end + 2;
        (!name.is_empty()).then_some(name)
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_noise();
        if self.rest().starts_with(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(sql: &str) -> Option<(Option<String>, String, String)> {
        parse_compiled_object(sql).map(|o| (o.owner, o.name, o.object_type))
    }

    fn named(
        owner: Option<&str>,
        name: &str,
        ty: &str,
    ) -> Option<(Option<String>, String, String)> {
        Some((owner.map(str::to_string), name.to_string(), ty.to_string()))
    }

    fn err(line: usize, col: usize, text: &str) -> CompileError {
        CompileError {
            line,
            col,
            text: text.to_string(),
            is_error: true,
        }
    }

    #[test]
    fn bare_names_fold_to_upper_case() {
        assert_eq!(
            object("create or replace procedure my_proc is begin null; end;"),
            named(None, "MY_PROC", "PROCEDURE")
        );
        assert_eq!(
            object("CREATE FUNCTION hr.calc(x NUMBER) RETURN NUMBER IS BEGIN RETURN x; END;"),
            named(Some("HR"), "CALC", "FUNCTION")
        );
    }

    #[test]
    fn quoted_names_keep_their_case() {
        assert_eq!(
            object("CREATE OR REPLACE PROCEDURE \"Hr\".\"myProc\" IS BEGIN NULL; END;"),
            named(Some("Hr"), "myProc", "PROCEDURE")
        );
    }

    #[test]
    fn bodies_and_modifiers() {
        assert_eq!(
            object("CREATE OR REPLACE PACKAGE BODY pkg AS END;"),
            named(None, "PKG", "PACKAGE BODY")
        );
        assert_eq!(
            object("CREATE OR REPLACE PACKAGE pkg AS END;"),
            named(None, "PKG", "PACKAGE")
        );
        assert_eq!(
            object("CREATE OR REPLACE EDITIONABLE TYPE BODY hr.t AS END;"),
            named(Some("HR"), "T", "TYPE BODY")
        );
        assert_eq!(
            object(
                "CREATE OR REPLACE NONEDITIONABLE TRIGGER trg BEFORE INSERT ON t BEGIN NULL; END;"
            ),
            named(None, "TRG", "TRIGGER")
        );
        assert_eq!(
            object("CREATE OR REPLACE FORCE VIEW v AS SELECT 1 x FROM dual"),
            named(None, "V", "VIEW")
        );
    }

    #[test]
    fn comments_and_newlines_between_tokens() {
        assert_eq!(
            object(
                "-- header\nCREATE /* x */ OR\nREPLACE\n  PROCEDURE -- name next\n  p IS BEGIN NULL; END;"
            ),
            named(None, "P", "PROCEDURE")
        );
    }

    #[test]
    fn a_package_named_body_like_word_is_not_swallowed() {
        assert_eq!(
            object("CREATE PACKAGE bodyguard AS END;"),
            named(None, "BODYGUARD", "PACKAGE")
        );
    }

    #[test]
    fn statements_that_create_no_stored_unit() {
        assert_eq!(object("CREATE TABLE t (id NUMBER)"), None);
        assert_eq!(object("SELECT 1 FROM dual"), None);
        assert_eq!(object("CREATE OR REPLACE"), None);
        assert_eq!(object("CREATE OR DROP PROCEDURE p"), None);
        assert_eq!(object("CREATE PROCEDURE \"unterminated"), None);
        assert_eq!(object(""), None);
    }

    #[test]
    fn source_starts_at_the_type_keyword() {
        let sql = "CREATE OR REPLACE\n  PROCEDURE p IS BEGIN NULL; END;";
        let offset = parse_compiled_object(sql).map(|o| o.source_offset);
        assert_eq!(offset, sql.find("PROCEDURE"));

        let sql = "CREATE OR REPLACE EDITIONABLE FUNCTION f RETURN NUMBER IS BEGIN RETURN 1; END;";
        let offset = parse_compiled_object(sql).map(|o| o.source_offset);
        assert_eq!(offset, sql.find("FUNCTION"));
    }

    #[test]
    fn position_on_a_later_line_is_offset_by_the_header_line() {
        // The stored source starts on line 2 of the statement.
        let sql = "-- fix me\nCREATE OR REPLACE PROCEDURE p IS\nBEGIN\n  nope;\nEND;";
        let object = parse_compiled_object(sql);
        let position = object
            .as_ref()
            .and_then(|o| first_error_position(sql, o, &[err(3, 3, "PLS-00201")]));
        assert_eq!(
            position,
            Some(ErrorPosition {
                line: 4,
                col: Some(3)
            })
        );
    }

    #[test]
    fn position_on_the_first_line_is_shifted_past_the_create_prefix() {
        let sql = "CREATE OR REPLACE PROCEDURE p IS BEGIN nope; END;";
        let object = parse_compiled_object(sql);
        // Column 22 of the stored source is the `n` of `nope`.
        let position = object
            .as_ref()
            .and_then(|o| first_error_position(sql, o, &[err(1, 22, "PLS-00201")]));
        assert_eq!(
            position,
            Some(ErrorPosition {
                line: 1,
                col: sql.find("nope").map(|i| i + 1)
            })
        );
    }

    #[test]
    fn warnings_and_unlocated_rows_do_not_win_over_a_real_error() {
        let sql = "CREATE PROCEDURE p IS\nBEGIN\n  nope;\nEND;";
        let object = parse_compiled_object(sql);
        let errors = [
            CompileError {
                line: 1,
                col: 1,
                text: "PLW-05018".to_string(),
                is_error: false,
            },
            err(0, 0, "PLS-00905: object is invalid"),
            err(3, 3, "PLS-00201"),
        ];
        let position = object
            .as_ref()
            .and_then(|o| first_error_position(sql, o, &errors));
        assert_eq!(
            position,
            Some(ErrorPosition {
                line: 3,
                col: Some(3)
            })
        );
    }

    #[test]
    fn triggers_and_empty_error_lists_have_no_position() {
        let sql = "CREATE TRIGGER trg BEFORE INSERT ON t\nBEGIN\n  nope;\nEND;";
        let object = parse_compiled_object(sql);
        assert_eq!(
            object
                .as_ref()
                .and_then(|o| first_error_position(sql, o, &[err(2, 3, "PLS-00201")])),
            None
        );

        let sql = "CREATE PROCEDURE p IS BEGIN NULL; END;";
        let object = parse_compiled_object(sql);
        assert_eq!(
            object
                .as_ref()
                .and_then(|o| first_error_position(sql, o, &[])),
            None
        );
    }

    #[test]
    fn message_lists_every_row() {
        let object = CompiledObject {
            owner: Some("HR".to_string()),
            name: "P".to_string(),
            object_type: "PROCEDURE".to_string(),
            source_offset: 0,
        };
        let message = failure_message(
            "ORA-24344: success with compilation error",
            &object,
            &[
                err(3, 3, "PLS-00201: identifier 'NOPE' must be declared\n"),
                err(3, 3, "PL/SQL: Statement ignored"),
            ],
        );
        assert_eq!(
            message,
            "ORA-24344: success with compilation error\n\
             PROCEDURE HR.P was created with compilation errors:\n\
             3/3: PLS-00201: identifier 'NOPE' must be declared\n\
             3/3: PL/SQL: Statement ignored"
        );
    }

    #[test]
    fn message_without_detail_says_so() {
        let object = CompiledObject {
            owner: None,
            name: "P".to_string(),
            object_type: "PACKAGE BODY".to_string(),
            source_offset: 0,
        };
        assert_eq!(
            failure_message("ORA-24344: success with compilation error", &object, &[]),
            "ORA-24344: success with compilation error\n\
             PACKAGE BODY P was created with compilation errors \
             (no detail available in ALL_ERRORS)"
        );
    }
}
