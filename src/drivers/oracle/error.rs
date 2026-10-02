//! Oracle error formatting and positions for user statements.

use crate::core::adapter::skip_leading_noise;
use crate::core::error::{DbError, ErrorPosition};

/// ORA-00900 "invalid SQL statement": the one parse error whose offset is
/// legitimately 0, because the very first word is what Oracle rejected.
const ORA_INVALID_SQL_STATEMENT: i32 = 900;

/// Convert an error raised while parsing or executing the user's statement.
pub(super) fn statement_error(err: &oracle::Error, sql: &str) -> DbError {
    let position = err
        .db_error()
        .and_then(|db| error_position(db.code(), db.offset(), db.message(), sql));
    DbError::query_failed(format_oracle_error(err, sql), position)
}

/// Convert an error raised while fetching rows. The offset OCI reports at
/// that stage is not a parse offset, so only a PL/SQL frame can locate it.
pub(super) fn fetch_error(err: &oracle::Error, sql: &str) -> DbError {
    let position = err.db_error().and_then(|db| plsql_position(db.message()));
    DbError::query_failed(format_oracle_error(err, sql), position)
}

/// Where in `sql` Oracle reported the failure, if it said.
///
/// * PL/SQL compile errors (ORA-06550) and anonymous-block stack frames name
///   a line themselves.
/// * Parse errors carry a byte offset into the statement.
/// * Runtime errors (ORA-01400, ORA-00001, ...) carry neither: OCI leaves
///   the offset at 0, which must not be read as "line 1, column 1".
fn error_position(code: i32, offset: u32, message: &str, sql: &str) -> Option<ErrorPosition> {
    if let Some(position) = plsql_position(message) {
        return Some(position);
    }
    let offset = usize::try_from(offset).ok()?;
    if offset > 0 && offset <= sql.len() {
        return Some(ErrorPosition::from_byte_offset(sql, offset));
    }
    if offset == 0 && code == ORA_INVALID_SQL_STATEMENT {
        return Some(ErrorPosition::from_byte_offset(
            sql,
            skip_leading_noise(sql),
        ));
    }
    None
}

/// Position named by the PL/SQL engine inside the error text.
fn plsql_position(message: &str) -> Option<ErrorPosition> {
    if let Some(at) = message.find("ORA-06550") {
        let (line, col) = parse_ora_line_col(&message[at..])?;
        return Some(ErrorPosition {
            line: line.max(1),
            col: Some(col.max(1)),
        });
    }
    anonymous_block_line(message).map(ErrorPosition::line_only)
}

/// Line of the anonymous block's own frame in a PL/SQL error stack.
///
/// `ORA-06512: at line N` is the submitted block; frames of stored units read
/// `ORA-06512: at "OWNER.NAME", line N` and refer to that unit's source.
fn anonymous_block_line(message: &str) -> Option<usize> {
    const FRAME: &str = "ORA-06512: at line ";
    let at = message.find(FRAME)?;
    let digits: String = message[at + FRAME.len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok().filter(|line| *line > 0)
}

/// Format an `oracle::Error` into a richer message that includes the
/// full OCI text (with the offending identifier for ORA-00904 etc.),
/// the error code, and — when present — the byte offset within the
/// SQL statement where the problem was detected. A small excerpt of the SQL
/// around that offset is appended so the user can see exactly what Oracle
/// choked on.
///
/// Called at every call site on the user-statement path so the structured
/// detail that OCI returns is never silently dropped.
pub(super) fn format_oracle_error(err: &oracle::Error, sql: &str) -> String {
    if let Some(db_err) = err.db_error() {
        let mut out = String::new();
        out.push_str(db_err.message().trim());
        let code = db_err.code();
        if code != 0 {
            out.push_str(&format!(" [ORA-{code:05}]"));
        }
        let offset = db_err.offset() as usize;
        if offset > 0 && offset <= sql.len() {
            // Clip ±30 bytes around the offset for context. Byte-safe
            // clipping (find UTF-8 char boundaries) so multi-byte chars do
            // not cause a panic.
            let start = find_char_boundary(sql, offset.saturating_sub(30));
            let end = find_char_boundary(sql, (offset + 30).min(sql.len()));
            let snippet = &sql[start..end];
            out.push_str(&format!(
                "\nat offset {offset} near: ...{}...",
                snippet.replace('\n', " ")
            ));
        }
        // Targeted hints for the most ambiguous ORA codes. Oracle often
        // returns a bare "invalid identifier" or "table or view does not
        // exist" when the real cause is missing privileges — the user
        // can *see* the object in the tree but Oracle pretends it's not
        // there because they lack SELECT/EXECUTE. Spell it out.
        if let Some(hint) = ora_hint(code) {
            out.push_str("\nPossible causes:\n");
            out.push_str(hint);
        }
        return out;
    }
    err.to_string()
}

fn ora_hint(code: i32) -> Option<&'static str> {
    match code {
        904 => Some(
            "  • Typo or wrong case in a column / function / package name.\n  \
               • Missing EXECUTE privilege on a schema.package.function (most common\n    \
                 when the function is from another schema).\n  \
               • Missing SELECT privilege on a column or table.\n  \
               • Column alias referenced where Oracle doesn't allow aliases (rare).",
        ),
        942 => Some(
            "  • Table / view is in another schema and you lack SELECT privilege.\n  \
               • Wrong schema prefix, or the object was renamed / dropped.\n  \
               • Object exists as a synonym that points at something you can't see.",
        ),
        1017 => Some("  • Username or password is wrong (passwords are case-sensitive)."),
        1031 => Some(
            "  • Insufficient privileges — the operation needs a grant the user\n    doesn't have (e.g. ALTER / CREATE / DROP on the object).",
        ),
        12541 => Some(
            "  • Oracle listener isn't running at the target host:port.\n  \
               • Firewall blocking the TNS port (default 1521).",
        ),
        _ => None,
    }
}

