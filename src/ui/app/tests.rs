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

// --- ER diagram ---------------------------------------------------------

fn fk(table: &str, col: &str, ref_table: &str) -> ForeignKeyInfo {
    ForeignKeyInfo {
        constraint_name: format!("fk_{table}_{col}"),
        table_name: table.into(),
        column_name: col.into(),
        referenced_schema: "app".into(),
        referenced_table: ref_table.into(),
        referenced_column: "id".into(),
    }
}

/// A diagram tab of a three-table schema with its data delivered.
fn app_with_diagram() -> (App, TabId) {
    let mut app = App::new();
    let tab_id = app.state.open_or_focus_tab(TabKind::Diagram {
        conn_name: "db".into(),
        schema: "app".into(),
    });
    app.state.focus = Focus::TabContent;
    app.handle_message(AppMessage::DiagramLoaded {
        tab_id,
        tables: vec!["users".into(), "orders".into(), "order_items".into()],
        fks: vec![
            fk("orders", "user_id", "users"),
            fk("order_items", "order_id", "orders"),
        ],
    });
    app.handle_message(AppMessage::DiagramColumnsLoaded {
        tab_id,
        table: "orders".into(),
        columns: vec![
            Column {
                name: "id".into(),
                data_type: "integer".into(),
                nullable: false,
                is_primary_key: true,
            },
            Column {
                name: "user_id".into(),
                data_type: "integer".into(),
                nullable: true,
                is_primary_key: false,
            },
        ],
    });
    (app, tab_id)
}

fn key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
}

fn press(app: &mut App, c: char) {
    let action = events::handle_key(&mut app.state, key(crossterm::event::KeyCode::Char(c)));
    app.dispatch_action(action);
}

#[test]
fn diagram_tab_draws_tables_edges_and_columns() {
    let (mut app, _) = app_with_diagram();
    let buffer = draw(&mut app, 120, 30);
    let rows = screen_rows(&buffer);
    let screen = rows.join("\n");
    for name in ["users", "orders", "order_items"] {
        assert!(screen.contains(name), "{name} is drawn:\n{screen}");
    }
    assert!(screen.contains("\u{25cf} id"), "primary key row:\n{screen}");
    assert!(
        screen.contains("\u{25cb} user_id"),
        "foreign key row:\n{screen}"
    );
    assert_eq!(
        screen.matches('\u{25c4}').count(),
        2,
        "one arrow per key:\n{screen}"
    );
    assert!(screen.contains("3 tables"), "status line:\n{screen}");
}

#[test]
fn diagram_keys_select_focus_and_search() {
    let (mut app, tab_id) = app_with_diagram();
    let selected = |app: &App| {
        app.state
            .find_tab(tab_id)
            .and_then(|t| t.diagram.as_ref())
            .and_then(|d| d.selected.clone())
    };
    assert_eq!(
        selected(&app).as_deref(),
        Some("users"),
        "first box in reading order"
    );

    press(&mut app, 'w');
    assert_eq!(selected(&app).as_deref(), Some("orders"));

    // Enter walks the columns; Enter on the user_id key jumps to users.
    let enter = || key(crossterm::event::KeyCode::Enter);
    let action = events::handle_key(&mut app.state, enter());
    app.dispatch_action(action);
    let cursor = |app: &App| {
        app.state
            .find_tab(tab_id)
            .and_then(|t| t.diagram.as_ref())
            .and_then(|d| d.column_cursor)
    };
    assert_eq!(cursor(&app), Some(0));
    press(&mut app, 'j');
    assert_eq!(cursor(&app), Some(1));
    let action = events::handle_key(&mut app.state, enter());
    app.dispatch_action(action);
    assert_eq!(selected(&app).as_deref(), Some("users"));
    let action = events::handle_key(&mut app.state, key(crossterm::event::KeyCode::Esc));
    app.dispatch_action(action);
    assert_eq!(cursor(&app), None);
    press(&mut app, 'w');
    assert_eq!(selected(&app).as_deref(), Some("orders"));

    // <leader>Enter focuses the selected table.
    press(&mut app, ' ');
    let action = events::handle_key(&mut app.state, enter());
    app.dispatch_action(action);
    let focus = app
        .state
        .find_tab(tab_id)
        .and_then(|t| t.diagram.as_ref())
        .and_then(|d| d.focus.clone());
    assert_eq!(focus.as_deref(), Some("orders"));

    // `f` toggles between the focused neighbourhood and the whole schema.
    press(&mut app, 'f');
    let focus_of = |app: &App| {
        app.state
            .find_tab(tab_id)
            .and_then(|t| t.diagram.as_ref())
            .and_then(|d| d.focus.clone())
    };
    assert_eq!(focus_of(&app), None);
    press(&mut app, 'f');
    assert_eq!(focus_of(&app).as_deref(), Some("orders"));

    // `/` reads a name; its letters must not reach the global handlers.
    press(&mut app, '/');
    for c in "users".chars() {
        press(&mut app, c);
    }
    assert!(
        app.state.overlay.is_none(),
        "`?`-style globals stay off while typing"
    );
    let action = events::handle_key(&mut app.state, key(crossterm::event::KeyCode::Enter));
    app.dispatch_action(action);
    let diagram = app
        .state
        .find_tab(tab_id)
        .and_then(|t| t.diagram.as_ref())
        .expect("diagram");
    assert!(diagram.search.is_none());
    assert_eq!(diagram.selected.as_deref(), Some("users"));
    assert_eq!(diagram.last_search, "users");

    // `o` opens the selected table as a tab.
    press(&mut app, 'o');
    assert!(app.state.tabs.iter().any(|t| matches!(
        &t.kind,
        TabKind::Table { table, schema, .. } if table == "users" && schema == "app"
    )));
}

