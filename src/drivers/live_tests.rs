//! Tests that talk to a real server. They are `#[ignore]`d and skip unless
//! the matching environment variable is set, so a plain `cargo test` never
//! needs a database:
//!
//! ```text
//! DBTUI_TEST_POSTGRES=host:port:user:password:database \
//! DBTUI_TEST_MYSQL=host:port:user:password:database \
//!     cargo test live_ -- --ignored --nocapture
//! ```
//!
//! The password is everything between the third colon and the last one, so
//! it may itself contain colons — these tests are how connecting with
//! awkward passwords is checked.

use tokio::sync::mpsc;

use crate::core::DatabaseAdapter;
use crate::core::error::DbResult;
use crate::core::models::{ConnectionConfig, DatabaseType, QueryResult};
use crate::drivers::create_adapter;

/// Parse `host:port:user:password:database` from the environment.
fn config_from_env(var: &str, db_type: DatabaseType) -> Option<ConnectionConfig> {
    let spec = std::env::var(var).ok()?;
    let (host, rest) = spec.split_once(':')?;
    let (port, rest) = rest.split_once(':')?;
    let (username, rest) = rest.split_once(':')?;
    let (password, database) = rest.rsplit_once(':')?;
    Some(ConnectionConfig {
        name: "live".to_string(),
        db_type,
        host: host.to_string(),
        port: port.parse().ok()?,
        username: username.to_string(),
        password: password.to_string(),
        database: Some(database.to_string()).filter(|d| !d.is_empty()),
        group: "Default".to_string(),
    })
}

async fn connect(var: &str, db_type: DatabaseType) -> Option<Box<dyn DatabaseAdapter>> {
    let config = config_from_env(var, db_type)?;
    match create_adapter(&config).await {
        Ok(adapter) => Some(adapter),
        Err(e) => panic!("connecting with {var}: {e}"),
    }
}

/// Run `sql` through the streaming entry point, the way a script tab does.
async fn stream(
    adapter: &dyn DatabaseAdapter,
    sql: &str,
    schema: Option<&str>,
) -> DbResult<QueryResult> {
    let (tx, mut rx) = mpsc::channel(4);
    let run = adapter.execute_streaming_in_schema(sql, schema, tx);
    let collect = async {
        let mut result = QueryResult {
            columns: vec![],
            rows: vec![],
            elapsed: None,
        };
        while let Some(batch) = rx.recv().await {
            let batch = batch?;
            result.columns = batch.columns;
            result.rows.extend(batch.rows);
        }
        Ok::<_, crate::core::error::DbError>(result)
    };
    let (ran, collected) = tokio::join!(run, collect);
    ran?;
    collected
}

async fn run(adapter: &dyn DatabaseAdapter, sql: &str) -> QueryResult {
    match stream(adapter, sql, None).await {
        Ok(result) => result,
        Err(e) => panic!("{sql}\n  failed: {e}"),
    }
}

fn cell(result: &QueryResult, row: usize, col: usize) -> &str {
    &result.rows[row][col]
}

