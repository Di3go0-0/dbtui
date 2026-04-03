use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::buffer::VimEditor;
use super::motions::Motion;
use super::{EditorAction, FindDirection, Operator, VimMode, VisualKind};

impl VimEditor {
    /// Main input handler. Returns EditorAction to inform the parent.
    pub fn handle_key(&mut self, key: KeyEvent) -> EditorAction {
        // Command mode (:) takes highest priority
        if self.command_active {
            let action = self.handle_command_input(key);
            self.update_command_line();
            return action;
        }

        // Leader keys are handled globally in events.rs handle_global_leader()
        // before reaching the VimEditor. No leader handling here.

        // Search input mode takes priority
        let action = if self.search.active {
            self.handle_search_input(key)
        } else {
            match &self.mode {
                VimMode::Normal => self.handle_normal(key),
                VimMode::Insert => self.handle_insert(key),
                VimMode::Visual(_) => self.handle_visual(key),
            }
        };

        self.update_command_line();
        self.ensure_cursor_visible();
        action
    }

    // --- Search Input ---

    fn handle_search_input(&mut self, key: KeyEvent) -> EditorAction {
        match key.code {
            KeyCode::Esc => {
                self.cancel_search();
                EditorAction::Handled
            }
            KeyCode::Enter => {
                self.commit_search();
                EditorAction::Handled
            }
            KeyCode::Backspace => {
                self.search.input_buffer.pop();
                EditorAction::Handled
            }
            KeyCode::Char(c) => {
                self.search.input_buffer.push(c);
                EditorAction::Handled
            }
            _ => EditorAction::Handled,
        }
    }

    // --- Command Input (:) ---

    fn handle_command_input(&mut self, key: KeyEvent) -> EditorAction {
        match key.code {
            KeyCode::Esc => {
                self.command_active = false;
                self.command_buffer.clear();
                EditorAction::Handled
            }
            KeyCode::Enter => {
                let cmd = self.command_buffer.clone();
                self.command_active = false;
                self.command_buffer.clear();
                let action = self.execute_command(&cmd);
                self.ensure_cursor_visible();
                action
            }
            KeyCode::Backspace => {
                if self.command_buffer.is_empty() {
                    self.command_active = false;
                } else {
                    self.command_buffer.pop();
                }
                EditorAction::Handled
            }
            KeyCode::Char(c) => {
                self.command_buffer.push(c);
                EditorAction::Handled
            }
            _ => EditorAction::Handled,
        }
    }

    fn execute_command(&mut self, cmd: &str) -> EditorAction {
        let trimmed = cmd.trim();
        // :number -> go to line
        if let Ok(line_num) = trimmed.parse::<usize>() {
            self.move_to_line(line_num);
            return EditorAction::Handled;
        }
        match trimmed {
            "w" => return EditorAction::Save,
            "q" => return EditorAction::Close,
            "q!" => return EditorAction::ForceClose,
            "wq" | "x" => return EditorAction::SaveAndClose,
            _ => {}
        }
        EditorAction::Handled
    }

    // --- Normal Mode ---