fn find_char_boundary(s: &str, mut idx: usize) -> usize {
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx.min(s.len())
}

/// Extract 1-based `(line, column)` from an Oracle error message.
///
/// Handles the two common shapes Oracle emits:
///   * `ORA-06550: line 26, column 1:PLS-00103...`  (PL/SQL compiler)
///   * `... at line N ...`                          (generic)
///
/// Returns `None` if no line/column pair can be found.
pub(super) fn parse_ora_line_col(msg: &str) -> Option<(usize, usize)> {
    let lower = msg.to_ascii_lowercase();
    let idx = lower.find("line ")?;
    let after_line = &msg[idx + "line ".len()..];
    let line: usize = after_line
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    let col = lower[idx..]
        .find("column ")
        .and_then(|c| {
            msg[idx + c + "column ".len()..]
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>()
                .parse::<usize>()
                .ok()
        })
        .unwrap_or(1);
    Some((line, col))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: usize, col: usize) -> Option<ErrorPosition> {
        Some(ErrorPosition {
            line,
            col: Some(col),
        })
    }

    #[test]
    fn ora_06550_with_line_and_column() {
        let msg = "OCI Error: ORA-06550: line 26, column 1:PLS-00103: Encountered the symbol '-'";
        assert_eq!(parse_ora_line_col(msg), Some((26, 1)));
    }

    #[test]
    fn ora_error_line_only() {
        let msg = "ORA-00933: SQL command not properly ended at line 7";
        assert_eq!(parse_ora_line_col(msg), Some((7, 1)));
    }

    #[test]
    fn no_line_returns_none() {
        assert_eq!(parse_ora_line_col("ORA-00001: unique constraint"), None);
    }

    #[test]
    fn line_larger_than_single_digit() {
        let msg = "ORA-06550: line 123, column 42: something";
        assert_eq!(parse_ora_line_col(msg), Some((123, 42)));
    }

    #[test]
    fn plsql_compile_error_uses_the_reported_line_and_column() {
        let msg = "ORA-06550: line 3, column 7:\nPLS-00201: identifier 'NOPE' must be declared\n\
                   ORA-06550: line 3, column 7:\nPL/SQL: Statement ignored";
        assert_eq!(
            error_position(6550, 0, msg, "BEGIN\n  NULL;\n  x := nope;\nEND;"),
            at(3, 7)
        );
    }

    #[test]
    fn parse_error_uses_the_byte_offset() {
        let sql = "SELECT *\nFROM nope";
        // Offset 14 is the `n` of `nope`: line 2, column 6.
        assert_eq!(
            error_position(942, 14, "ORA-00942: table or view does not exist", sql),
            at(2, 6)
        );
    }

    #[test]
    fn parse_offset_is_in_bytes_and_the_column_in_characters() {
        let sql = "SELECT 'ñ' FROM nope";
        // `ñ` is two bytes: byte offset 17 has only 16 characters before it.
        assert_eq!(
            error_position(942, 17, "ORA-00942: table or view does not exist", sql),
            at(1, 17)
        );
    }

    #[test]
    fn runtime_errors_have_no_position() {
        let sql = "INSERT INTO t (a) VALUES (NULL)";
        assert_eq!(
            error_position(
                1400,
                0,
                "ORA-01400: cannot insert NULL into (\"HR\".\"T\".\"A\")",
                sql
            ),
            None
        );
        assert_eq!(
            error_position(1, 0, "ORA-00001: unique constraint (HR.T_PK) violated", sql),
            None
        );
    }

    #[test]
    fn offset_past_the_statement_is_ignored() {
        assert_eq!(
            error_position(942, 500, "ORA-00942: nope", "SELECT 1 FROM x"),
            None
        );
    }

    #[test]
    fn invalid_statement_points_at_the_first_word() {
        let sql = "-- oops\n  SELEC * FROM dual";
        assert_eq!(
            error_position(900, 0, "ORA-00900: invalid SQL statement", sql),
            at(2, 3)
        );
    }

    #[test]
    fn anonymous_block_frame_gives_a_line() {
        let msg =
            "ORA-01403: no data found\nORA-06512: at \"HR.LOOKUP\", line 4\nORA-06512: at line 2";
        assert_eq!(
            error_position(1403, 0, msg, "BEGIN\n  hr.lookup;\nEND;"),
            Some(ErrorPosition::line_only(2))
        );
    }

    #[test]
    fn stored_unit_frames_alone_give_no_position() {
        let msg = "ORA-20001: boom\nORA-06512: at \"HR.TRG_AUDIT\", line 9";
        assert_eq!(error_position(20001, 0, msg, "UPDATE t SET a = 1"), None);
        assert_eq!(anonymous_block_line(msg), None);
    }

    #[test]
    fn char_boundary_search_moves_forward() {
        let s = "añb";
        assert_eq!(find_char_boundary(s, 2), 3);
        assert_eq!(find_char_boundary(s, 1), 1);
        assert_eq!(find_char_boundary(s, 99), s.len());
    }
}
