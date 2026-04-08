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

/// Translate a raw driver error string into a friendlier multi-line message
/// for the Connection dialog. First line is a short headline, following
/// lines are the raw driver message and (when recognised) an actionable
/// hint prefixed with `Hint:`.
///
/// The string format is a plain `\n`-separated message so it can flow
/// through `DbError::ConnectionFailed(String)` without schema changes.
/// The connection dialog splits on `\n` to render each line distinctly.
pub fn friendly_connect_error(db_type: crate::core::models::DatabaseType, raw: &str) -> String {
    use crate::core::models::DatabaseType;

    let lc = raw.to_lowercase();

    // Oracle ORA-NNNNN code lookup — `oracle` crate embeds the code in its
    // Display impl, so a simple substring match is reliable.
    if matches!(db_type, DatabaseType::Oracle)
        && let Some((title, hint)) = oracle_hint(&lc)
    {
        return format_error(title, raw, Some(hint));
    }

    // Generic network / auth patterns — shared by sqlx (Postgres/MySQL) and
    // the low-level OS error strings from tokio/oracle.
    if lc.contains("connection refused")
        || lc.contains("no route to host")
        || lc.contains("network is unreachable")
    {
        return format_error(
            "Can't reach the server",
            raw,
            Some(
                "Check the host/port and that the database is running and accepts remote connections.",
            ),
        );
    }
    if lc.contains("password authentication failed")
        || lc.contains("access denied for user")
        || lc.contains("authentication failed")
    {
        return format_error(
            "Invalid credentials",
            raw,
            Some("The server rejected the username or password."),
        );
    }
    if lc.contains("timed out") || lc.contains("timeout") || lc.contains("timer expired") {
        return format_error(
            "Connection timeout",
            raw,
            Some("The server didn't answer in time — firewall, VPN, or wrong host?"),
        );
    }
    if lc.contains("no such host")
        || lc.contains("failed to lookup address")
        || lc.contains("name or service not known")
        || lc.contains("nodename nor servname")
    {
        return format_error(
            "Host not found",
            raw,
            Some("DNS couldn't resolve the host — check the host field for typos."),
        );
    }
    if (lc.contains("database") && lc.contains("does not exist")) || lc.contains("unknown database")
    {
        return format_error(
            "Database does not exist",
            raw,
            Some("Check the database/schema name in the connection form."),
        );
    }
    if lc.contains("ssl") || lc.contains("tls") {
        return format_error(
            "SSL/TLS error",
            raw,
            Some("Server may require SSL, or the certificate could not be verified."),
        );
    }
    if lc.contains("too many connections") {
        return format_error(
            "Too many connections",
            raw,
            Some("The server has reached its connection limit — retry shortly."),
        );
    }
    if lc.contains("role") && lc.contains("does not exist") {
        return format_error(
            "Unknown role",
            raw,
            Some("Verify the username exists on the server."),
        );
    }

    // Fallback — keep the raw message verbatim so power users still see it.
    format_error("Connection failed", raw, None)
}

