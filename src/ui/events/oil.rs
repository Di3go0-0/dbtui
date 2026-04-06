use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

    // Close oil
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            close_oil(state);
            return Action::Render;
        }
        _ => {}
    }

    // Switch panes: Ctrl+h/l or Left/Right arrows
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('h') => {
                if let Some(ref mut oil) = state.oil {
                    oil.pane = OilPane::Explorer;
                }
                return Action::Render;
            }
            KeyCode::Char('l') => {
                if let Some(ref mut oil) = state.oil {
                    oil.pane = OilPane::Scripts;
                }
                return Action::Render;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Left => {
            if let Some(ref mut oil) = state.oil {
                oil.pane = OilPane::Explorer;
            }
            return Action::Render;
        }
        KeyCode::Right => {
            if let Some(ref mut oil) = state.oil {
                oil.pane = OilPane::Scripts;
            }
            return Action::Render;
        }
        _ => {}
    }

    let pane = state.oil.as_ref().map(|o| o.pane);

    match pane {
        Some(OilPane::Explorer) => handle_explorer_pane(state, key),
        Some(OilPane::Scripts) => handle_scripts_pane(state, key),
        None => Action::Render,
    }
}

fn handle_explorer_pane(state: &mut AppState, key: KeyEvent) -> Action {
    // Ctrl+S on a tree node — open in vertical split.
    // - No tabs: behaves like Enter (opens in single group)
    // - One group, has tabs: creates new group 2 and opens there
    // - Two groups already: behaves like Enter (opens in current focused group)
    if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
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
    if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
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