#[tokio::test]
#[ignore = "needs DBTUI_TEST_POSTGRES"]
async fn live_postgres() {
    let Some(adapter) = connect("DBTUI_TEST_POSTGRES", DatabaseType::PostgreSQL).await else {
        return;
    };
    let db = adapter.as_ref();

    run(db, "DROP SCHEMA IF EXISTS dbtui_live CASCADE").await;
    run(db, "CREATE SCHEMA dbtui_live").await;
    run(
        db,
        "CREATE TABLE dbtui_live.users (id int PRIMARY KEY, name text NOT NULL)",
    )
    .await;
    run(
        db,
        "CREATE TABLE dbtui_live.roles (id int PRIMARY KEY, label text)",
    )
    .await;
    run(
        db,
        "CREATE TABLE dbtui_live.user_roles (\
           user_id int REFERENCES dbtui_live.users (id), \
           role_id int REFERENCES dbtui_live.roles (id), \
           PRIMARY KEY (user_id, role_id))",
    )
    .await;

    // DML reports a count; RETURNING and friends report rows.
    let inserted = run(
        db,
        "INSERT INTO dbtui_live.users VALUES (1, 'ana'), (2, 'José')",
    )
    .await;
    assert!(cell(&inserted, 0, 0).contains("2 row"), "{inserted:?}");
    let returning = run(
        db,
        "INSERT INTO dbtui_live.users VALUES (3, 'zoe') RETURNING id, name",
    )
    .await;
    assert_eq!(returning.columns, ["id", "name"]);
    assert_eq!(returning.rows, [["3", "zoe"]]);

    // A row-returning statement that writes must persist its write.
    let deleted = run(
        db,
        "WITH d AS (DELETE FROM dbtui_live.users WHERE id = 3 RETURNING *) SELECT * FROM d",
    )
    .await;
    assert_eq!(deleted.rows.len(), 1);
    let count = run(db, "SELECT count(*) FROM dbtui_live.users").await;
    assert_eq!(
        cell(&count, 0, 0),
        "2",
        "the DELETE inside the CTE was rolled back"
    );

    // An empty result still has headers.
    let empty = run(db, "SELECT id, name FROM dbtui_live.users WHERE false").await;
    assert_eq!(empty.columns, ["id", "name"]);
    assert!(empty.rows.is_empty());

    // Statements that are not SELECT/WITH but return rows.
    let explain = run(db, "EXPLAIN SELECT * FROM dbtui_live.users").await;
    assert!(!explain.rows.is_empty(), "{explain:?}");
    let show = run(db, "SHOW server_version").await;
    assert_eq!(show.rows.len(), 1);
    let values = run(db, "VALUES (1, 'a'), (2, 'b')").await;
    assert_eq!(values.rows.len(), 2);

    // Per-script schema applies to queries and to DML alike.
    let scoped = stream(db, "SELECT name FROM users ORDER BY id", Some("dbtui_live"))
        .await
        .expect("unqualified name resolves through the script schema");
    assert_eq!(scoped.rows, [["ana"], ["José"]]);
    stream(
        db,
        "UPDATE users SET name = 'Ana' WHERE id = 1",
        Some("dbtui_live"),
    )
    .await
    .expect("DML honours the script schema");
    let renamed = run(db, "SELECT name FROM dbtui_live.users WHERE id = 1").await;
    assert_eq!(cell(&renamed, 0, 0), "Ana");

    // Statements PostgreSQL refuses inside a transaction block.
    run(db, "VACUUM dbtui_live.users").await;

    // Values that used to panic, vanish or lose precision.
    let odd = run(
        db,
        "SELECT 'infinity'::timestamptz, '-infinity'::date, \
                '2024-01-02 03:04:05.678901'::timestamp, \
                '-1 hour 30 minutes'::interval, 1.50::numeric(10,2), \
                'NaN'::numeric, 1.5::real, ARRAY[1,2,3], \
                '{\"a\": 1}'::jsonb, NULL::int, true, \
                'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'::uuid",
    )
    .await;
    println!("postgres values: {:?}", odd.rows[0]);
    assert_eq!(cell(&odd, 0, 0), "infinity");
    assert_eq!(cell(&odd, 0, 1), "-infinity");
    assert_eq!(cell(&odd, 0, 2), "2024-01-02 03:04:05.678901");
    assert_eq!(cell(&odd, 0, 3), "-00:30:00");
    assert_eq!(cell(&odd, 0, 4), "1.50");
    assert_eq!(cell(&odd, 0, 5), "NaN");
    assert_eq!(cell(&odd, 0, 6), "1.5");
    assert_eq!(cell(&odd, 0, 9), "NULL");
    assert_eq!(cell(&odd, 0, 11), "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11");

    // The server's error position reaches the caller, in statement lines.
    let err = stream(db, "SELECT *\nFROM dbtui_live.users\nWHERE nope = 1", None)
        .await
        .expect_err("unknown column");
    let position = err.position().expect("PostgreSQL reports a position");
    println!("postgres error: {err} at {position:?}");
    assert_eq!((position.line, position.col), (3, Some(7)));

    // Metadata.
    let columns = db
        .get_columns("dbtui_live", "user_roles")
        .await
        .expect("columns");
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["user_id", "role_id"], "one row per column");
    assert!(columns.iter().all(|c| c.is_primary_key));
    let fks = db
        .get_foreign_keys("dbtui_live", "user_roles")
        .await
        .expect("foreign keys");
    println!("postgres fks: {fks:?}");
    assert_eq!(fks.len(), 2);
    let ddl = db
        .get_table_ddl("dbtui_live", "user_roles")
        .await
        .expect("ddl");
    println!("postgres ddl:\n{ddl}");
    assert!(
        ddl.contains("integer") && !ddl.contains("integer("),
        "{ddl}"
    );

    run(db, "DROP SCHEMA dbtui_live CASCADE").await;
}

