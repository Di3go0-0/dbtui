//! tiberius error → `DbError` conversion for user statements.

use tiberius::error::Error;

use crate::core::error::{DbError, ErrorPosition};

/// Convert an error raised while running the user's batch.
///
/// A server error carries the line it was raised on. That line counts from
/// the start of the submitted text both for a raw batch and for
/// `sp_executesql` — the statement is the procedure's `@stmt` argument and
/// forms a batch of its own — so it maps onto the editor without adjustment.
pub(super) fn statement_error(err: &Error) -> DbError {
    match err {
        Error::Server(token) => DbError::query_failed(
            server_message(
                token.message(),
                token.code(),
                token.class(),
                token.state(),
                token.line(),
                token.procedure(),
            ),
            batch_line(token.procedure(), token.line()),
        ),
        other => DbError::QueryFailed(other.to_string()),
    }
}

/// The server's message followed by the coordinates SSMS prints with it.
fn server_message(
    message: &str,
    code: u32,
    class: u8,
    state: u8,
    line: u32,
    procedure: &str,
) -> String {
    let mut out = format!("{message} (Msg {code}, Level {class}, State {state}");
    if !procedure.is_empty() {
        out.push_str(&format!(", Procedure {procedure}"));
    }
    if line > 0 {
        out.push_str(&format!(", Line {line}"));
    }
    out.push(')');
    out
}

/// The failing line of the submitted batch, when the server's line refers to
/// it. An error raised inside a stored procedure, function or trigger reports
/// a line of that module's source instead, and line 0 means "not applicable".
fn batch_line(procedure: &str, line: u32) -> Option<ErrorPosition> {
    if !procedure.is_empty() || line == 0 {
        return None;
    }
    Some(ErrorPosition::line_only(usize::try_from(line).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_error_maps_to_its_line() {
        assert_eq!(batch_line("", 3), Some(ErrorPosition::line_only(3)));
        assert_eq!(batch_line("", 1), Some(ErrorPosition::line_only(1)));
    }

    #[test]
    fn module_errors_and_line_zero_have_no_position() {
        assert_eq!(batch_line("dbo.usp_report", 12), None);
        assert_eq!(batch_line("", 0), None);
    }

    #[test]
    fn message_for_a_batch_error() {
        assert_eq!(
            server_message("Invalid object name 'nope'.", 208, 16, 1, 2, ""),
            "Invalid object name 'nope'. (Msg 208, Level 16, State 1, Line 2)"
        );
    }

    #[test]
    fn message_names_the_module_that_raised_it() {
        assert_eq!(
            server_message(
                "Divide by zero error encountered.",
                8134,
                16,
                1,
                7,
                "usp_calc"
            ),
            "Divide by zero error encountered. (Msg 8134, Level 16, State 1, Procedure usp_calc, Line 7)"
        );
    }

    #[test]
    fn message_without_a_line() {
        assert_eq!(
            server_message("Login failed.", 18456, 14, 1, 0, ""),
            "Login failed. (Msg 18456, Level 14, State 1)"
        );
    }

    #[test]
    fn non_server_errors_carry_no_position() {
        let err = Error::Protocol("unexpected token".into());
        assert_eq!(statement_error(&err).position(), None);
    }
}
