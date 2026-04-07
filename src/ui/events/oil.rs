use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keybindings::Context;
use crate::ui::state::{AppState, OilPane};

use super::scripts::handle_scripts_panel;
use super::sidebar::handle_sidebar;
use super::Action;

pub(super) fn handle_oil(state: &mut AppState, key: KeyEvent) -> Action {
    // Leader key pass-through
    if (state.leader.pending
        || state.leader.b_pending
        || state.leader.w_pending
        || state.leader.s_pending
        || state.leader.f_pending
        || state.leader.q_pending
        || state.leader.leader_pending)
        && let Some(action) = super::leader::handle_global_leader(state, key)
    {
        return action;
    }

    // Leader key trigger
    if state.mode == crate::ui::state::Mode::Normal
        && let KeyCode::Char(c) = key.code
        && c == vimltui::LEADER_KEY
        && !key.modifiers.contains(KeyModifiers::CONTROL)
    {
        state.leader.pending = true;
        state.leader.pressed_at = Some(std::time::Instant::now());
        return Action::Render;
    }

    // Esc / q close oil — but ONLY if we're at the top level. Esc must
    // unwind one layer at a time: a rename/create/search/sub-mode in progress
    // should be cancelled by Esc first, leaving oil itself open.
    let in_inner_mode = state.dialogs.group_renaming.is_some()
        || state.dialogs.group_creating
        || state.dialogs.conn_renaming.is_some()
        || state.sidebar.tree_state.search_active
        || !matches!(state.scripts.mode, crate::ui::state::ScriptsMode::Normal);

    // Close oil — but ONLY at the top level. While inner modes are active
    // the close key must be consumed by the underlying handler first.
    if !in_inner_mode && state.bindings.matches(Context::Oil, "close", &key) {
        close_oil(state);
        return Action::Render;
    }

    // Switch panes (configurable).
    if state
        .bindings
        .matches(Context::Oil, "switch_pane_left", &key)
    {
        if let Some(ref mut oil) = state.oil {
            oil.pane = OilPane::Explorer;
        }
        return Action::Render;
    }
    if state
        .bindings
        .matches(Context::Oil, "switch_pane_right", &key)
    {
        if let Some(ref mut oil) = state.oil {
            oil.pane = OilPane::Scripts;
        }
        return Action::Render;
    }

    let pane = state.oil.as_ref().map(|o| o.pane);

    match pane {
        Some(OilPane::Explorer) => handle_explorer_pane(state, key),
        Some(OilPane::Scripts) => handle_scripts_pane(state, key),
        None => Action::Render,
    }
}

fn handle_explorer_pane(state: &mut AppState, key: KeyEvent) -> Action {
    // 'F' — open object filter (normally handled by handle_global_normal_keys,
    // which doesn't run when oil owns the input).
    if key.code == KeyCode::Char('F') {
        return super::sidebar::handle_filter_key(state);
    }

    // Ctrl+S on a tree node — open in vertical split.
    // - No tabs: behaves like Enter (opens in single group)
    // - One group, has tabs: creates new group 2 and opens there
    // - Two groups already: behaves like Enter (opens in current focused group)
    if state.bindings.matches(Context::Oil, "open_in_split", &key) {
        if !state.tabs.is_empty() && state.groups.is_none() {
            state.create_empty_split();
        }
        let enter_event = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let action = handle_sidebar(state, enter_event);
        if matches!(
            action,
            Action::LoadTableData { .. }
                | Action::LoadPackageContent { .. }
                | Action::LoadSourceCode { .. }
        ) {
            close_oil(state);
        }
        return action;
    }

    // 'a' to add connection (same as sidebar global 'a')
    if key.code == KeyCode::Char('a') {
        let groups = state.available_groups();
        let current_group = state
            .selected_tree_index()
            .and_then(|idx| {
                let mut i = idx;
                loop {
                    if let crate::ui::state::TreeNode::Group { name, .. } =
                        &state.sidebar.tree[i]
                    {
                        return Some(name.clone());
                    }
                    if i == 0 {
                        break;
                    }
                    i -= 1;
                }
                None
            })
            .unwrap_or_else(|| "Default".to_string());
        state.dialogs.connection_form = crate::ui::state::ConnectionFormState::new();
        state.dialogs.connection_form.group = current_group;
        state.dialogs.connection_form.group_options = groups;
        state.overlay = Some(crate::ui::state::Overlay::ConnectionDialog);
        return Action::Render;
    }

    let action = handle_sidebar(state, key);

    // Auto-close oil when an action opens a tab
    if matches!(
        action,
        Action::LoadTableData { .. }
            | Action::LoadPackageContent { .. }
            | Action::LoadSourceCode { .. }
    ) {
        close_oil(state);
    }

    action
}

fn handle_scripts_pane(state: &mut AppState, key: KeyEvent) -> Action {
    // Ctrl+S — open script in vertical split (only when there's a single group with tabs)
    if state.bindings.matches(Context::Oil, "open_in_split", &key) {
        if !state.tabs.is_empty() && state.groups.is_none() {
            state.create_empty_split();
        }
        let enter_event = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let action = handle_scripts_panel(state, enter_event);
        if matches!(action, Action::OpenScript { .. }) {
            close_oil(state);
        }
        return action;
    }

    let action = handle_scripts_panel(state, key);

    // Auto-close oil when opening a script
    if matches!(action, Action::OpenScript { .. }) {
        close_oil(state);
    }

    action
}

fn close_oil(state: &mut AppState) {
    if let Some(oil) = state.oil.take() {
        state.focus = oil.previous_focus;
    }
}
