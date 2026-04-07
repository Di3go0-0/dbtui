use crossterm::event::{KeyCode, KeyEvent};

use super::{Action, ScriptOperation};
use crate::keybindings::Context;
use crate::ui::state::{AppState, ScriptNode, ScriptsMode};

pub(super) fn handle_scripts_panel(state: &mut AppState, key: KeyEvent) -> Action {
    match &state.scripts.mode {
        ScriptsMode::ConfirmDelete { .. } => return handle_scripts_confirm(state, key),
        ScriptsMode::Insert { .. } => return handle_scripts_insert(state, key),
        ScriptsMode::Rename { .. } => return handle_scripts_rename_mode(state, key),
        ScriptsMode::PendingD => return handle_scripts_pending_d(state, key),
        ScriptsMode::PendingY => return handle_scripts_pending_y(state, key),
        ScriptsMode::Normal => {}
    }

    let visible = state.scripts.visible_scripts();
    let count = visible.len();
    let selected: Option<(usize, ScriptNode)> = visible
        .get(state.scripts.cursor)
        .map(|(idx, node)| (*idx, (*node).clone()));
    drop(visible);

    let b = &state.bindings;
    if b.matches(Context::Scripts, "scroll_down", &key) {
        if count > 0 && state.scripts.cursor + 1 < count {
            state.scripts.cursor += 1;
        }
        return Action::Render;
    }
    if b.matches(Context::Scripts, "scroll_up", &key) {
        if state.scripts.cursor > 0 {
            state.scripts.cursor -= 1;
        }
        return Action::Render;
    }
    if b.matches(Context::Scripts, "scroll_top", &key) {
        state.scripts.cursor = 0;
        state.scripts.offset = 0;
        return Action::Render;
    }
    if b.matches(Context::Scripts, "scroll_bottom", &key) {
        if count > 0 {
            state.scripts.cursor = count - 1;
        }
        return Action::Render;
    }
    if b.matches(Context::Scripts, "expand_or_open", &key) {
        return if let Some((idx, node)) = selected {
            match node {
                ScriptNode::Collection { .. } => {
                    if let Some(ScriptNode::Collection { expanded, .. }) =
                        state.scripts.tree.get_mut(idx)
                    {
                        *expanded = !*expanded;
                    }
                    Action::Render
                }
                ScriptNode::Script { file_path, .. } => {
                    let name = file_path
                        .strip_suffix(".sql")
                        .unwrap_or(&file_path)
                        .to_string();
                    Action::OpenScript { name }
                }
            }
        } else {
            Action::None
        };
    }
    if b.matches(Context::Scripts, "create_new", &key) {
        state.scripts.mode = ScriptsMode::Insert { buf: String::new() };
        return Action::Render;
    }
    if b.matches(Context::Scripts, "rename", &key) {
        if let Some((_, node)) = selected {
            let (buf, path) = match node {
                ScriptNode::Collection { name, .. } => (format!("{name}/"), name),
                ScriptNode::Script {
                    name, file_path, ..
                } => (name, file_path),
            };
            state.scripts.mode = ScriptsMode::Rename {
                buf,
                original_path: path,
            };
        }
        return Action::Render;
    }
    if b.matches(Context::Scripts, "delete_pending", &key) {
        state.scripts.mode = ScriptsMode::PendingD;
        return Action::Render;
    }
    if b.matches(Context::Scripts, "yank_pending", &key) {
        state.scripts.mode = ScriptsMode::PendingY;
        return Action::Render;
    }
    if b.matches(Context::Scripts, "paste", &key) {
        if let Some(from) = state.scripts.yank.clone() {
            let to_collection = state.scripts.current_collection();
            state.scripts.yank = None;
            return Action::ScriptOp {
                op: ScriptOperation::Move {
                    from,
                    to_collection,
                },
            };
        }
        return Action::None;
    }

    // h — collapse, kept as a non-configurable chrome key (mirrors the
    // sidebar collapse behavior and isn't listed in the scripts context).
    if key.code == KeyCode::Char('h') {
        if let Some((idx, node)) = selected {
            match node {
                ScriptNode::Collection { .. } => {
                    if let Some(ScriptNode::Collection { expanded, .. }) =
                        state.scripts.tree.get_mut(idx)
                    {
                        *expanded = false;
                    }
                }
                ScriptNode::Script {
                    collection: Some(coll_name),
                    ..
                } => {
                    for tnode in state.scripts.tree.iter_mut() {
                        if let ScriptNode::Collection { name, expanded } = tnode
                            && *name == coll_name
                        {
                            *expanded = false;
                            break;
                        }
                    }
                    let vis = state.scripts.visible_scripts();
                    for (vi, (_, vnode)) in vis.iter().enumerate() {
                        if let ScriptNode::Collection { name, .. } = vnode
                            && *name == coll_name
                        {
                            state.scripts.cursor = vi;
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        return Action::Render;
    }

    Action::None
}

pub(super) fn handle_scripts_confirm(state: &mut AppState, key: KeyEvent) -> Action {
    let path = if let ScriptsMode::ConfirmDelete { path } = &state.scripts.mode {
        path.clone()
    } else {
        return Action::None;
    };
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            state.scripts.mode = ScriptsMode::Normal;
            if !path.contains('.') {
                // Collection (directory name, no extension)
                Action::ScriptOp {
                    op: ScriptOperation::DeleteCollection { name: path },
                }
            } else {
                Action::ScriptOp {
                    op: ScriptOperation::Delete { path },
                }
            }
        }
        _ => {
            state.scripts.mode = ScriptsMode::Normal;
            Action::Render
        }
    }
}

pub(super) fn handle_scripts_insert(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.scripts.mode = ScriptsMode::Normal;
            Action::Render
        }
        KeyCode::Enter => {
            let buf = if let ScriptsMode::Insert { buf } = &state.scripts.mode {
                buf.clone()
            } else {
                return Action::None;
            };
            state.scripts.mode = ScriptsMode::Normal;
            if buf.is_empty() {
                return Action::Render;
            }
            let in_collection = state.scripts.current_collection();
            Action::ScriptOp {
                op: ScriptOperation::Create {
                    name: buf,
                    in_collection,
                },
            }
        }
        KeyCode::Backspace => {
            if let ScriptsMode::Insert { buf } = &mut state.scripts.mode {
                buf.pop();
            }
            Action::Render
        }
        KeyCode::Char(c) => {
            if let ScriptsMode::Insert { buf } = &mut state.scripts.mode {
                buf.push(c);
            }
            Action::Render
        }
        _ => Action::None,
    }
}

pub(super) fn handle_scripts_rename_mode(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.scripts.mode = ScriptsMode::Normal;
            Action::Render
        }
        KeyCode::Enter => {
            let (buf, original_path) =
                if let ScriptsMode::Rename { buf, original_path } = &state.scripts.mode {
                    (buf.clone(), original_path.clone())
                } else {
                    return Action::None;
                };
            state.scripts.mode = ScriptsMode::Normal;
            if buf.is_empty() {
                return Action::Render;
            }
            if buf.ends_with('/') {
                let new_name = buf.trim_end_matches('/').to_string();
                Action::ScriptOp {
                    op: ScriptOperation::RenameCollection {
                        old_name: original_path,
                        new_name,
                    },
                }
            } else {
                Action::ScriptOp {
                    op: ScriptOperation::Rename {
                        old_path: original_path,
                        new_name: buf,
                    },
                }
            }
        }
        KeyCode::Backspace => {
            if let ScriptsMode::Rename { buf, .. } = &mut state.scripts.mode {
                buf.pop();
            }
            Action::Render
        }
        KeyCode::Char(c) => {
            if let ScriptsMode::Rename { buf, .. } = &mut state.scripts.mode {
                buf.push(c);
            }
            Action::Render
        }
        _ => Action::None,
    }
}

pub(super) fn handle_scripts_pending_d(state: &mut AppState, key: KeyEvent) -> Action {
    state.scripts.mode = ScriptsMode::Normal;
    if key.code == KeyCode::Char('d') {
        let visible = state.scripts.visible_scripts();
        let selected = visible
            .get(state.scripts.cursor)
            .map(|(_, node)| (*node).clone());
        drop(visible);
        if let Some(node) = selected {
            let path = match node {
                ScriptNode::Collection { name, .. } => name,
                ScriptNode::Script { file_path, .. } => file_path,
            };
            state.scripts.mode = ScriptsMode::ConfirmDelete { path };
        }
    }
    Action::Render
}

pub(super) fn handle_scripts_pending_y(state: &mut AppState, key: KeyEvent) -> Action {
    state.scripts.mode = ScriptsMode::Normal;
    if key.code == KeyCode::Char('y') {
        let visible = state.scripts.visible_scripts();
        let selected = visible
            .get(state.scripts.cursor)
            .map(|(_, node)| (*node).clone());
        drop(visible);
        if let Some(ScriptNode::Script { file_path, .. }) = selected {
            state.scripts.yank = Some(file_path);
        }
    }
    Action::Render
}