#[tokio::test]
#[ignore = "needs DBTUI_TEST_MYSQL"]
async fn live_mysql() {
    let Some(adapter) = connect("DBTUI_TEST_MYSQL", DatabaseType::MySQL).await else {
        return;
    };
    let db = adapter.as_ref();

    run(db, "DROP DATABASE IF EXISTS dbtui_live").await;
    run(db, "CREATE DATABASE dbtui_live").await;
    run(
        db,
        "CREATE TABLE dbtui_live.t (\
           id INT UNSIGNED PRIMARY KEY, f FLOAT, d DOUBLE, y YEAR, tm TIME, \
           dt DATETIME(6), dec_col DECIMAL(10,2), flag TINYINT(1), bits BIT(4), \
           big BIGINT UNSIGNED, bin VARBINARY(4), doc JSON, note VARCHAR(40))",
    )
    .await;

    let inserted = run(
        db,
        "INSERT INTO dbtui_live.t VALUES \
           (1, 1.5, 2.25, 2024, '48:00:00', '2024-01-02 03:04:05.678901', 12.30, 5, b'1010', \
            18446744073709551615, x'DEADBEEF', '{\"a\": 1}', 'José'), \
           (2, NULL, NULL, NULL, '-01:30:00', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
    )
    .await;
    assert!(cell(&inserted, 0, 0).contains("2 row"), "{inserted:?}");

    let rows = run(db, "SELECT * FROM dbtui_live.t ORDER BY id").await;
    println!("mysql row 1: {:?}", rows.rows[0]);
    println!("mysql row 2: {:?}", rows.rows[1]);
    assert_eq!(rows.columns.len(), 13);
    assert_eq!(cell(&rows, 0, 0), "1");
    assert_eq!(
        cell(&rows, 0, 1),
        "1.5",
        "FLOAT used to panic the query task"
    );
    assert_eq!(cell(&rows, 0, 2), "2.25");
    assert_eq!(cell(&rows, 0, 3), "2024");
    assert_eq!(cell(&rows, 0, 4), "48:00:00");
    assert_eq!(cell(&rows, 0, 5), "2024-01-02 03:04:05.678901");
    assert_eq!(cell(&rows, 0, 6), "12.30");
    assert_eq!(cell(&rows, 0, 7), "5");
    assert_eq!(cell(&rows, 0, 9), "18446744073709551615");
    assert_eq!(cell(&rows, 0, 12), "José");
    assert_eq!(cell(&rows, 1, 1), "NULL");
    assert_eq!(cell(&rows, 1, 4), "-01:30:00");

    // An empty result still has headers.
    let empty = run(db, "SELECT id, note FROM dbtui_live.t WHERE 1 = 0").await;
    assert_eq!(empty.columns, ["id", "note"]);
    assert!(empty.rows.is_empty());

    // Row-returning statements that are not SELECT.
    let show = run(db, "SHOW TABLES FROM dbtui_live").await;
    assert_eq!(show.rows, [["t"]]);
    let describe = run(db, "DESCRIBE dbtui_live.t").await;
    assert_eq!(describe.rows.len(), 13);
    let explain = run(db, "EXPLAIN SELECT * FROM dbtui_live.t").await;
    assert!(!explain.rows.is_empty());

    // Statements the prepared-statement protocol refuses.
    run(
        db,
        "CREATE PROCEDURE dbtui_live.list_ids() BEGIN SELECT id FROM dbtui_live.t ORDER BY id; END",
    )
    .await;
    let called = run(db, "CALL dbtui_live.list_ids()").await;
    assert_eq!(called.rows, [["1"], ["2"]]);

    // DML is committed.
    let updated = run(db, "UPDATE dbtui_live.t SET note = 'x' WHERE id = 2").await;
    assert!(cell(&updated, 0, 0).contains("1 row"), "{updated:?}");
    let note = run(db, "SELECT note FROM dbtui_live.t WHERE id = 2").await;
    assert_eq!(cell(&note, 0, 0), "x");

    // The server's error line reaches the caller.
    let err = stream(db, "SELECT *\nFROM dbtui_live.t\nWHERE (", None)
        .await
        .expect_err("syntax error");
    println!("mysql error: {err} at {:?}", err.position());
    assert_eq!(err.position().map(|p| p.line), Some(3));

    // Metadata still works through the prepared path.
    let columns = db.get_columns("dbtui_live", "t").await.expect("columns");
    assert_eq!(columns.len(), 13);
    assert!(columns[0].is_primary_key);
    let ddl = db.get_table_ddl("dbtui_live", "t").await.expect("ddl");
    assert!(ddl.contains("CREATE TABLE"), "{ddl}");

    run(db, "DROP DATABASE dbtui_live").await;
}
