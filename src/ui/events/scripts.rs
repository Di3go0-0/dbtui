use crossterm::event::{KeyCode, KeyEvent};

use super::{Action, ScriptOperation};
use crate::ui::state::{AppState, ScriptNode, ScriptsMode};

pub(super) fn handle_scripts_panel(state: &mut AppState, key: KeyEvent) -> Action {
    match &state.scripts_mode {
        ScriptsMode::ConfirmDelete { .. } => return handle_scripts_confirm(state, key),
        ScriptsMode::Insert { .. } => return handle_scripts_insert(state, key),
        ScriptsMode::Rename { .. } => return handle_scripts_rename_mode(state, key),
        ScriptsMode::PendingD => return handle_scripts_pending_d(state, key),
        ScriptsMode::PendingY => return handle_scripts_pending_y(state, key),
        ScriptsMode::Normal => {}
    }

    let visible = state.visible_scripts();
    let count = visible.len();
    let selected: Option<(usize, ScriptNode)> = visible
        .get(state.scripts_cursor)
        .map(|(idx, node)| (*idx, (*node).clone()));
    drop(visible);

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if count > 0 && state.scripts_cursor + 1 < count {
                state.scripts_cursor += 1;
            }
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if state.scripts_cursor > 0 {
                state.scripts_cursor -= 1;
            }
            Action::Render
        }
        KeyCode::Char('g') => {
            state.scripts_cursor = 0;
            state.scripts_offset = 0;
            Action::Render
        }
        KeyCode::Char('G') => {
            if count > 0 {
                state.scripts_cursor = count - 1;
            }
            Action::Render
        }
        KeyCode::Enter | KeyCode::Char('l') => {
            if let Some((idx, node)) = selected {
                match node {
                    ScriptNode::Collection { .. } => {
                        if let Some(ScriptNode::Collection { expanded, .. }) =
                            state.scripts_tree.get_mut(idx)
                        {
                            *expanded = !*expanded;
                        }
                        Action::Render
                    }
                    ScriptNode::Script { file_path, .. } => {
                        // Strip .sql extension for open_script which adds it back
                        let name = file_path
                            .strip_suffix(".sql")
                            .unwrap_or(&file_path)
                            .to_string();
                        Action::OpenScript { name }
                    }
                }
            } else {
                Action::None
            }
        }
        KeyCode::Char('h') => {
            if let Some((idx, node)) = selected {
                match node {
                    ScriptNode::Collection { .. } => {
                        if let Some(ScriptNode::Collection { expanded, .. }) =
                            state.scripts_tree.get_mut(idx)
                        {
                            *expanded = false;
                        }
                    }
                    ScriptNode::Script {
                        collection: Some(coll_name),
                        ..
                    } => {
                        for tnode in state.scripts_tree.iter_mut() {
                            if let ScriptNode::Collection { name, expanded } = tnode
                                && *name == coll_name
                            {
                                *expanded = false;
                                break;
                            }
                        }
                        let vis = state.visible_scripts();
                        for (vi, (_, vnode)) in vis.iter().enumerate() {
                            if let ScriptNode::Collection { name, .. } = vnode
                                && *name == coll_name
                            {
                                state.scripts_cursor = vi;
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
            Action::Render
        }
        // dd — delete (first d enters PendingD)
        KeyCode::Char('d') => {
            state.scripts_mode = ScriptsMode::PendingD;
            Action::Render
        }
        // yy — yank (first y enters PendingY)
        KeyCode::Char('y') => {
            state.scripts_mode = ScriptsMode::PendingY;
            Action::Render
        }
        // p — paste (move yanked script to current location)
        KeyCode::Char('p') => {
            if let Some(from) = state.scripts_yank.clone() {
                let to_collection = state.current_collection();
                state.scripts_yank = None;
                return Action::ScriptOp {
                    op: ScriptOperation::Move {
                        from,
                        to_collection,
                    },
                };
            }
            Action::None
        }
        // i/o — insert new item
        KeyCode::Char('i') | KeyCode::Char('o') => {
            state.scripts_mode = ScriptsMode::Insert { buf: String::new() };
            Action::Render
        }
        // cw/cc — rename (c enters pending, then w/c confirms)
        KeyCode::Char('c') => {
            if let Some((_, node)) = selected {
                let (buf, path) = match node {
                    ScriptNode::Collection { name, .. } => (format!("{name}/"), name),
                    ScriptNode::Script {
                        name, file_path, ..
                    } => (name, file_path),
                };
                state.scripts_mode = ScriptsMode::Rename {
                    buf,
                    original_path: path,
                };
            }
            Action::Render
        }
        _ => Action::None,
    }
}

pub(super) fn handle_scripts_confirm(state: &mut AppState, key: KeyEvent) -> Action {
    let path = if let ScriptsMode::ConfirmDelete { path } = &state.scripts_mode {
        path.clone()
    } else {
        return Action::None;
    };
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            state.scripts_mode = ScriptsMode::Normal;
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
            state.scripts_mode = ScriptsMode::Normal;
            Action::Render
        }
    }
}

pub(super) fn handle_scripts_insert(state: &mut AppState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            state.scripts_mode = ScriptsMode::Normal;
            Action::Render
        }
        KeyCode::Enter => {
            let buf = if let ScriptsMode::Insert { buf } = &state.scripts_mode {
                buf.clone()
            } else {
                return Action::None;
            };
            state.scripts_mode = ScriptsMode::Normal;
            if buf.is_empty() {
                return Action::Render;
            }
            let in_collection = state.current_collection();
            Action::ScriptOp {
                op: ScriptOperation::Create {
                    name: buf,
                    in_collection,
                },
            }
        }
        KeyCode::Backspace => {
            if let ScriptsMode::Insert { buf } = &mut state.scripts_mode {
                buf.pop();
            }
            Action::Render
        }
        KeyCode::Char(c) => {
            if let ScriptsMode::Insert { buf } = &mut state.scripts_mode {
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
            state.scripts_mode = ScriptsMode::Normal;
            Action::Render
        }
        KeyCode::Enter => {
            let (buf, original_path) =
                if let ScriptsMode::Rename { buf, original_path } = &state.scripts_mode {
                    (buf.clone(), original_path.clone())
                } else {
                    return Action::None;
                };
            state.scripts_mode = ScriptsMode::Normal;
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
            if let ScriptsMode::Rename { buf, .. } = &mut state.scripts_mode {
                buf.pop();
            }
            Action::Render
        }
        KeyCode::Char(c) => {
            if let ScriptsMode::Rename { buf, .. } = &mut state.scripts_mode {
                buf.push(c);
            }
            Action::Render
        }
        _ => Action::None,
    }
}

pub(super) fn handle_scripts_pending_d(state: &mut AppState, key: KeyEvent) -> Action {
    state.scripts_mode = ScriptsMode::Normal;
    if key.code == KeyCode::Char('d') {
        let visible = state.visible_scripts();
        let selected = visible
            .get(state.scripts_cursor)
            .map(|(_, node)| (*node).clone());
        drop(visible);
        if let Some(node) = selected {
            let path = match node {
                ScriptNode::Collection { name, .. } => name,
                ScriptNode::Script { file_path, .. } => file_path,
            };
            state.scripts_mode = ScriptsMode::ConfirmDelete { path };
        }
    }
    Action::Render
}

pub(super) fn handle_scripts_pending_y(state: &mut AppState, key: KeyEvent) -> Action {
    state.scripts_mode = ScriptsMode::Normal;
    if key.code == KeyCode::Char('y') {
        let visible = state.visible_scripts();
        let selected = visible
            .get(state.scripts_cursor)
            .map(|(_, node)| (*node).clone());
        drop(visible);
        if let Some(ScriptNode::Script { file_path, .. }) = selected {
            state.scripts_yank = Some(file_path);
        }
    }
    Action::Render
}