    fn handle_normal(&mut self, key: KeyEvent) -> EditorAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Handle register prefix ("x)
        if self.pending_register {
            self.pending_register = false;
            if let KeyCode::Char('+') = key.code {
                self.use_system_clipboard = true;
            }
            return EditorAction::Handled;
        }

        // Handle pending find char (f/F/t/T)
        if let Some((direction, before)) = self.pending_find.take() {
            if let KeyCode::Char(c) = key.code {
                if let Some(op) = self.pending_operator.take() {
                    // Operator + find: compute range from cursor to found char, apply operator
                    let motion = match (direction, before) {
                        (FindDirection::Forward, false) => Motion::FindCharForward(c),
                        (FindDirection::Forward, true) => Motion::FindCharBefore(c),
                        (FindDirection::Backward, false) => Motion::FindCharBackward(c),
                        (FindDirection::Backward, true) => Motion::FindCharAfter(c),
                    };
                    let count = self.take_count();
                    self.execute_operator(&op, &motion, count);
                } else {
                    match direction {
                        FindDirection::Forward => self.find_char_forward(c, before),
                        FindDirection::Backward => self.find_char_backward(c, before),
                    }
                }
            }
            return EditorAction::Handled;
        }

        // Handle pending replace (r)
        if self.pending_replace {
            self.pending_replace = false;
            if let KeyCode::Char(c) = key.code {
                self.save_undo();
                self.replace_char(c);
            }
            return EditorAction::Handled;
        }

        // Handle 'g' prefix
        if self.pending_g {
            self.pending_g = false;
            return self.handle_g_prefix(key);
        }

        // Count prefix (digits)
        if let KeyCode::Char(c) = key.code
            && c.is_ascii_digit() && (c != '0' || self.pending_count.is_some()) {
                let digit = c.to_digit(10).unwrap_or(0) as usize;
                let current = self.pending_count.unwrap_or(0);
                self.pending_count = Some(current * 10 + digit);
                return EditorAction::Handled;
            }

        // If we have a pending operator, the next key is a motion
        if self.pending_operator.is_some() {
            return self.handle_operator_motion(key);
        }

        match key.code {
            // --- Movement ---
            KeyCode::Char('h') | KeyCode::Left if !ctrl => {
                let n = self.take_count();
                self.move_left(n);
                EditorAction::Handled
            }
            KeyCode::Char('l') | KeyCode::Right if !ctrl => {
                let n = self.take_count();
                self.move_right(n);
                EditorAction::Handled
            }
            KeyCode::Char('j') | KeyCode::Down if !ctrl => {
                let n = self.take_count();
                self.move_down(n);
                EditorAction::Handled
            }
            KeyCode::Char('k') | KeyCode::Up if !ctrl => {
                let n = self.take_count();
                self.move_up(n);
                EditorAction::Handled
            }
            KeyCode::Char('w') => {
                let n = self.take_count();
                self.move_word_forward(n, false);
                EditorAction::Handled
            }
            KeyCode::Char('W') => {
                let n = self.take_count();
                self.move_word_forward(n, true);
                EditorAction::Handled
            }
            KeyCode::Char('e') => {
                let n = self.take_count();
                self.move_word_end(n, false);
                EditorAction::Handled
            }
            KeyCode::Char('E') => {
                let n = self.take_count();
                self.move_word_end(n, true);
                EditorAction::Handled
            }
            KeyCode::Char('b') => {
                let n = self.take_count();
                self.move_word_back(n, false);
                EditorAction::Handled
            }
            KeyCode::Char('B') => {
                let n = self.take_count();
                self.move_word_back(n, true);
                EditorAction::Handled
            }
            KeyCode::Char('0') => {
                self.move_to_line_start();
                EditorAction::Handled
            }
            KeyCode::Char('^') => {
                self.move_to_first_non_blank();
                EditorAction::Handled
            }
            KeyCode::Char('$') => {
                self.move_to_line_end();
                EditorAction::Handled
            }
            KeyCode::Char('G') => {
                let count = self.pending_count.take();
                if let Some(n) = count {
                    self.move_to_line(n);
                } else {
                    self.move_to_bottom();
                }
                EditorAction::Handled
            }
            KeyCode::Char('g') => {
                self.pending_g = true;
                EditorAction::Handled
            }

            // --- Scroll ---
            KeyCode::Char('d') if ctrl => {
                self.pending_count = None;
                self.half_page_down();
                EditorAction::Handled
            }
            KeyCode::Char('u') if ctrl => {
                self.pending_count = None;
                self.half_page_up();
                EditorAction::Handled
            }

            // --- Insert mode entry ---
            KeyCode::Char('i') => {
                if self.config.insert_allowed {
                    self.pending_count = None;
                    self.start_recording();
                    self.mode = VimMode::Insert;
                }
                EditorAction::Handled
            }
            KeyCode::Char('a') => {
                if self.config.insert_allowed {
                    self.pending_count = None;
                    self.start_recording();
                    self.move_right(1);
                    self.mode = VimMode::Insert;
                }
                EditorAction::Handled
            }
            KeyCode::Char('I') => {
                if self.config.insert_allowed {
                    self.pending_count = None;
                    self.start_recording();
                    self.move_to_first_non_blank();
                    self.mode = VimMode::Insert;
                }
                EditorAction::Handled
            }
            KeyCode::Char('A') => {
                if self.config.insert_allowed {
                    self.pending_count = None;
                    self.start_recording();
                    self.cursor_col = self.current_line_len();
                    self.mode = VimMode::Insert;
                }
                EditorAction::Handled
            }
            KeyCode::Char('o') => {
                if self.config.insert_allowed {
                    self.pending_count = None;
                    self.save_undo();
                    self.start_recording();
                    let indent = {
                        let line = self.current_line();
                        let trimmed = line.trim_start();
                        line[..line.len() - trimmed.len()].to_string()
                    };
                    let row = self.cursor_row + 1;
                    self.lines.insert(row, indent.clone());
                    self.cursor_row = row;
                    self.cursor_col = indent.len();
                    self.mode = VimMode::Insert;
                    self.modified = true;
                }
                EditorAction::Handled
            }
            KeyCode::Char('O') => {
                if self.config.insert_allowed {
                    self.pending_count = None;
                    self.save_undo();
                    self.start_recording();
                    let indent = {
                        let line = self.current_line();
                        let trimmed = line.trim_start();
                        line[..line.len() - trimmed.len()].to_string()
                    };
                    self.lines.insert(self.cursor_row, indent.clone());
                    self.cursor_col = indent.len();
                    self.mode = VimMode::Insert;
                    self.modified = true;
                }
                EditorAction::Handled
            }

            // --- Operators ---
            KeyCode::Char('d') => {
                self.pending_operator = Some(Operator::Delete);
                EditorAction::Handled
            }
            KeyCode::Char('y') => {
                self.pending_operator = Some(Operator::Yank);
                EditorAction::Handled
            }
            KeyCode::Char('c') => {
                if self.config.insert_allowed {
                    self.pending_operator = Some(Operator::Change);
                } else {
                    self.pending_operator = Some(Operator::Delete);
                }
                EditorAction::Handled
            }
            KeyCode::Char('>') => {
                self.pending_operator = Some(Operator::Indent);
                EditorAction::Handled
            }
            KeyCode::Char('<') => {
                self.pending_operator = Some(Operator::Dedent);
                EditorAction::Handled
            }

            // --- Single-key edit ---
            KeyCode::Char('x') => {
                let n = self.take_count();
                self.save_undo();
                for _ in 0..n {
                    self.delete_char_at_cursor();
                }
                EditorAction::Handled
            }
            // --- Undo/Redo ---
            KeyCode::Char('u') if !ctrl => {
                self.pending_count = None;
                self.undo();
                EditorAction::Handled
            }
            KeyCode::Char('r') if ctrl => {
                self.pending_count = None;
                self.redo();
                EditorAction::Handled
            }
            KeyCode::Char('r') => {
                self.pending_replace = true;
                EditorAction::Handled
            }
            KeyCode::Char('"') => {
                self.pending_register = true;
                EditorAction::Handled
            }
            KeyCode::Char('p') => {
                self.save_undo();
                self.paste_from_system_clipboard();
                self.use_system_clipboard = false;
                EditorAction::Handled
            }
            KeyCode::Char('P') => {
                self.save_undo();
                self.paste_from_system_clipboard();
                self.use_system_clipboard = false;
                EditorAction::Handled
            }
            KeyCode::Char('~') => {
                let n = self.take_count();
                for _ in 0..n {
                    self.toggle_case_at_cursor();
                }
                EditorAction::Handled
            }

            // --- Find char (f/F/t/T) ---
            KeyCode::Char('f') => {
                self.pending_count = None;
                self.pending_find = Some((FindDirection::Forward, false));
                EditorAction::Handled
            }
            KeyCode::Char('F') => {
                self.pending_count = None;
                self.pending_find = Some((FindDirection::Backward, false));
                EditorAction::Handled
            }
            KeyCode::Char('t') => {
                self.pending_count = None;
                self.pending_find = Some((FindDirection::Forward, true));
                EditorAction::Handled
            }
            KeyCode::Char('T') => {
                self.pending_count = None;
                self.pending_find = Some((FindDirection::Backward, true));
                EditorAction::Handled
            }

            // --- Substitute (s) ---
            KeyCode::Char('s') if !ctrl => {
                if self.config.insert_allowed {
                    self.save_undo();
                    self.start_recording();
                    self.delete_char_at_cursor();
                    self.mode = VimMode::Insert;
                }
                EditorAction::Handled
            }

            // --- Search ---
            KeyCode::Char('/') => {
                self.pending_count = None;
                self.start_search(true);
                EditorAction::Handled
            }
            KeyCode::Char('?') => {
                self.pending_count = None;
                self.start_search(false);
                EditorAction::Handled
            }
            KeyCode::Char('n') => {
                self.jump_to_next_match();
                EditorAction::Handled
            }
            KeyCode::Char('N') => {
                self.jump_to_prev_match();
                EditorAction::Handled
            }

            // --- Visual mode ---
            KeyCode::Char('v') if ctrl => {
                self.pending_count = None;
                self.enter_visual(VisualKind::Block);
                EditorAction::Handled
            }
            KeyCode::Char('v') => {
                self.pending_count = None;
                self.enter_visual(VisualKind::Char);
                EditorAction::Handled
            }
            KeyCode::Char('V') => {
                self.pending_count = None;
                self.enter_visual(VisualKind::Line);
                EditorAction::Handled
            }

            // --- Repeat ---
            KeyCode::Char('.') => {
                self.repeat_last_edit();
                EditorAction::Handled
            }

            // --- Command mode (:) ---
            KeyCode::Char(':') => {
                self.command_active = true;
                self.command_buffer.clear();
                EditorAction::Handled
            }

            // Space -> pass through to parent as Unhandled
            KeyCode::Char(' ') => EditorAction::Unhandled(key),

            // --- Save buffer ---
            KeyCode::Char('S') if ctrl => EditorAction::Save,

            // --- Execute query ---
            // Query execution is now <leader>Enter, not Ctrl+Enter
            KeyCode::Enter if ctrl => EditorAction::Handled,

            // --- Escape clears pending and search highlights ---
            KeyCode::Esc => {
                self.pending_count = None;
                self.pending_operator = None;
                self.pending_g = false;
                self.pending_find = None;
                self.pending_replace = false;
                self.search.pattern.clear();
                EditorAction::Unhandled(key)
            }

            _ => EditorAction::Unhandled(key),
        }
    }

