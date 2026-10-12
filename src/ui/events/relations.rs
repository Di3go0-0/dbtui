//! Keys for the Relations sub-view of a table tab: the data grid's
//! navigation plus opening the table at the other end of a foreign key
//! and exporting the schema diagram.

use crossterm::event::KeyEvent;

use crate::keybindings::Context;
use crate::ui::state::AppState;
use crate::ui::tabs::TabKind;

use super::Action;
use super::grid::handle_tab_data_grid;

pub(super) fn handle_tab_relations(state: &mut AppState, key: KeyEvent) -> Action {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return Action::None;
    }

    if state.bindings.matches(Context::Grid, "open_relation", &key) {
        let tab = &state.tabs[tab_idx];
        if tab.grid_on_header || tab.grid_visual_mode {
            return Action::None;
        }
        let Some((schema, table)) = tab.relation_target(tab.grid_selected_row) else {
            return Action::None;
        };
        let conn_name = tab.kind.conn_name().unwrap_or_default().to_string();
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

    if state
        .bindings
        .matches(Context::Grid, "export_diagram", &key)
    {
        return Action::ExportDiagram {
            tab_id: state.tabs[tab_idx].id,
        };
    }

    handle_tab_data_grid(state, key)
}
