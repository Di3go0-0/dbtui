//! Keeps a `VimEditor` cursor on character boundaries.
//!
//! vimltui stores the cursor column as a byte offset and moves it one byte at
//! a time. That is only right for ASCII: after typing `ñ` the cursor sits in
//! the middle of the character, and the next insert or backspace asks `String`
//! to split it — a panic. Until the editor crate steps by characters itself,
//! every key goes through here: the cursor is snapped to a boundary on the way
//! in and on the way out, and the one edit that cannot be repaired afterwards
//! (backspacing over a multi-byte character) is done here instead.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use vimltui::{EditorAction, VimEditor, VimMode};

/// `VimEditor::handle_key`, safe on non-ASCII text.
pub fn handle_key(editor: &mut VimEditor, key: KeyEvent) -> EditorAction {
    snap_cursor(editor, Snap::Forward);
    let col_before = editor.cursor_col;
    let row_before = editor.cursor_row;

    if is_text_backspace(editor, key) && backspace_wide_char(editor) {
        return EditorAction::Handled;
    }

    let action = editor.handle_key(key);

    // A step left that landed inside a character belongs at its start; any
    // other landing (a step right, an insert) belongs just past it.
    let moved_left = editor.cursor_row == row_before && editor.cursor_col < col_before;
    snap_cursor(
        editor,
        if moved_left {
            Snap::Back
        } else {
            Snap::Forward
        },
    );
    action
}

/// `VimEditor::insert_char`, leaving the cursor after the whole character.
pub fn insert_char(editor: &mut VimEditor, ch: char) {
    snap_cursor(editor, Snap::Forward);
    editor.insert_char(ch);
    snap_cursor(editor, Snap::Forward);
}

#[derive(Clone, Copy)]
enum Snap {
    Back,
    Forward,
}

fn snap_cursor(editor: &mut VimEditor, snap: Snap) {
    let Some(line) = editor.lines.get(editor.cursor_row) else {
        return;
    };
    let mut col = editor.cursor_col.min(line.len());
    while !line.is_char_boundary(col) {
        match snap {
            Snap::Back => col -= 1,
            Snap::Forward => col += 1,
        }
    }
    // Only touch the column when it was actually off a boundary — vimltui
    // deliberately parks it past the end in some states.
    if !line.is_char_boundary(editor.cursor_col.min(line.len())) {
        editor.cursor_col = col;
    }
}

fn is_text_backspace(editor: &VimEditor, key: KeyEvent) -> bool {
    key.code == KeyCode::Backspace
        && key.modifiers == KeyModifiers::NONE
        && matches!(editor.mode, VimMode::Insert | VimMode::Replace)
        && !editor.search.active
        && !editor.command_active
}

/// Backspace over the character before the cursor when it is wider than one
/// byte. Returns false — leaving the key to vimltui — for ASCII, where its
/// own handling is correct.
fn backspace_wide_char(editor: &mut VimEditor) -> bool {
    let col = editor.cursor_col;
    let Some(line) = editor.lines.get_mut(editor.cursor_row) else {
        return false;
    };
    let Some((start, ch)) = line
        .get(..col)
        .and_then(|head| head.char_indices().next_back())
    else {
        return false;
    };
    if ch.len_utf8() == 1 {
        return false;
    }
    line.remove(start);
    editor.cursor_col = start;
    editor.modified = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_mode_editor(text: &str) -> VimEditor {
        let mut editor = VimEditor::new(text, vimltui::VimModeConfig::default());
        editor.mode = VimMode::Insert;
        editor.cursor_row = 0;
        editor.cursor_col = editor.lines[0].len();
        editor
    }

    fn press(editor: &mut VimEditor, code: KeyCode) {
        handle_key(editor, KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn typing_after_a_non_ascii_character_does_not_panic() {
        let mut editor = insert_mode_editor("");
        for ch in "'año'".chars() {
            press(&mut editor, KeyCode::Char(ch));
        }
        assert_eq!(editor.lines[0], "'año'");
        assert_eq!(editor.cursor_col, editor.lines[0].len());
    }

    #[test]
    fn backspace_removes_a_whole_non_ascii_character() {
        let mut editor = insert_mode_editor("año");
        press(&mut editor, KeyCode::Backspace); // o
        press(&mut editor, KeyCode::Backspace); // ñ
        assert_eq!(editor.lines[0], "a");
        assert_eq!(editor.cursor_col, 1);
    }

    #[test]
    fn pasting_non_ascii_text_keeps_the_cursor_on_a_boundary() {
        let mut editor = insert_mode_editor("");
        for ch in "José".chars() {
            insert_char(&mut editor, ch);
        }
        assert_eq!(editor.lines[0], "José");
        assert!(editor.lines[0].is_char_boundary(editor.cursor_col));
    }

    #[test]
    fn horizontal_motion_steps_over_whole_characters() {
        let mut editor = VimEditor::new("añb", vimltui::VimModeConfig::default());
        editor.mode = VimMode::Normal;
        editor.cursor_row = 0;
        editor.cursor_col = 0;
        press(&mut editor, KeyCode::Char('l'));
        assert_eq!(editor.cursor_col, 1); // on ñ
        press(&mut editor, KeyCode::Char('l'));
        assert_eq!(editor.cursor_col, 3); // on b, past both bytes of ñ
        press(&mut editor, KeyCode::Char('h'));
        assert_eq!(editor.cursor_col, 1); // back on ñ
    }
}
