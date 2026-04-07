//! Default keybindings for every context. This is the source of truth that
//! `--dump-keybindings` writes out and that loads when no user config exists.

use super::parser::{KeyBinding, parse_key};
use std::collections::BTreeMap;

/// Build the full default keybinding set.
///
/// Returned as nested maps so we can serialize them to TOML directly:
///   { context_name -> { action_name -> [key_string, ...] } }
///
/// Context names match the [section] headers in keybindings.toml.
pub fn defaults() -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
    let mut out = BTreeMap::new();

    // -------------------------------------------------------------------
    // [global] — always active. Quit, help, leader, panel switching,
    // sub-view brackets, oil/sidebar toggles, splits, file ops.
    // -------------------------------------------------------------------
    let mut global = BTreeMap::new();
    add(&mut global, "leader", &["Space"]);
    add(&mut global, "help", &["?"]);
    add(&mut global, "next_tab", &["Tab"]);
    add(&mut global, "prev_tab", &["BackTab"]);
    add(&mut global, "next_sub_view", &["]"]);
    add(&mut global, "prev_sub_view", &["["]);
    add(&mut global, "next_diagnostic", &["Ctrl+]"]);
    add(&mut global, "prev_diagnostic", &["Ctrl+["]);
    add(&mut global, "navigate_left", &["Ctrl+h", "Left"]);
    add(&mut global, "navigate_right", &["Ctrl+l", "Right"]);
    add(&mut global, "navigate_down", &["Ctrl+j", "Down"]);
    add(&mut global, "navigate_up", &["Ctrl+k", "Up"]);
    add(&mut global, "filter_objects", &["F"]);
    add(&mut global, "add_connection", &["a"]);
    // `-` toggles the floating oil navigator (matches oil.nvim muscle memory).
    add(&mut global, "toggle_oil_navigator", &["-"]);

    out.insert("global".to_string(), global);

    // -------------------------------------------------------------------
    // [leader] — bindings that fire after the leader key was pressed.
    // -------------------------------------------------------------------
    let mut leader = BTreeMap::new();
    add(&mut leader, "toggle_sidebar", &["e"]);
    add(&mut leader, "vertical_split", &["|"]);
    add(&mut leader, "move_tab_to_other_group", &["m"]);
    add(&mut leader, "open_theme_picker", &["t"]);
    add(&mut leader, "open_script_connection_picker", &["c"]);
    add(&mut leader, "toggle_diagnostic_list", &["x"]);
    add(&mut leader, "execute_query", &["Enter"]);
    add(&mut leader, "execute_query_new_tab", &["/"]);
    add(&mut leader, "open_buffer_submenu", &["b"]);
    add(&mut leader, "open_window_submenu", &["w"]);
    add(&mut leader, "open_snippet_submenu", &["s"]);
    add(&mut leader, "open_file_submenu", &["f"]);
    add(&mut leader, "open_quit_submenu", &["q"]);
    out.insert("leader".to_string(), leader);

    // -------------------------------------------------------------------
    // [leader_buffer] — second key after <leader>b
    // -------------------------------------------------------------------
    let mut leader_buffer = BTreeMap::new();
    add(&mut leader_buffer, "close_tab", &["d"]);
    out.insert("leader_buffer".to_string(), leader_buffer);

    // -------------------------------------------------------------------
    // [leader_window] — second key after <leader>w
    // -------------------------------------------------------------------
    let mut leader_window = BTreeMap::new();
    add(&mut leader_window, "close_group", &["d"]);
    out.insert("leader_window".to_string(), leader_window);

    // -------------------------------------------------------------------
    // [leader_file] — second key after <leader>f
    // -------------------------------------------------------------------
    let mut leader_file = BTreeMap::new();
    add(&mut leader_file, "export_connections", &["e"]);
    add(&mut leader_file, "import_connections", &["i"]);
    out.insert("leader_file".to_string(), leader_file);

    // -------------------------------------------------------------------
    // [leader_quit] — second key after <leader>q
    // -------------------------------------------------------------------
    let mut leader_quit = BTreeMap::new();
    add(&mut leader_quit, "quit_app", &["q"]);
    out.insert("leader_quit".to_string(), leader_quit);

    // -------------------------------------------------------------------
    // [leader_snippet] — second key after <leader>s
    // -------------------------------------------------------------------
    let mut leader_snippet = BTreeMap::new();
    add(&mut leader_snippet, "snippet_select", &["s"]);
    add(&mut leader_snippet, "snippet_update", &["u"]);
    add(&mut leader_snippet, "snippet_delete", &["d"]);
    add(&mut leader_snippet, "snippet_call_proc", &["p"]);
    add(&mut leader_snippet, "snippet_select_func", &["f"]);
    add(&mut leader_snippet, "snippet_create_table", &["t"]);
    out.insert("leader_snippet".to_string(), leader_snippet);

    // -------------------------------------------------------------------
    // [sidebar] — explorer (the sidebar tree, also reused by oil)
    // -------------------------------------------------------------------
    let mut sidebar = BTreeMap::new();
    add(&mut sidebar, "scroll_down", &["j", "Down"]);
    add(&mut sidebar, "scroll_up", &["k", "Up"]);
    add(&mut sidebar, "half_page_down", &["Ctrl+d"]);
    add(&mut sidebar, "half_page_up", &["Ctrl+u"]);
    add(&mut sidebar, "scroll_top", &["g"]);
    add(&mut sidebar, "scroll_bottom", &["G"]);
    add(&mut sidebar, "expand_or_open", &["l", "Enter"]);
    add(&mut sidebar, "collapse_or_parent", &["h", "Left"]);
    add(&mut sidebar, "create_new", &["i", "o"]);
    add(&mut sidebar, "group_menu", &["m"]);
    add(&mut sidebar, "rename_or_refresh", &["r"]);
    add(&mut sidebar, "yank_pending", &["y"]);
    add(&mut sidebar, "paste", &["p"]);
    add(&mut sidebar, "delete_pending", &["d"]);
    add(&mut sidebar, "start_search", &["/"]);
    add(&mut sidebar, "next_match", &["n"]);
    add(&mut sidebar, "prev_match", &["N"]);
    out.insert("sidebar".to_string(), sidebar);

    // -------------------------------------------------------------------
    // [scripts] — scripts panel (and the right pane of oil)
    // -------------------------------------------------------------------
    let mut scripts = BTreeMap::new();
    add(&mut scripts, "scroll_down", &["j", "Down"]);
    add(&mut scripts, "scroll_up", &["k", "Up"]);
    add(&mut scripts, "scroll_top", &["g"]);
    add(&mut scripts, "scroll_bottom", &["G"]);
    add(&mut scripts, "expand_or_open", &["l", "Enter"]);
    add(&mut scripts, "create_new", &["i", "o"]);
    add(&mut scripts, "rename", &["r"]);
    add(&mut scripts, "delete_pending", &["d"]);
    add(&mut scripts, "yank_pending", &["y"]);
    add(&mut scripts, "paste", &["p"]);
    out.insert("scripts".to_string(), scripts);

    // -------------------------------------------------------------------
    // [grid] — data grid (table data, query results, properties)
    // -------------------------------------------------------------------
    let mut grid = BTreeMap::new();
    add(&mut grid, "scroll_down", &["j", "Down"]);
    add(&mut grid, "scroll_up", &["k", "Up"]);
    add(&mut grid, "scroll_left", &["h", "Left"]);
    add(&mut grid, "scroll_right", &["l", "Right"]);
    add(&mut grid, "next_cell", &["e"]);
    add(&mut grid, "prev_cell", &["b"]);
    add(&mut grid, "scroll_top", &["g"]);
    add(&mut grid, "scroll_bottom", &["G"]);
    add(&mut grid, "half_page_down", &["Ctrl+d"]);
    add(&mut grid, "half_page_up", &["Ctrl+u"]);
    add(&mut grid, "toggle_visual", &["v"]);
    add(&mut grid, "yank", &["y"]);
    add(&mut grid, "refresh_data", &["r"]);
    add(&mut grid, "edit_cell", &["i"]);
    add(&mut grid, "new_row", &["o"]);
    add(&mut grid, "delete_pending", &["d"]);
    add(&mut grid, "undo_changes", &["u"]);
    add(&mut grid, "save_changes", &["Ctrl+s"]);
    add(&mut grid, "exit_grid", &["Esc"]);
    out.insert("grid".to_string(), grid);

    // -------------------------------------------------------------------
    // [oil] — keys specific to the floating navigator chrome (Esc, pane
    // switching, the open-in-split shortcut). Inside each pane the
    // sidebar/scripts contexts apply.
    // -------------------------------------------------------------------
    let mut oil = BTreeMap::new();
    add(&mut oil, "close", &["Esc", "q"]);
    add(&mut oil, "switch_pane_left", &["Ctrl+h", "Left"]);
    add(&mut oil, "switch_pane_right", &["Ctrl+l", "Right"]);
    add(&mut oil, "open_in_split", &["Ctrl+s"]);
    out.insert("oil".to_string(), oil);

    // -------------------------------------------------------------------
    // [overlay] — modals (help, theme picker, conn menu, etc.)
    // -------------------------------------------------------------------
    let mut overlay = BTreeMap::new();
    add(&mut overlay, "close", &["Esc", "q"]);
    add(&mut overlay, "confirm", &["Enter"]);
    add(&mut overlay, "nav_down", &["j", "Down"]);
    add(&mut overlay, "nav_up", &["k", "Up"]);
    out.insert("overlay".to_string(), overlay);

    out
}

/// Helper to insert one (action, [keys]) pair, validating each key string
/// at startup. We unwrap because these are hardcoded — a panic here is a
/// programmer bug, not a user error.
fn add(map: &mut BTreeMap<String, Vec<String>>, action: &str, keys: &[&str]) {
    for k in keys {
        let _: KeyBinding = parse_key(k).unwrap_or_else(|e| {
            panic!("default keybinding for '{action}' has invalid key '{k}': {e}")
        });
    }
    map.insert(
        action.to_string(),
        keys.iter().map(|s| s.to_string()).collect(),
    );
}
