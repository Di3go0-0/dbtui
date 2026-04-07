//! Experimental oil-style inline connection editor (Proposal D).
//!
//! Renders a floating buffer-like panel where each connection field is
//! one line. Standard vim-ish navigation:
//! - Normal: `j`/`k` move between fields, `i` enters Insert on the
//!   current field, `Enter` saves & connects, `Esc` cancels the editor.
//! - Insert: typed chars append to the current field, `Backspace` pops,
//!   `Esc` returns to Normal.
//! - Type / Group rows cycle with `Tab` (or `l`/`h`) in Normal mode.
//! - Password visibility toggles with `Ctrl+P` in either mode.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::ui::state::{AppState, InlineConnField, InlineConnMode};

use super::Action;

pub(super) fn handle_inline_conn_editor(state: &mut AppState, key: KeyEvent) -> Action {
    // Clear any stale error on next keystroke so the feedback isn't sticky.
    if let Some(ed) = state.dialogs.inline_conn_editor.as_mut()
        && !ed.error_message.is_empty()
    {
        ed.error_message.clear();
    }

    // Ctrl+P toggles password visibility regardless of mode.
    if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
        if let Some(ed) = state.dialogs.inline_conn_editor.as_mut() {
            ed.password_visible = !ed.password_visible;
        }
        return Action::Render;
    }

    let mode = match state.dialogs.inline_conn_editor.as_ref() {
        Some(ed) => ed.mode,
        None => return Action::None,
    };

    match mode {
        InlineConnMode::Normal => handle_normal(state, key),
        InlineConnMode::Insert => handle_insert(state, key),
    }
}

fn handle_normal(state: &mut AppState, key: KeyEvent) -> Action {
    let ed = match state.dialogs.inline_conn_editor.as_mut() {
        Some(e) => e,
        None => return Action::None,
    };
    let field = ed.current_field();

    match key.code {
        KeyCode::Esc => {
            // Cancel editor entirely.
            state.dialogs.inline_conn_editor = None;
            Action::Render
        }
        KeyCode::Char('j') | KeyCode::Down => {
            ed.move_down();
            Action::Render
        }
        KeyCode::Char('k') | KeyCode::Up => {
            ed.move_up();
            Action::Render
        }
        KeyCode::Char('i') | KeyCode::Char('a') | KeyCode::Char('o') => {
            if field.is_text() {
                ed.mode = InlineConnMode::Insert;
            } else if matches!(field, InlineConnField::Type) {
                ed.cycle_db_type();
            } else if matches!(field, InlineConnField::Group) {
                ed.cycle_group();
            }
            Action::Render
        }
        // Left/right on Type or Group cycles; on text fields it's a no-op
        // (the experimental editor doesn't support in-value cursor moves).
        KeyCode::Char('l') | KeyCode::Char('h') | KeyCode::Tab | KeyCode::BackTab => {
            match field {
                InlineConnField::Type => ed.cycle_db_type(),
                InlineConnField::Group => ed.cycle_group(),
                _ => {}
            }
            Action::Render
        }
        KeyCode::Enter => {
            // Validate + emit action to spawn the connect.
            if let Err(msg) = ed.validate() {
                ed.error_message = msg.to_string();
                return Action::Render;
            }
            ed.connecting = true;
            ed.connecting_since = Some(std::time::Instant::now());
            Action::InlineConnSaveAndConnect
        }
        KeyCode::Char('x') => {
            // Quick clear current value — handy when fiddling.
            if let Some(val) = ed.field_value_mut(field) {
                val.clear();
            }
            Action::Render
        }
        _ => Action::None,
    }
}

fn handle_insert(state: &mut AppState, key: KeyEvent) -> Action {
    let ed = match state.dialogs.inline_conn_editor.as_mut() {
        Some(e) => e,
        None => return Action::None,
    };
    let field = ed.current_field();

    match key.code {
        KeyCode::Esc => {
            ed.mode = InlineConnMode::Normal;
            Action::Render
        }
        KeyCode::Enter => {
            // Enter in Insert mode leaves insert and moves to next row —
            // quick bulk-fill UX.
            ed.mode = InlineConnMode::Normal;
            ed.move_down();
            Action::Render
        }
        KeyCode::Backspace => {
            if let Some(val) = ed.field_value_mut(field) {
                val.pop();
            }
            Action::Render
        }
        KeyCode::Char(c) => {
            if let Some(val) = ed.field_value_mut(field) {
                val.push(c);
            }
            Action::Render
        }
        KeyCode::Tab | KeyCode::Down => {
            ed.mode = InlineConnMode::Normal;
            ed.move_down();
            Action::Render
        }
        KeyCode::BackTab | KeyCode::Up => {
            ed.mode = InlineConnMode::Normal;
            ed.move_up();
            Action::Render
        }
        _ => Action::None,
    }
}
