//! PostgreSQL statements that cannot run inside a transaction block.

use crate::core::models::DatabaseType;
use crate::drivers::statement::bare_words;

/// How many leading words are inspected for `CONCURRENTLY`. It always sits
/// before the object name: `CREATE UNIQUE INDEX CONCURRENTLY`,
/// `REINDEX (VERBOSE) TABLE CONCURRENTLY`.
const CONCURRENTLY_WINDOW: usize = 6;

/// Whether PostgreSQL rejects `sql` with "cannot run inside a transaction
/// block", so the adapter must send it on a connection in autocommit mode.
///
/// Statements this misses are still handled: the server answers them with
/// SQLSTATE 25001 before doing any work and the adapter retries outside the
/// transaction. Recognising the common ones up front just saves that round
/// trip.
pub(super) fn runs_outside_transaction(sql: &str) -> bool {
    let words = bare_words(DatabaseType::PostgreSQL, sql);
    let word = |i: usize| words.get(i).map(String::as_str).unwrap_or_default();
    let concurrently = words
        .iter()
        .take(CONCURRENTLY_WINDOW)
        .any(|w| w == "CONCURRENTLY");

    match (word(0), word(1)) {
        ("VACUUM", _) => true,
        ("CREATE" | "DROP", "DATABASE" | "TABLESPACE") => true,
        ("ALTER", "SYSTEM") => true,
        ("CREATE" | "DROP", _) => {
            concurrently && words.iter().take(CONCURRENTLY_WINDOW).any(|w| w == "INDEX")
        }
        ("REINDEX", _) => concurrently,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::runs_outside_transaction;

    #[test]
    fn maintenance_and_cluster_wide_statements() {
        for sql in [
            "VACUUM",
            "vacuum (analyze, verbose) public.t",
            "-- tidy up\nVACUUM FULL t",
            "CREATE DATABASE shop",
            "DROP DATABASE IF EXISTS shop",
            "CREATE TABLESPACE fast LOCATION '/mnt/ssd'",
            "DROP TABLESPACE fast",
            "ALTER SYSTEM SET work_mem = '64MB'",
        ] {
            assert!(runs_outside_transaction(sql), "{sql}");
        }
    }

    #[test]
    fn concurrent_index_operations() {
        for sql in [
            "CREATE INDEX CONCURRENTLY idx ON t (a)",
            "create unique index concurrently if not exists idx on t (a)",
            "DROP INDEX CONCURRENTLY idx",
            "REINDEX INDEX CONCURRENTLY idx",
            "REINDEX (VERBOSE) TABLE CONCURRENTLY t",
        ] {
            assert!(runs_outside_transaction(sql), "{sql}");
        }
    }

    #[test]
    fn ordinary_statements_stay_transactional() {
        for sql in [
            "SELECT 1",
            "CREATE INDEX idx ON t (a)",
            "DROP INDEX idx",
            "REINDEX TABLE t",
            "CREATE TABLE database (id int)",
            "ALTER TABLE t ADD COLUMN system int",
            "REFRESH MATERIALIZED VIEW CONCURRENTLY mv",
            "INSERT INTO notes VALUES ('VACUUM')",
            "/* VACUUM */ UPDATE t SET a = 1",
            "CREATE TABLE t (note text DEFAULT 'index concurrently')",
            "",
        ] {
            assert!(!runs_outside_transaction(sql), "{sql}");
        }
    }
}
