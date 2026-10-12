//! Keys of a diagram tab: pan, move the selection between tables, focus,
//! depth, search, open the selected table, export.

use crossterm::event::{KeyCode, KeyEvent};

use crate::keybindings::Context;
use crate::ui::diagram::{PAN_STEP_X, PAN_STEP_Y};
use crate::ui::state::AppState;
use crate::ui::tabs::TabKind;

use super::Action;

pub(super) fn handle_diagram_keys(state: &mut AppState, key: KeyEvent) -> Action {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return Action::None;
    }
    let tab_id = state.tabs[tab_idx].id;
    let (conn_name, schema) = match &state.tabs[tab_idx].kind {
        TabKind::Diagram { conn_name, schema } => (conn_name.clone(), schema.clone()),
        _ => return Action::None,
    };

    // The search prompt takes every key until it is closed.
    if state.tabs[tab_idx]
        .diagram
        .as_ref()
        .is_some_and(|d| d.search.is_some())
    {
        return handle_search_key(state, tab_idx, tab_id, key);
    }

    let b = &state.bindings;
    let action: Option<&'static str> = [
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
        "focus_table",
        "toggle_focus",
        "depth_more",
        "depth_less",
        "toggle_columns",
        "open_table",
        "search",
        "next_match",
        "prev_match",
        "center",
        "export_diagram",
        "reload",
    ]
    .into_iter()
    .find(|name| b.matches(Context::Diagram, name, &key));
    let Some(action) = action else {
        return Action::None;
    };

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
        "focus_table" => visible_changed = diagram.focus_selected(),
        "toggle_focus" => visible_changed = diagram.toggle_focus(),
        "depth_more" => visible_changed = diagram.change_depth(1),
        "depth_less" => visible_changed = diagram.change_depth(-1),
        "toggle_columns" => visible_changed = diagram.toggle_columns(),
        "center" => diagram.scroll_request = Some(crate::ui::diagram::ScrollRequest::Center),
        "search" => diagram.search = Some(String::new()),
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
            diagram.search = None;
        }
        KeyCode::Enter => {
            if let Some(query) = diagram.search.take() {
                diagram.last_search = query;
                visible_changed = diagram.jump_to_match(true) || visible_changed;
            }
        }
        KeyCode::Backspace => {
            if let Some(query) = diagram.search.as_mut() {
                query.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Some(query) = diagram.search.as_mut() {
                query.push(c);
                let query = query.clone();
                // Live preview: select the first match while typing.
                if let Some(first) = diagram.matches(&query).into_iter().next() {
                    visible_changed = diagram.jump_to(&first);
                }
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