    fn handle_g_prefix(&mut self, key: KeyEvent) -> EditorAction {
        match key.code {
            KeyCode::Char('g') => {
                let count = self.pending_count.take();
                if let Some(n) = count {
                    self.move_to_line(n);
                } else {
                    self.move_to_top();
                }
                EditorAction::Handled
            }
            KeyCode::Char('U') => {
                self.pending_operator = Some(Operator::Uppercase);
                EditorAction::Handled
            }
            KeyCode::Char('u') => {
                self.pending_operator = Some(Operator::Lowercase);
                EditorAction::Handled
            }
            _ => {
                self.pending_count = None;
                EditorAction::Handled
            }
        }
    }

    fn handle_operator_motion(&mut self, key: KeyEvent) -> EditorAction {
        let op = match self.pending_operator.take() {
            Some(op) => op,
            None => return EditorAction::Handled,
        };
        let count = self.take_count();

        // Check for doubled operator (dd, yy, cc, >>, <<)
        let motion = match key.code {
            KeyCode::Char('d') if op == Operator::Delete => Some(Motion::Line),
            KeyCode::Char('y') if op == Operator::Yank => Some(Motion::Line),
            KeyCode::Char('c') if op == Operator::Change => Some(Motion::Line),
            KeyCode::Char('>') if op == Operator::Indent => Some(Motion::Line),
            KeyCode::Char('<') if op == Operator::Dedent => Some(Motion::Line),

            // Motions
            KeyCode::Char('h') | KeyCode::Left => Some(Motion::Left),
            KeyCode::Char('l') | KeyCode::Right => Some(Motion::Right),
            KeyCode::Char('j') | KeyCode::Down => Some(Motion::Down),
            KeyCode::Char('k') | KeyCode::Up => Some(Motion::Up),
            KeyCode::Char('w') => Some(Motion::WordForward),
            KeyCode::Char('W') => Some(Motion::BigWordForward),
            KeyCode::Char('e') => Some(Motion::WordEnd),
            KeyCode::Char('E') => Some(Motion::BigWordEnd),
            KeyCode::Char('b') => Some(Motion::WordBack),
            KeyCode::Char('B') => Some(Motion::BigWordBack),
            KeyCode::Char('0') => Some(Motion::LineStart),
            KeyCode::Char('$') => Some(Motion::LineEnd),
            KeyCode::Char('^') => Some(Motion::FirstNonBlank),
            KeyCode::Char('G') => Some(Motion::ToBottom),
            KeyCode::Char('g') => {
                // gg
                self.pending_g = false;
                Some(Motion::ToTop)
            }

            // Find char motions (f/F/t/T) - store operator back and set pending_find
            KeyCode::Char('f') => {
                self.pending_operator = Some(op);
                self.pending_find = Some((FindDirection::Forward, false));
                return EditorAction::Handled;
            }
            KeyCode::Char('F') => {
                self.pending_operator = Some(op);
                self.pending_find = Some((FindDirection::Backward, false));
                return EditorAction::Handled;
            }
            KeyCode::Char('t') => {
                self.pending_operator = Some(op);
                self.pending_find = Some((FindDirection::Forward, true));
                return EditorAction::Handled;
            }
            KeyCode::Char('T') => {
                self.pending_operator = Some(op);
                self.pending_find = Some((FindDirection::Backward, true));
                return EditorAction::Handled;
            }

            // Text objects: iw, i", i(
            KeyCode::Char('i') => {
                // Need next char for text object
                // Store operator back and set flag
                self.pending_operator = Some(op);
                // We'll handle text object in a simplified way
                return EditorAction::Handled;
            }

            KeyCode::Esc => return EditorAction::Handled,
            _ => None,
        };

        if let Some(m) = motion {
            self.execute_operator(&op, &m, count);
        }

        EditorAction::Handled
    }

