//! Keys of a diagram tab: pan, move the selection between tables, focus,
//! depth, search, open the selected table, export.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keybindings::Context;
use crate::ui::diagram::{PAN_STEP_X, PAN_STEP_Y};
use crate::ui::state::AppState;
use crate::ui::tabs::TabKind;

use super::Action;

/// Every action of the diagram context, in dispatch order.
pub(super) const DIAGRAM_ACTIONS: &[&str] = &[
    "select_left",
    "select_right",
    "select_down",
    "select_up",
    "pan_left",
    "pan_right",
    "pan_down",
    "pan_up",
    "half_page_down",
    "half_page_up",
    "next_table",
    "prev_table",
    "first_table",
    "last_table",
    "enter_columns",
    "leave_columns",
    "toggle_focus",
    "depth_more",
    "depth_less",
    "zoom_prefix",
    "open_table",
    "search",
    "search_columns",
    "next_match",
    "prev_match",
    "export_diagram",
    "reload",
];

pub(super) fn handle_diagram_keys(state: &mut AppState, key: KeyEvent) -> Action {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return Action::None;
    }
    let tab_id = state.tabs[tab_idx].id;
    let Some((conn_name, schema)) = state.tabs[tab_idx]
        .diagram_context()
        .map(|(c, s)| (c.to_string(), s.to_string()))
    else {
        return Action::None;
    };

    // The search prompt takes every key until it is closed.
    if state.tabs[tab_idx]
        .diagram
        .as_ref()
        .is_some_and(|d| d.search.is_some())
    {
        return handle_search_key(state, tab_idx, tab_id, key);
    }

    // After `z`: zoom in, zoom out or centre, vim's fold keys reused.
    if state.tabs[tab_idx]
        .diagram
        .as_ref()
        .is_some_and(|d| d.z_pending)
    {
        let b = &state.bindings;
        let picked = if b.matches(Context::Diagram, "zoom_in", &key) {
            Some("zoom_in")
        } else if b.matches(Context::Diagram, "zoom_out", &key) {
            Some("zoom_out")
        } else if b.matches(Context::Diagram, "center", &key) {
            Some("center")
        } else {
            None
        };
        let Some(diagram) = state.tabs[tab_idx].diagram.as_mut() else {
            return Action::None;
        };
        diagram.z_pending = false;
        let visible_changed = match picked {
            Some("zoom_in") => diagram.zoom(true),
            Some("zoom_out") => diagram.zoom(false),
            Some("center") => {
                diagram.scroll_request = Some(crate::ui::diagram::ScrollRequest::Center);
                false
            }
            _ => false,
        };
        return if visible_changed {
            Action::DiagramViewChanged { tab_id }
        } else {
            Action::Render
        };
    }

    let b = &state.bindings;
    let action = DIAGRAM_ACTIONS
        .iter()
        .copied()
        .find(|name| b.matches(Context::Diagram, name, &key));
    let Some(action) = action else {
        return Action::None;
    };

    // Inside a table's columns, up/down walk the rows and Enter follows a
    // foreign key; any other move leaves the columns and acts on tables.
    if state.tabs[tab_idx]
        .diagram
        .as_ref()
        .is_some_and(|d| d.column_cursor.is_some())
    {
        let Some(diagram) = state.tabs[tab_idx].diagram.as_mut() else {
            return Action::None;
        };
        match action {
            "select_down" => {
                diagram.move_column(1);
                return Action::Render;
            }
            "select_up" => {
                diagram.move_column(-1);
                return Action::Render;
            }
            "enter_columns" => {
                return if diagram.follow_column() {
                    Action::DiagramViewChanged { tab_id }
                } else {
                    Action::Render
                };
            }
            "leave_columns" => {
                diagram.leave_columns();
                return Action::Render;
            }
            _ => {}
        }
    }

    if action == "reload" {
        return Action::LoadDiagram { tab_id, schema };
    }
    if action == "export_diagram" {
        return Action::ExportDiagram { tab_id };
    }

    let Some(diagram) = state.tabs[tab_idx].diagram.as_mut() else {
        return Action::None;
    };
    if !diagram.loaded {
        return Action::None;
    }
    let mut visible_changed = false;
    match action {
        "select_left" => diagram.select_direction(-1, 0),
        "select_right" => diagram.select_direction(1, 0),
        "select_down" => diagram.select_direction(0, 1),
        "select_up" => diagram.select_direction(0, -1),
        "pan_left" => diagram.pan(-PAN_STEP_X, 0),
        "pan_right" => diagram.pan(PAN_STEP_X, 0),
        "pan_down" => diagram.pan(0, PAN_STEP_Y),
        "pan_up" => diagram.pan(0, -PAN_STEP_Y),
        "half_page_down" => diagram.half_page(true),
        "half_page_up" => diagram.half_page(false),
        "next_table" => diagram.select_offset(1),
        "prev_table" => diagram.select_offset(-1),
        "first_table" => diagram.select_first(),
        "last_table" => diagram.select_last(),
        "enter_columns" => {
            diagram.enter_columns();
        }
        "leave_columns" => diagram.leave_columns(),
        "toggle_focus" => visible_changed = diagram.toggle_focus(),
        "depth_more" => visible_changed = diagram.change_depth(1),
        "depth_less" => visible_changed = diagram.change_depth(-1),
        "zoom_prefix" => diagram.z_pending = true,
        "search" => {
            diagram.start_search(crate::ui::diagram::SearchKind::Table);
        }
        "search_columns" => {
            diagram.start_search(crate::ui::diagram::SearchKind::Column);
            // Columns of every table are wanted now, zoom or no zoom.
            visible_changed = true;
        }
        "next_match" => visible_changed = diagram.jump_to_match(true),
        "prev_match" => visible_changed = diagram.jump_to_match(false),
        "open_table" => {
            let Some((schema, table)) = diagram.selected_table() else {
                return Action::None;
            };
            let tab_id = state.open_or_focus_tab(TabKind::Table {
                conn_name,
                schema: schema.clone(),
                table: table.clone(),
            });
            return Action::LoadTableData {
                tab_id,
                schema,
                table,
            };
        }
        _ => return Action::None,
    }
    if visible_changed {
        Action::DiagramViewChanged { tab_id }
    } else {
        Action::Render
    }
}

fn handle_search_key(
    state: &mut AppState,
    tab_idx: usize,
    tab_id: crate::ui::tabs::TabId,
    key: KeyEvent,
) -> Action {
    let Some(diagram) = state.tabs[tab_idx].diagram.as_mut() else {
        return Action::None;
    };
    let mut visible_changed = false;
    match key.code {
        KeyCode::Esc => {
            visible_changed = diagram.cancel_search();
        }
        KeyCode::Enter => {
            visible_changed = diagram.accept_search();
        }
        KeyCode::Backspace => {
            if let Some(query) = diagram.search.as_mut() {
                query.pop();
            }
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            if let Some(query) = diagram.search.as_mut() {
                query.push(c);
                let query = query.clone();
                // Live preview: land on the first match while typing.
                visible_changed = diagram.preview_search(&query);
            }
        }
        _ => return Action::None,
    }
    if visible_changed {
        Action::DiagramViewChanged { tab_id }
    } else {
        Action::Render
    }
}
