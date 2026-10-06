//! Tests that drive `App` the way the event loop does — messages in, state
//! and rendered frame out — without a terminal.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;

use super::*;
use crate::core::error::ErrorPosition;
use crate::ui::state::Focus;

/// An app with one script tab holding `sql`, focused and ready to run.
fn app_with_script(sql: &str) -> (App, TabId) {
    let mut app = App::new();
    let tab_id = app.state.open_or_focus_tab(TabKind::Script {
        file_path: None,
        name: "Script 1".to_string(),
        conn_name: None,
        schema: None,
    });
    if let Some(editor) = app
        .state
        .find_tab_mut(tab_id)
        .and_then(|tab| tab.editor.as_mut())
    {
        editor.set_content(sql);
        editor.mode = vimltui::VimMode::Normal;
    }
    app.state.focus = Focus::TabContent;
    (app, tab_id)
}

/// Mark `tab_id` as running a query and return the run's id.
fn begin_run(app: &mut App, tab_id: TabId, new_tab: bool) -> u64 {
    app.next_run_id += 1;
    let run_id = app.next_run_id;
    let tab = app.state.find_tab_mut(tab_id).expect("tab exists");
    tab.query_run_id = run_id;
    tab.run_result_idx = (!new_tab).then_some(tab.active_result_idx);
    tab.streaming = true;
    tab.first_batch_pending = true;
    tab.pending_query = Some(("SELECT 1".to_string(), 0));
    run_id
}

fn batch(tab_id: TabId, run_id: u64, rows: &[&str], done: bool) -> AppMessage {
    AppMessage::QueryBatch {
        tab_id,
        run_id,
        columns: vec!["c".to_string()],
        rows: rows.iter().map(|r| vec![r.to_string()]).collect(),
        done,
        new_tab: false,
        elapsed: None,
    }
}

/// Render one frame and return the buffer.
fn draw(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    terminal
        .draw(|frame| crate::ui::layout::render(frame, &mut app.state, &app.theme))
        .expect("draw");
    terminal.backend().buffer().clone()
}