    // --- Insert Mode ---

    fn handle_insert(&mut self, key: KeyEvent) -> EditorAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Esc => {
                self.mode = VimMode::Normal;
                // Move cursor back one if possible (vim behavior)
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
                self.stop_recording();
                self.clamp_cursor();
                EditorAction::Handled
            }
            KeyCode::Char('s') if ctrl => {
                self.mode = VimMode::Normal;
                self.stop_recording();
                EditorAction::Save
            }
            // Query execution is now <leader>Enter (not available in Insert mode)
            KeyCode::Enter if ctrl => EditorAction::Handled,
            KeyCode::Enter => {
                self.save_undo();
                self.insert_newline();
                self.record_key(key);
                EditorAction::Handled
            }
            KeyCode::Backspace => {
                self.save_undo();
                self.backspace();
                self.record_key(key);
                EditorAction::Handled
            }
            KeyCode::Char(c) => {
                self.save_undo();
                self.insert_char(c);
                self.record_key(key);
                EditorAction::Handled
            }
            KeyCode::Tab => {
                self.save_undo();
                // Insert 4 spaces
                for _ in 0..4 {
                    self.insert_char(' ');
                }
                self.record_key(key);
                EditorAction::Handled
            }
            _ => EditorAction::Handled,
        }
    }

    // --- Visual Mode ---

    fn handle_visual(&mut self, key: KeyEvent) -> EditorAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Esc => {
                self.exit_visual();
                EditorAction::Handled
            }
            // Movement (updates selection)
            KeyCode::Char('h') | KeyCode::Left if !ctrl => {
                self.move_left(1);
                EditorAction::Handled
            }
            KeyCode::Char('l') | KeyCode::Right if !ctrl => {
                self.move_right(1);
                EditorAction::Handled
            }
            KeyCode::Char('j') | KeyCode::Down if !ctrl => {
                self.move_down(1);
                EditorAction::Handled
            }
            KeyCode::Char('k') | KeyCode::Up if !ctrl => {
                self.move_up(1);
                EditorAction::Handled
            }
            KeyCode::Char('w') => {
                self.move_word_forward(1, false);
                EditorAction::Handled
            }
            KeyCode::Char('b') => {
                self.move_word_back(1, false);
                EditorAction::Handled
            }
            KeyCode::Char('e') => {
                self.move_word_end(1, false);
                EditorAction::Handled
            }
            KeyCode::Char('0') => {
                self.move_to_line_start();
                EditorAction::Handled
            }
            KeyCode::Char('$') => {
                self.move_to_line_end();
                EditorAction::Handled
            }
            KeyCode::Char('G') => {
                self.move_to_bottom();
                EditorAction::Handled
            }
            KeyCode::Char('g') => {
                self.pending_g = true;
                EditorAction::Handled
            }
            KeyCode::Char('d') if ctrl => {
                self.half_page_down();
                EditorAction::Handled
            }
            KeyCode::Char('u') if ctrl => {
                self.half_page_up();
                EditorAction::Handled
            }

            // Actions on selection
            KeyCode::Char('d') | KeyCode::Char('x') => {
                self.visual_delete();
                EditorAction::Handled
            }
            KeyCode::Char('y') => {
                self.visual_yank();
                EditorAction::Handled
            }
            KeyCode::Char('>') => {
                self.visual_indent();
                EditorAction::Handled
            }
            KeyCode::Char('<') => {
                self.visual_dedent();
                EditorAction::Handled
            }

            // Switch visual sub-mode
            KeyCode::Char('v') if !ctrl => {
                match &self.mode {
                    VimMode::Visual(VisualKind::Char) => self.exit_visual(),
                    _ => {
                        let anchor = self.visual_anchor;
                        self.mode = VimMode::Visual(VisualKind::Char);
                        self.visual_anchor = anchor;
                    }
                }
                EditorAction::Handled
            }
            KeyCode::Char('V') => {
                match &self.mode {
                    VimMode::Visual(VisualKind::Line) => self.exit_visual(),
                    _ => {
                        let anchor = self.visual_anchor;
                        self.mode = VimMode::Visual(VisualKind::Line);
                        self.visual_anchor = anchor;
                    }
                }
                EditorAction::Handled
            }

            _ => EditorAction::Handled,
        }
    }

    // --- Edit Recording (for . repeat) ---

    fn start_recording(&mut self) {
        self.is_recording = true;
        self.recording_edit.clear();
    }

    fn record_key(&mut self, key: KeyEvent) {
        if self.is_recording {
            self.recording_edit.push(key);
        }
    }

    fn stop_recording(&mut self) {
        if self.is_recording {
            self.is_recording = false;
            if !self.recording_edit.is_empty() {
                self.last_edit = Some(super::EditRecord {
                    keys: self.recording_edit.clone(),
                });
            }
        }
    }

    fn repeat_last_edit(&mut self) {
        if let Some(edit) = self.last_edit.clone()
            && self.config.insert_allowed {
                self.save_undo();
                self.mode = VimMode::Insert;
                for key in &edit.keys {
                    // Replay insert mode keys
                    match key.code {
                        KeyCode::Char(c) => self.insert_char(c),
                        KeyCode::Enter => self.insert_newline(),
                        KeyCode::Backspace => self.backspace(),
                        KeyCode::Tab => {
                            for _ in 0..4 {
                                self.insert_char(' ');
                            }
                        }
                        _ => {}
                    }
                }
                self.mode = VimMode::Normal;
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
                self.clamp_cursor();
            }
    }
}