fn oracle_hint(lc: &str) -> Option<(&'static str, &'static str)> {
    // Order matters: more-specific codes first.
    let table: &[(&str, &str, &str)] = &[
        // ─── DPI / ODPI-C client-library errors ───
        // These are raised by the `oracle` crate's underlying ODPI-C layer
        // BEFORE any network round-trip, so they indicate the local
        // Oracle Instant Client install is missing or broken — not an
        // actual server problem.
        (
            "dpi-1047",
            "Oracle Instant Client not installed",
            "dbtui needs the Oracle Instant Client to talk to Oracle. Install it (e.g. `oracle-instantclient-basic` on Arch / AUR, or download from oracle.com) and make sure `libclntsh.so` is in `LD_LIBRARY_PATH` (Linux) or `DYLD_LIBRARY_PATH` (macOS). PostgreSQL and MySQL connections do NOT need this.",
        ),
        (
            "dpi-1050",
            "Oracle Instant Client too old",
            "Your installed Oracle Client library is older than what dbtui needs. Install a newer Instant Client (11.2 or above) and update `LD_LIBRARY_PATH`.",
        ),
        (
            "dpi-1072",
            "Oracle Client library not supported",
            "The Oracle Client library found on this machine isn't supported. Install a newer Instant Client (11.2+) and verify `LD_LIBRARY_PATH` points to it.",
        ),
        // Generic substring fallbacks for the same condition — some
        // distros surface the dynamic-loader error before ODPI-C can
        // emit its own DPI-1047 message.
        (
            "libclntsh",
            "Oracle Instant Client not installed",
            "The dynamic loader couldn't find `libclntsh` — install the Oracle Instant Client and add it to `LD_LIBRARY_PATH`. PostgreSQL and MySQL connections do NOT need this.",
        ),
        (
            "cannot locate a 64-bit oracle client",
            "Oracle Instant Client not installed",
            "Install the Oracle Instant Client (64-bit) and make sure `libclntsh.so` is in `LD_LIBRARY_PATH`. PostgreSQL and MySQL connections do NOT need this.",
        ),
        (
            "ora-12541",
            "Listener not running",
            "The Oracle TNS listener isn't reachable — verify host/port and that the listener is up.",
        ),
        (
            "ora-12514",
            "Unknown service name",
            "Verify the Service Name / SID in the connection string — the listener doesn't know this service.",
        ),
        (
            "ora-12505",
            "Unknown SID",
            "The listener doesn't know this SID — check Database field or switch to Service Name.",
        ),
        (
            "ora-01017",
            "Invalid username/password",
            "Oracle rejected your credentials. Note: passwords may be case-sensitive.",
        ),
        (
            "ora-28000",
            "Account locked",
            "The Oracle account is locked — ask a DBA to run `ALTER USER <user> ACCOUNT UNLOCK`.",
        ),
        (
            "ora-28001",
            "Password has expired",
            "The password must be changed on the server before you can connect.",
        ),
        (
            "ora-12545",
            "Host not found",
            "DNS couldn't resolve the target host — verify it's correct.",
        ),
        (
            "ora-12170",
            "TNS connect timeout",
            "Firewall/VPN may be dropping packets to the listener port.",
        ),
        (
            "ora-12154",
            "TNS could not resolve connect identifier",
            "Check the service name / connect string — it didn't match any TNS alias.",
        ),
        (
            "ora-12537",
            "TNS connection closed",
            "The listener accepted the connection then dropped it — possible misconfiguration on the server.",
        ),
        (
            "ora-12560",
            "TNS protocol adapter error",
            "The Oracle client couldn't initialise — check ORACLE_HOME / instant client install.",
        ),
        (
            "ora-01034",
            "Oracle not available",
            "The database instance is down (not just the listener).",
        ),
        (
            "ora-01005",
            "Null password given",
            "Password field is empty.",
        ),
        (
            "ora-00942",
            "Table or view does not exist",
            "The credentials worked, but the schema lacks the expected objects.",
        ),
    ];
    for (code, title, hint) in table {
        if lc.contains(code) {
            return Some((*title, *hint));
        }
    }
    None
}

fn format_error(title: &str, detail: &str, hint: Option<&str>) -> String {
    let detail = detail.trim();
    let mut out = title.to_string();
    if !detail.is_empty() && !detail.eq_ignore_ascii_case(title) {
        out.push('\n');
        out.push_str(detail);
    }
    if let Some(h) = hint {
        out.push('\n');
        out.push_str("Hint: ");
        out.push_str(h);
    }
    out
}

#[cfg(test)]
mod friendly_tests {
    use super::*;
    use crate::core::models::DatabaseType;

    #[test]
    fn oracle_ora_01017_invalid_credentials() {
        let msg = friendly_connect_error(
            DatabaseType::Oracle,
            "ORA-01017: invalid username/password; logon denied",
        );
        assert!(msg.starts_with("Invalid username/password"));
        assert!(msg.contains("Hint:"));
    }

    #[test]
    fn oracle_listener_down() {
        let msg = friendly_connect_error(DatabaseType::Oracle, "ORA-12541: TNS:no listener");
        assert!(msg.starts_with("Listener not running"));
    }

    #[test]
    fn postgres_auth_failure() {
        let msg = friendly_connect_error(
            DatabaseType::PostgreSQL,
            "error returned from database: password authentication failed for user \"bob\"",
        );
        assert!(msg.starts_with("Invalid credentials"));
    }

    #[test]
    fn network_refused() {
        let msg = friendly_connect_error(
            DatabaseType::PostgreSQL,
            "error communicating with the server: Connection refused (os error 111)",
        );
        assert!(msg.starts_with("Can't reach the server"));
    }

    #[test]
    fn oracle_dpi_1047_missing_client() {
        // Real ODPI-C error string when libclntsh isn't on the loader path.
        let raw = "DPI-1047: Cannot locate a 64-bit Oracle Client library: \
                   \"libclntsh.so: cannot open shared object file: No such file or directory\". \
                   See https://oracle.github.io/odpi/doc/installation.html for help";
        let msg = friendly_connect_error(DatabaseType::Oracle, raw);
        assert!(msg.starts_with("Oracle Instant Client not installed"));
        assert!(msg.contains("Hint:"));
        assert!(msg.contains("LD_LIBRARY_PATH"));
    }

    #[test]
    fn oracle_libclntsh_loader_error() {
        // Some loaders surface a bare libclntsh error before ODPI-C
        // can wrap it in a DPI code.
        let raw = "error while loading shared libraries: libclntsh.so.21.1: \
                   cannot open shared object file: No such file or directory";
        let msg = friendly_connect_error(DatabaseType::Oracle, raw);
        assert!(msg.starts_with("Oracle Instant Client not installed"));
    }

    #[test]
    fn fallback_keeps_raw() {
        let msg = friendly_connect_error(DatabaseType::MySQL, "something truly weird");
        assert!(msg.starts_with("Connection failed"));
        assert!(msg.contains("something truly weird"));
    }
}