/// Screen rows as plain text.
fn screen_rows(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

#[test]
fn failed_query_marks_the_position_the_server_reported() {
    let sql = "SELECT *\nFROM users\nWHERE nope = 1";
    let (mut app, tab_id) = app_with_script(sql);
    let run_id = begin_run(&mut app, tab_id, false);

    app.handle_message(AppMessage::QueryFailed {
        tab_id,
        run_id,
        error: "Query failed: error returned from database: column \"nope\" does not exist"
            .to_string(),
        position: Some(ErrorPosition {
            line: 3,
            col: Some(7),
        }),
        query: sql.to_string(),
        new_tab: false,
        start_line: 0,
    });

    let tab = app.state.find_tab(tab_id).expect("tab exists");
    assert!(!tab.streaming, "a failed run is over");
    assert_eq!(tab.result_tabs.len(), 1, "the error gets a result tab");
    let mark = &tab.server_diagnostics[0];
    assert_eq!((mark.row, mark.col_start, mark.col_end), (2, 6, 10));
    assert_eq!(mark.message, "column \"nope\" does not exist");
    assert!(
        app.state.engine.diagnostics.iter().any(|d| d.row == 2),
        "the mark is on screen for the active tab"
    );

    // And it is drawn: the token is underlined in the editor, nothing else
    // on that line is.
    let buffer = draw(&mut app, 100, 30);
    let rows = screen_rows(&buffer);
    let y = rows
        .iter()
        .position(|row| row.contains("WHERE nope = 1"))
        .expect("the editor shows the failing line");
    let x = rows[y].find("nope").expect("token on screen");
    let x = rows[y][..x].chars().count() as u16;
    let underlined = |x: u16| {
        buffer[(x, y as u16)]
            .modifier
            .contains(Modifier::UNDERLINED)
    };
    assert!((x..x + 4).all(underlined), "`nope` is underlined");
    assert!(!underlined(x - 2) && !underlined(x + 5));
}

#[test]
fn a_clean_rerun_clears_the_error_mark() {
    let (mut app, tab_id) = app_with_script("SELECT 1");
    let run_id = begin_run(&mut app, tab_id, false);
    app.handle_message(AppMessage::QueryFailed {
        tab_id,
        run_id,
        error: "Query failed: boom".to_string(),
        position: None,
        query: "SELECT 1".to_string(),
        new_tab: false,
        start_line: 0,
    });
    assert_eq!(app.state.engine.diagnostics.len(), 1);

    let run_id = begin_run(&mut app, tab_id, false);
    app.handle_message(batch(tab_id, run_id, &["1"], true));

    let tab = app.state.find_tab(tab_id).expect("tab exists");
    assert!(tab.server_diagnostics.is_empty());
    assert!(app.state.engine.diagnostics.is_empty());
    assert_eq!(
        tab.result_tabs.len(),
        1,
        "the error tab was replaced in place"
    );
    assert!(tab.result_tabs[0].error_editor.is_none());
}

#[test]
fn results_of_a_superseded_run_are_dropped() {
    let (mut app, tab_id) = app_with_script("SELECT 1");
    let old_run = begin_run(&mut app, tab_id, false);
    let new_run = begin_run(&mut app, tab_id, false);

    app.handle_message(batch(tab_id, old_run, &["stale"], true));
    let tab = app.state.find_tab(tab_id).expect("tab exists");
    assert!(
        tab.result_tabs.is_empty(),
        "the old run must not create a result"
    );
    assert!(tab.streaming, "the new run is still in flight");

    app.handle_message(batch(tab_id, new_run, &["fresh"], true));
    let tab = app.state.find_tab(tab_id).expect("tab exists");
    assert_eq!(tab.result_tabs[0].result.rows, [["fresh"]]);
    assert!(!tab.streaming);
}

#[test]
fn a_stream_keeps_feeding_its_own_result_tab() {
    let (mut app, tab_id) = app_with_script("SELECT 1");
    // Two finished results; the second is selected.
    for _ in 0..2 {
        let run_id = begin_run(&mut app, tab_id, true);
        app.handle_message(batch(tab_id, run_id, &["old"], true));
    }
    let tab = app.state.find_tab_mut(tab_id).expect("tab exists");
    assert_eq!(tab.result_tabs.len(), 2);
    tab.active_result_idx = 0;

    // Re-run into result 1, then look at result 2 while it streams.
    let run_id = begin_run(&mut app, tab_id, false);
    app.handle_message(batch(tab_id, run_id, &["a"], false));
    app.state
        .find_tab_mut(tab_id)
        .expect("tab exists")
        .active_result_idx = 1;
    app.handle_message(batch(tab_id, run_id, &["b"], true));

    let tab = app.state.find_tab(tab_id).expect("tab exists");
    assert_eq!(tab.result_tabs[0].result.rows, [["a"], ["b"]]);
    assert_eq!(tab.result_tabs[1].result.rows, [["old"]]);
}

#[test]
fn connect_outcome_only_touches_the_dialog_that_started_it() {
    let mut app = App::new();
    // A half-typed form is open while some other connection fails.
    app.state.overlay = Some(Overlay::ConnectionDialog);
    app.state.dialogs.connection_form.name = "draft".to_string();

    app.handle_message(AppMessage::ConnectFailed {
        name: "other".to_string(),
        error: "Connection failed: refused".to_string(),
    });
    app.handle_message(AppMessage::Error("background fetch failed".to_string()));

    assert!(app.state.dialogs.connection_form.error_message.is_empty());
    assert!(
        app.state.dialogs.saved_connections.is_empty(),
        "nothing saved"
    );
    assert!(matches!(app.state.overlay, Some(Overlay::ConnectionDialog)));
}

#[test]
fn long_password_keeps_the_caret_and_its_length_on_screen() {
    let mut app = App::new();
    app.state.overlay = Some(Overlay::ConnectionDialog);
    let form = &mut app.state.dialogs.connection_form;
    form.name = "prod".to_string();
    form.password = "x".repeat(120);
    form.selected_field = 5;

    let rows = screen_rows(&draw(&mut app, 100, 40));
    let row = rows
        .iter()
        .find(|row| row.contains("Password"))
        .expect("password row");
    assert!(row.contains('█'), "caret visible: {row}");
    assert!(row.contains("120 chars"), "length shown: {row}");
    assert!(row.contains('…'), "the value is cut at its start: {row}");
    assert!(!row.contains('x'), "hidden by default: {row}");
}

/// End to end against a real PostgreSQL (see `drivers::live_tests` for the
/// environment variable): execute a failing statement the way a script tab
/// does and check the mark lands on the right line and token.
#[tokio::test]
#[ignore = "needs DBTUI_TEST_POSTGRES"]
async fn live_failed_statement_is_marked_in_the_editor() {
    let Ok(spec) = std::env::var("DBTUI_TEST_POSTGRES") else {
        return;
    };
    let mut parts = spec.splitn(4, ':');
    let (Some(host), Some(port), Some(user), Some(rest)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        panic!("DBTUI_TEST_POSTGRES must be host:port:user:password:database");
    };
    let (password, database) = rest.rsplit_once(':').expect("password:database");
    let config = ConnectionConfig {
        name: "live".to_string(),
        db_type: DatabaseType::PostgreSQL,
        host: host.to_string(),
        port: port.parse().expect("port"),
        username: user.to_string(),
        password: password.to_string(),
        database: Some(database.to_string()),
        group: "Default".to_string(),
    };
    let adapter: Arc<dyn DatabaseAdapter> = crate::drivers::create_adapter(&config)
        .await
        .expect("connect")
        .into();

    // Two blocks; the second one fails on its third line (buffer line 6).
    let sql = "SELECT 1;\n\n\nSELECT relname\nFROM pg_class\nWHERE nope = 1";
    let (mut app, tab_id) = app_with_script(sql);
    app.adapters.insert("live".to_string(), adapter);
    if let Some(tab) = app.state.find_tab_mut(tab_id)
        && let TabKind::Script { conn_name, .. } = &mut tab.kind
    {
        *conn_name = Some("live".to_string());
    }

    let statement = "SELECT relname\nFROM pg_class\nWHERE nope = 1";
    app.spawn_execute_query_at(tab_id, statement, false, 3);
    while app.state.find_tab(tab_id).is_some_and(|t| t.streaming) {
        let message = app.msg_rx.recv().await.expect("a message from the run");
        app.handle_message(message);
    }

    let tab = app.state.find_tab(tab_id).expect("tab exists");
    let mark = tab
        .server_diagnostics
        .first()
        .expect("the failure is marked");
    println!("mark: {mark:?}");
    assert_eq!(mark.row, 5, "third line of the block that starts on line 4");
    let line = &tab.editor.as_ref().expect("editor").lines[5];
    assert_eq!(&line[mark.col_start..mark.col_end], "nope");
    println!("status: {}", app.state.status_message);
}