#[test]
fn table_relations_view_draws_the_table_with_its_neighbours() {
    let mut app = App::new();
    let tab_id = app.state.open_or_focus_tab(TabKind::Table {
        conn_name: "db".into(),
        schema: "app".into(),
        table: "orders".into(),
    });
    app.state.focus = Focus::TabContent;
    // The table's rows are still streaming in; that must not hold the
    // Relations diagram back.
    if let Some(tab) = app.state.find_tab_mut(tab_id) {
        tab.streaming = true;
        tab.streaming_since = Some(std::time::Instant::now());
    }
    // Data -> Properties -> Relations: entering the view asks for the load.
    press(&mut app, ']');
    press(&mut app, ']');
    let tab = app.state.find_tab(tab_id).expect("tab");
    assert_eq!(
        tab.active_sub_view,
        Some(crate::ui::tabs::SubView::TableRelations)
    );
    let diagram = tab
        .diagram
        .as_ref()
        .expect("diagram state created on entry");
    assert_eq!(diagram.focus.as_deref(), Some("orders"));

    app.handle_message(AppMessage::DiagramLoaded {
        tab_id,
        tables: vec![
            "users".into(),
            "orders".into(),
            "order_items".into(),
            "products".into(),
        ],
        fks: vec![
            fk("orders", "user_id", "users"),
            fk("order_items", "order_id", "orders"),
            fk("order_items", "product_id", "products"),
        ],
    });
    let screen = screen_rows(&draw(&mut app, 120, 30)).join("\n");
    for name in ["users", "orders", "order_items"] {
        assert!(screen.contains(name), "{name} is a neighbour:\n{screen}");
    }
    assert!(
        !screen.contains("products"),
        "two hops away stays out:\n{screen}"
    );
    assert_eq!(screen.matches('\u{25c4}').count(), 2, "{screen}");

    // The diagram keys work here too: `o` opens the selected table.
    press(&mut app, 'w');
    press(&mut app, 'o');
    assert!(app.state.tabs.iter().any(|t| matches!(
        &t.kind,
        TabKind::Table { table, .. } if table != "orders"
    )));
}

#[test]
fn leader_v_d_opens_the_diagram_of_the_active_table() {
    let mut app = App::new();
    app.state.open_or_focus_tab(TabKind::Table {
        conn_name: "db".into(),
        schema: "app".into(),
        table: "orders".into(),
    });
    app.state.focus = Focus::TabContent;
    press(&mut app, ' ');
    assert!(app.state.leader.pending);
    press(&mut app, 'v');
    assert!(app.state.leader.v_pending);
    press(&mut app, 'd');
    let diagram = app
        .state
        .tabs
        .iter()
        .find(|t| matches!(&t.kind, TabKind::Diagram { schema, .. } if schema == "app"))
        .expect("diagram tab opened");
    assert_eq!(
        diagram.diagram.as_ref().and_then(|d| d.focus.as_deref()),
        Some("orders")
    );
    assert!(!app.state.leader.v_pending);
}

#[test]
fn the_connected_database_expands_by_itself() {
    use crate::core::models::{Catalog, ConnectionConfig, DatabaseType};
    use crate::ui::state::TreeNode;
    let mut app = App::new();
    app.state.dialogs.saved_connections.push(ConnectionConfig {
        name: "pg".into(),
        db_type: DatabaseType::PostgreSQL,
        host: "localhost".into(),
        port: 5432,
        username: "me".into(),
        password: String::new(),
        database: Some("identity".into()),
        group: "Default".into(),
    });
    app.state.sidebar.tree.push(TreeNode::Connection {
        name: "pg".into(),
        expanded: true,
        status: crate::ui::state::ConnStatus::Connected,
    });
    app.handle_message(AppMessage::CatalogsLoaded {
        conn_name: "pg".into(),
        catalogs: ["catalog", "identity", "postgres"]
            .iter()
            .map(|n| Catalog {
                name: n.to_string(),
            })
            .collect(),
    });
    let expanded: Vec<(String, bool)> = app
        .state
        .sidebar
        .tree
        .iter()
        .filter_map(|n| match n {
            TreeNode::Catalog { name, expanded } => Some((name.clone(), *expanded)),
            _ => None,
        })
        .collect();
    assert_eq!(
        expanded,
        [
            ("catalog".to_string(), false),
            ("identity".to_string(), true),
            ("postgres".to_string(), false)
        ]
    );
}

#[test]
fn diagram_zoom_keys_win_over_the_global_ones() {
    let (mut app, tab_id) = app_with_diagram();
    let detail = |app: &App| {
        app.state
            .find_tab(tab_id)
            .and_then(|t| t.diagram.as_ref())
            .map(|d| d.detail)
    };
    assert_eq!(detail(&app), Some(crate::ui::diagram::Detail::Full));
    for _ in 0..3 {
        press(&mut app, 'z');
        press(&mut app, 'o');
    }
    assert_eq!(detail(&app), Some(crate::ui::diagram::Detail::Mini));
    let screen = screen_rows(&draw(&mut app, 120, 30)).join("\n");
    assert!(
        screen.contains("[users]") && screen.contains("[order_items]"),
        "{screen}"
    );
    assert!(screen.contains("zoom mini"), "{screen}");
    press(&mut app, 'z');
    press(&mut app, 'i');
    assert_eq!(detail(&app), Some(crate::ui::diagram::Detail::Names));

    // `-` is not a diagram key: it still opens the floating navigator.
    press(&mut app, '-');
    assert!(app.state.oil.is_some(), "`-` opens the navigator");
}
