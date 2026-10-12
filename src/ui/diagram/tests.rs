use super::*;

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

fn col(name: &str, pk: bool) -> Column {
    Column {
        name: name.into(),
        data_type: "integer".into(),
        nullable: !pk,
        is_primary_key: pk,
    }
}

fn shop(focus: Option<&str>) -> DiagramState {
    let mut d = DiagramState::new("app".into(), focus.map(str::to_string));
    let tables = ["users", "orders", "order_items", "products", "logs"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    d.set_data(
        tables,
        vec![
            fk("orders", "user_id", "users"),
            fk("order_items", "order_id", "orders"),
            fk("order_items", "product_id", "products"),
        ],
    );
    d.viewport = (80, 24);
    d
}

#[test]
fn small_schema_opens_whole_with_columns() {
    let mut d = shop(None);
    assert!(d.focus.is_none());
    assert_eq!(d.detail, Detail::Full);
    assert_eq!(d.layout().boxes.len(), 5);
    assert_eq!(d.layout().edges.len(), 3);
    let mut needed = d.tables_needing_columns();
    needed.sort();
    assert_eq!(
        needed,
        ["logs", "order_items", "orders", "products", "users"]
    );
}

#[test]
fn focus_limits_the_boxes_and_the_column_requests() {
    let mut d = shop(Some("orders"));
    let mut names: Vec<String> = d.layout().boxes.iter().map(|b| b.name.clone()).collect();
    names.sort();
    assert_eq!(names, ["order_items", "orders", "users"]);
    assert_eq!(d.layout().edges.len(), 2);
    let mut needed = d.tables_needing_columns();
    needed.sort();
    assert_eq!(needed, ["order_items", "orders", "users"]);

    d.pending_columns.insert("users".into());
    d.set_columns(
        "orders".into(),
        vec![col("id", true), col("user_id", false)],
    );
    assert_eq!(d.tables_needing_columns(), ["order_items"]);
}

#[test]
fn edges_attach_to_the_column_rows_once_loaded() {
    let mut d = shop(Some("orders"));
    d.set_columns(
        "orders".into(),
        vec![col("id", true), col("user_id", false)],
    );
    let layout = d.layout().clone();
    let orders = layout.box_index("orders").expect("orders box");
    let b = &layout.boxes[orders];
    assert_eq!(b.rows[0].key, KeyKind::Primary);
    assert_eq!(b.rows[1].key, KeyKind::Foreign);
    assert_eq!(b.height, 6);
    let to_users = layout
        .edges
        .iter()
        .find(|e| layout.boxes[e.to].name == "users")
        .expect("orders -> users");
    assert_eq!(to_users.from_y, b.row_y(1), "leaves from the user_id row");
    let from_items = layout
        .edges
        .iter()
        .find(|e| layout.boxes[e.from].name == "order_items")
        .expect("order_items -> orders");
    assert_eq!(from_items.to_y, b.row_y(0), "arrives at the id row");
}

#[test]
fn walking_columns_and_following_a_foreign_key() {
    let mut d = shop(None);
    d.set_columns(
        "orders".into(),
        vec![col("id", true), col("user_id", false)],
    );
    d.set_columns("users".into(), vec![col("id", true), col("name", false)]);
    d.select_direction(1, 0);
    assert_eq!(d.selected.as_deref(), Some("orders"));
    assert!(d.enter_columns());
    assert_eq!(d.column_cursor, Some(0));
    d.move_column(-1);
    assert_eq!(d.column_cursor, Some(1), "wraps to the last row");
    assert_eq!(d.current_column().map(|r| r.column), Some("user_id".into()));
    assert!(!d.follow_column(), "users is already drawn");
    assert_eq!(d.selected.as_deref(), Some("users"));
    assert_eq!(d.column_cursor, Some(0), "landed on users.id");
    d.select_direction(1, 0);
    assert_eq!(
        d.column_cursor, None,
        "moving between tables leaves the columns"
    );
    d.leave_columns();
    assert!(!d.enter_columns() || d.column_cursor.is_some());
}

#[test]
fn hjkl_select_the_nearest_table_in_that_direction() {
    let mut d = shop(None);
    // Columns: users and products (referenced), orders, order_items, logs.
    assert_eq!(d.selected.as_deref(), Some("users"));
    d.select_direction(1, 0);
    assert_eq!(d.selected.as_deref(), Some("orders"));
    d.select_direction(1, 0);
    assert_eq!(d.selected.as_deref(), Some("order_items"));
    d.select_direction(-1, 0);
    assert_eq!(d.selected.as_deref(), Some("orders"));
    d.select_direction(-1, 0);
    assert_eq!(d.selected.as_deref(), Some("users"));
    d.select_direction(0, 1);
    assert_eq!(
        d.selected.as_deref(),
        Some("products"),
        "below users in the same column"
    );
    d.select_direction(0, -1);
    assert_eq!(d.selected.as_deref(), Some("users"));
    d.select_direction(0, -1);
    assert_eq!(d.selected.as_deref(), Some("users"), "nothing above: stays");
}

#[test]
fn selection_wraps_and_jump_refocuses_hidden_tables() {
    let mut d = shop(Some("orders"));
    assert_eq!(d.selected.as_deref(), Some("orders"));
    for _ in 0..3 {
        d.select_offset(1);
    }
    assert_eq!(
        d.selected.as_deref(),
        Some("orders"),
        "three steps wrap around three boxes"
    );

    assert!(!d.jump_to("users"), "users is already drawn");
    assert_eq!(d.focus.as_deref(), Some("orders"));
    assert!(d.jump_to("logs"), "logs is outside the neighbourhood");
    assert_eq!(d.focus.as_deref(), Some("logs"));
    assert_eq!(d.selected_table(), Some(("app".into(), "logs".into())));
}

#[test]
fn cancelling_a_search_restores_focus_and_selection() {
    let mut d = shop(Some("orders"));
    d.start_search(SearchKind::Table);
    assert!(d.jump_to("logs"), "the preview refocused on a hidden table");
    assert_eq!(d.focus.as_deref(), Some("logs"));
    assert!(d.cancel_search(), "back to the orders neighbourhood");
    assert_eq!(d.focus.as_deref(), Some("orders"));
    assert_eq!(d.selected.as_deref(), Some("orders"));
    assert!(d.search.is_none() && d.search_origin.is_none());

    d.start_search(SearchKind::Table);
    d.search = Some("log".into());
    assert!(d.accept_search());
    assert_eq!(d.last_search, "log");
    assert_eq!(d.focus.as_deref(), Some("logs"), "Enter keeps the result");
}

#[test]
fn column_search_lands_on_the_table_and_the_column() {
    let mut d = shop(None);
    d.set_columns(
        "orders".into(),
        vec![col("id", true), col("user_id", false)],
    );
    d.set_columns("users".into(), vec![col("id", true), col("email", false)]);
    d.start_search(SearchKind::Column);
    assert!(d.columns_everywhere);
    let mut wanted = d.tables_needing_columns();
    wanted.sort();
    assert_eq!(
        wanted,
        ["logs", "order_items", "products"],
        "every other table is asked for"
    );

    d.search = Some("id".into());
    assert!(!d.preview_search("id"));
    let hit = d.last_hit.clone().expect("a hit");
    assert_eq!(
        (hit.table.as_str(), hit.column.as_deref()),
        ("users", Some("id"))
    );
    assert_eq!(d.column_cursor, Some(0));
    d.accept_search();
    assert!(!d.columns_everywhere);
    assert_eq!(
        d.selected.as_deref(),
        Some("users"),
        "Enter keeps the preview"
    );

    d.jump_to_match(true);
    assert_eq!(d.selected.as_deref(), Some("orders"));
    assert_eq!(d.current_column().map(|r| r.column), Some("id".into()));
    d.jump_to_match(true);
    assert_eq!(
        d.current_column().map(|r| r.column),
        Some("user_id".into()),
        "second column of orders"
    );
    d.jump_to_match(true);
    assert_eq!(d.selected.as_deref(), Some("users"), "wraps around");
}

#[test]
fn enter_keeps_the_match_the_preview_landed_on() {
    let mut d = shop(None);
    d.start_search(SearchKind::Table);
    d.search = Some("order".into());
    // What typing does: preview the first match.
    d.preview_search("order");
    assert_eq!(d.selected.as_deref(), Some("orders"));
    d.accept_search();
    assert_eq!(
        d.selected.as_deref(),
        Some("orders"),
        "Enter does not skip ahead"
    );
    assert!(d.jump_to_match(true) || d.selected.as_deref() == Some("order_items"));
    assert_eq!(d.selected.as_deref(), Some("order_items"), "n moves on");
}

#[test]
fn search_lists_visible_matches_first() {
    let mut d = shop(Some("orders"));
    let tables = |hits: Vec<SearchHit>| hits.into_iter().map(|h| h.table).collect::<Vec<_>>();
    assert_eq!(tables(d.matches("order")), ["orders", "order_items"]);
    assert_eq!(tables(d.matches("prod")), ["products"]);
    assert!(d.matches("").is_empty());
    d.last_search = "o".into();
    assert!(d.jump_to_match(true) || d.selected.is_some());
}

#[test]
fn depth_only_changes_in_focus_mode() {
    let mut d = shop(None);
    assert!(!d.change_depth(1));
    assert!(d.toggle_focus());
    assert!(d.change_depth(1));
    assert_eq!(d.depth, 2);
    assert!(d.change_depth(10));
    assert_eq!(d.depth, MAX_DEPTH, "clamped");
    assert!(!d.change_depth(1), "already at the maximum");
}

#[test]
fn scroll_stays_inside_the_canvas() {
    let mut d = shop(None);
    d.viewport = (40, 10);
    d.pan(-100, -100);
    assert_eq!((d.scroll_x, d.scroll_y), (0, 0));
    d.pan(10_000, 10_000);
    let c = d.canvas();
    let (w, h) = (c.width as i32, c.height as i32);
    assert_eq!((d.scroll_x, d.scroll_y), ((w - 40).max(0), (h - 10).max(0)));
    d.scroll_request = Some(ScrollRequest::Center);
    d.apply_scroll_request();
    assert!(d.scroll_request.is_none());
}

#[test]
fn zoom_levels_change_box_shapes_and_column_needs() {
    let mut d = shop(None);
    d.set_columns(
        "orders".into(),
        vec![col("id", true), col("user_id", false)],
    );
    d.set_columns("users".into(), vec![col("id", true), col("name", false)]);
    let height = |d: &mut DiagramState, name: &str| {
        let i = d.layout().box_index(name).expect(name);
        d.layout().boxes[i].height
    };
    assert_eq!(height(&mut d, "orders"), 6);
    assert!(!d.zoom(false), "keys still want columns");
    assert_eq!(d.detail, Detail::Keys);
    assert_eq!(height(&mut d, "orders"), 6, "both columns are keys");
    assert_eq!(height(&mut d, "users"), 5, "name is not a key");
    d.zoom(false);
    assert_eq!(d.detail, Detail::Names);
    assert_eq!(height(&mut d, "orders"), 3);
    assert!(d.tables_needing_columns().is_empty());
    d.zoom(false);
    assert_eq!(d.detail, Detail::Mini);
    assert_eq!(height(&mut d, "orders"), 1);
    assert!(!d.zoom(false), "already at the smallest");
    assert!(!d.zoom(true), "names need no columns");
    assert!(d.zoom(true), "keys need columns again");
    assert_eq!(d.detail, Detail::Keys);
    assert_eq!(Detail::for_size(100), Detail::Names);
}
