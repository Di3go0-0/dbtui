//! sqlx/MySQL error → `DbError` conversion for user statements.

use sqlx::mysql::MySqlDatabaseError;

use crate::core::error::{DbError, ErrorPosition};

/// Convert an error raised by anything other than the user's statement
/// (acquiring a connection, `BEGIN`, `COMMIT`).
pub(super) fn plain_error(err: &sqlx::Error) -> DbError {
    DbError::QueryFailed(err.to_string())
}

/// Convert an error raised by the user's statement, attaching the line the
/// server names in its message.
pub(super) fn statement_error(err: &sqlx::Error) -> DbError {
    let position = err
        .as_database_error()
        .and_then(|e| e.try_downcast_ref::<MySqlDatabaseError>())
        .and_then(|e| line_from_message(e.message()));
    DbError::query_failed(err.to_string(), position)
}

/// MySQL has no structured error position; syntax errors end with
/// `... near '<text>' at line N`, counted from the start of the statement.
fn line_from_message(message: &str) -> Option<ErrorPosition> {
    let (_, tail) = message.trim_end().rsplit_once(" at line ")?;
    if tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let line: usize = tail.parse().ok()?;
    (line > 0).then(|| ErrorPosition::line_only(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_error_reports_its_line() {
        let msg = "You have an error in your SQL syntax; check the manual that corresponds \
                   to your MySQL server version for the right syntax to use near \
                   'FORM users' at line 3";
        assert_eq!(line_from_message(msg), Some(ErrorPosition::line_only(3)));
    }

    #[test]
    fn multi_digit_line_and_trailing_whitespace() {
        assert_eq!(
            line_from_message("... near '' at line 128\n"),
            Some(ErrorPosition::line_only(128))
        );
    }

    #[test]
    fn the_last_marker_wins() {
        // The quoted fragment of the user's SQL can itself contain the phrase.
        assert_eq!(
            line_from_message("near ''x at line 9' FROM t' at line 2"),
            Some(ErrorPosition::line_only(2))
        );
    }

    #[test]
    fn messages_without_a_line() {
        assert_eq!(line_from_message("Table 'shop.nope' doesn't exist"), None);
        assert_eq!(
            line_from_message("Duplicate entry '1' for key 'PRIMARY'"),
            None
        );
        assert_eq!(
            line_from_message("Data truncated for column 'a' at row 1"),
            None
        );
        assert_eq!(line_from_message("stopped at line 3 of the file"), None);
        assert_eq!(line_from_message("near '' at line "), None);
        assert_eq!(line_from_message("near '' at line 0"), None);
        assert_eq!(line_from_message(""), None);
    }

    #[test]
    fn non_database_errors_carry_no_position() {
        assert_eq!(statement_error(&sqlx::Error::PoolTimedOut).position(), None);
    }
}
