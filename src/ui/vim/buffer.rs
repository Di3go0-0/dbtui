use super::{
    EditRecord, FindDirection, Operator, Register, SearchState, Snapshot, VimMode, VimModeConfig, SCROLLOFF,
};

/// A self-contained Vim editor instance with its own buffer, cursor, mode, and state.
/// Each view that needs a Vim editor creates its own VimEditor.
pub struct VimEditor {
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub mode: VimMode,
    pub config: VimModeConfig,

    // Scroll
    pub scroll_offset: usize,
    pub visible_height: usize,

    // Undo/Redo
    pub undo_stack: Vec<Snapshot>,
    pub redo_stack: Vec<Snapshot>,

    // Registers
    pub unnamed_register: Register,

    // Search
    pub search: SearchState,

    // Visual mode anchor
    pub visual_anchor: Option<(usize, usize)>,

    // Pending operator/count for Normal mode commands
    pub pending_count: Option<usize>,
    pub pending_operator: Option<Operator>,
    pub pending_g: bool,
    pub pending_register: bool, // waiting for register name after "
    pub use_system_clipboard: bool, // next yank/paste uses system clipboard
    pub pending_find: Option<(FindDirection, bool)>, // for f/F/t/T (direction, before_flag)
    pub pending_replace: bool, // for r command

    // Repeat
    pub last_edit: Option<EditRecord>,
    pub recording_edit: Vec<crossterm::event::KeyEvent>,
    pub is_recording: bool,

    // Status
    pub modified: bool,
    pub command_line: String,

    // Command mode (:)
    pub command_active: bool,
    pub command_buffer: String,
}

impl VimEditor {
    pub fn new(content: &str, config: VimModeConfig) -> Self {
        let expanded = content.replace('\t', "    ");
        let lines: Vec<String> = if expanded.is_empty() {
            vec![String::new()]
        } else {
            expanded.lines().map(String::from).collect()
        };

        Self {
            lines,
            cursor_row: 0,
            cursor_col: 0,
            mode: VimMode::Normal,
            config,
            scroll_offset: 0,
            visible_height: 20,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            unnamed_register: Register::default(),
            search: SearchState::default(),
            visual_anchor: None,
            pending_count: None,
            pending_operator: None,
            pending_g: false,
            pending_register: false,
            use_system_clipboard: false,
            pending_find: None,
            pending_replace: false,
            last_edit: None,
            recording_edit: Vec::new(),
            is_recording: false,
            modified: false,
            command_line: String::new(),
            command_active: false,
            command_buffer: String::new(),
        }
    }

    pub fn new_empty(config: VimModeConfig) -> Self {
        Self::new("", config)
    }

    pub fn set_content(&mut self, content: &str) {
        let expanded = content.replace('\t', "    ");
        self.lines = if expanded.is_empty() {
            vec![String::new()]
        } else {
            expanded.lines().map(String::from).collect()
        };
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.modified = false;
    }

    pub fn content(&self) -> String {
        self.lines.join("\n")
    }

    /// Get the visually selected text
    pub fn selected_text(&self) -> Option<String> {
        let ((sr, sc), (er, ec)) = self.visual_range()?;
        let kind = match &self.mode {
            super::VimMode::Visual(k) => k.clone(),
            _ => return None,
        };

        match kind {
            super::VisualKind::Line => {
                Some(self.lines[sr..=er].join("\n"))
            }
            super::VisualKind::Char => {
                if sr == er {
                    let line = &self.lines[sr];
                    let s = sc.min(line.len());
                    let e = (ec + 1).min(line.len());
                    Some(line[s..e].to_string())
                } else {
                    let mut text = String::new();
                    let first = &self.lines[sr];
                    text.push_str(&first[sc.min(first.len())..]);
                    for row in (sr + 1)..er {
                        text.push('\n');
                        text.push_str(&self.lines[row]);
                    }
                    text.push('\n');
                    let last = &self.lines[er];
                    text.push_str(&last[..(ec + 1).min(last.len())]);
                    Some(text)
                }
            }
            super::VisualKind::Block => {
                let left = sc.min(ec);
                let right = sc.max(ec) + 1;
                let mut text = String::new();
                for row in sr..=er {
                    let line = &self.lines[row];
                    let s = left.min(line.len());
                    let e = right.min(line.len());
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&line[s..e]);
                }
                Some(text)
            }
        }
    }

    #[allow(dead_code)]
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn current_line(&self) -> &str {
        self.lines.get(self.cursor_row).map(|s| s.as_str()).unwrap_or("")
    }

    pub fn current_line_len(&self) -> usize {
        self.current_line().len()
    }

    /// Clamp cursor column to valid range for current line
    pub fn clamp_cursor(&mut self) {
        let max_col = match self.mode {
            VimMode::Insert => self.current_line_len(),
            _ => self.current_line_len().saturating_sub(1).max(0),
        };
        if self.cursor_col > max_col {
            self.cursor_col = max_col;
        }
        if self.cursor_row >= self.lines.len() {
            self.cursor_row = self.lines.len().saturating_sub(1);
        }
    }

    /// Save current state for undo
    pub fn save_undo(&mut self) {
        self.undo_stack.push(Snapshot {
            lines: self.lines.clone(),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
        });
        self.redo_stack.clear();
    }

    /// Undo last change
    pub fn undo(&mut self) {
        if let Some(snapshot) = self.undo_stack.pop() {
            self.redo_stack.push(Snapshot {
                lines: self.lines.clone(),
                cursor_row: self.cursor_row,
                cursor_col: self.cursor_col,
            });
            self.lines = snapshot.lines;
            self.cursor_row = snapshot.cursor_row;
            self.cursor_col = snapshot.cursor_col;
            self.clamp_cursor();
            self.modified = true;
        }
    }

    /// Redo last undone change
    pub fn redo(&mut self) {
        if let Some(snapshot) = self.redo_stack.pop() {
            self.undo_stack.push(Snapshot {
                lines: self.lines.clone(),
                cursor_row: self.cursor_row,
                cursor_col: self.cursor_col,
            });
            self.lines = snapshot.lines;
            self.cursor_row = snapshot.cursor_row;
            self.cursor_col = snapshot.cursor_col;
            self.clamp_cursor();
            self.modified = true;
        }
    }

    /// Ensure scroll keeps cursor visible with scrolloff
    pub fn ensure_cursor_visible(&mut self) {
        let scrolloff = SCROLLOFF.min(self.visible_height / 2);

        if self.cursor_row < self.scroll_offset + scrolloff {
            self.scroll_offset = self.cursor_row.saturating_sub(scrolloff);
        }

        if self.cursor_row + scrolloff >= self.scroll_offset + self.visible_height {
            self.scroll_offset = (self.cursor_row + scrolloff + 1).saturating_sub(self.visible_height);
        }

        let max_offset = self.lines.len().saturating_sub(self.visible_height);
        if self.scroll_offset > max_offset {
            self.scroll_offset = max_offset;
        }
    }

    // --- Insert mode text operations ---

    pub fn insert_char(&mut self, c: char) {
        if self.cursor_row < self.lines.len() {
            let col = self.cursor_col.min(self.lines[self.cursor_row].len());
            self.lines[self.cursor_row].insert(col, c);
            self.cursor_col = col + 1;
            self.modified = true;
        }
    }

    pub fn insert_newline(&mut self) {
        if self.cursor_row < self.lines.len() {
            let col = self.cursor_col.min(self.lines[self.cursor_row].len());
            let rest = self.lines[self.cursor_row][col..].to_string();
            self.lines[self.cursor_row].truncate(col);
            self.cursor_row += 1;
            self.lines.insert(self.cursor_row, rest);
            self.cursor_col = 0;
            self.modified = true;
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            let col = self.cursor_col.min(self.lines[self.cursor_row].len());
            if col > 0 {
                self.lines[self.cursor_row].remove(col - 1);
                self.cursor_col = col - 1;
                self.modified = true;
            }
        } else if self.cursor_row > 0 {
            let current_line = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
            self.lines[self.cursor_row].push_str(&current_line);
            self.modified = true;
        }
    }

    // --- Delete operations ---

    pub fn delete_char_at_cursor(&mut self) {
        if self.cursor_row < self.lines.len() {
            let line_len = self.lines[self.cursor_row].len();
            if self.cursor_col < line_len {
                let ch = self.lines[self.cursor_row].remove(self.cursor_col);
                self.unnamed_register = Register {
                    content: ch.to_string(),
                    linewise: false,
                };
                self.modified = true;
                self.clamp_cursor();
            }
        }
    }

    #[allow(dead_code)]
    pub fn delete_line(&mut self, row: usize) -> Option<String> {
        if row < self.lines.len() {
            let line = self.lines.remove(row);
            if self.lines.is_empty() {
                self.lines.push(String::new());
            }
            self.clamp_cursor();
            self.modified = true;
            Some(line)
        } else {
            None
        }
    }

    pub fn delete_lines(&mut self, start: usize, count: usize) -> String {
        let end = (start + count).min(self.lines.len());
        let removed: Vec<String> = self.lines.drain(start..end).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        if self.cursor_row >= self.lines.len() {
            self.cursor_row = self.lines.len() - 1;
        }
        self.clamp_cursor();
        self.modified = true;
        removed.join("\n")
    }

    pub fn delete_range(&mut self, start_col: usize, end_col: usize, row: usize) -> String {
        if row >= self.lines.len() {
            return String::new();
        }
        let line_len = self.lines[row].len();
        let s = start_col.min(line_len);
        let e = end_col.min(line_len);
        if s >= e {
            return String::new();
        }
        let removed: String = self.lines[row][s..e].to_string();
        self.lines[row] = format!("{}{}", &self.lines[row][..s], &self.lines[row][e..]);
        self.modified = true;
        removed
    }

    // --- Paste ---

    #[allow(dead_code)]
    pub fn paste_after(&mut self) {
        let reg = self.unnamed_register.clone();
        if reg.content.is_empty() {
            return;
        }
        self.save_undo();
        if reg.linewise {
            let new_lines: Vec<String> = reg.content.lines().map(String::from).collect();
            let insert_at = (self.cursor_row + 1).min(self.lines.len());
            for (i, line) in new_lines.into_iter().enumerate() {
                self.lines.insert(insert_at + i, line);
            }
            self.cursor_row = insert_at;
            self.cursor_col = 0;
        } else {
            let col = (self.cursor_col + 1).min(self.lines[self.cursor_row].len());
            self.lines[self.cursor_row].insert_str(col, &reg.content);
            self.cursor_col = col + reg.content.len() - 1;
        }
        self.modified = true;
    }

    #[allow(dead_code)]
    pub fn paste_before(&mut self) {
        let reg = self.unnamed_register.clone();
        if reg.content.is_empty() {
            return;
        }
        self.save_undo();
        if reg.linewise {
            let new_lines: Vec<String> = reg.content.lines().map(String::from).collect();
            for (i, line) in new_lines.into_iter().enumerate() {
                self.lines.insert(self.cursor_row + i, line);
            }
            self.cursor_col = 0;
        } else {
            let col = self.cursor_col.min(self.lines[self.cursor_row].len());
            self.lines[self.cursor_row].insert_str(col, &reg.content);
            self.cursor_col = col + reg.content.len() - 1;
        }
        self.modified = true;
    }

    // --- Indentation ---

    pub fn indent_line(&mut self, row: usize) {
        if row < self.lines.len() {
            self.lines[row].insert_str(0, "    ");
            self.modified = true;
        }
    }

    pub fn dedent_line(&mut self, row: usize) {
        if row < self.lines.len() {
            let line = &self.lines[row];
            let spaces = line.len() - line.trim_start().len();
            let remove = spaces.min(4);
            if remove > 0 {
                self.lines[row] = self.lines[row][remove..].to_string();
                self.modified = true;
            }
        }
    }

    /// Get effective count: pending_count or 1
    pub fn take_count(&mut self) -> usize {
        self.pending_count.take().unwrap_or(1)
    }

    /// Update command line based on current mode
    pub fn update_command_line(&mut self) {
        if self.command_active {
            self.command_line = format!(":{}", self.command_buffer);
            return;
        }
        self.command_line = match &self.mode {
            VimMode::Normal => {
                if self.search.active {
                    let prefix = if self.search.forward { "/" } else { "?" };
                    format!("{}{}", prefix, self.search.input_buffer)
                } else if self.pending_operator.is_some() || self.pending_count.is_some() {
                    let mut s = String::new();
                    if let Some(n) = self.pending_count {
                        s.push_str(&n.to_string());
                    }
                    if let Some(op) = &self.pending_operator {
                        s.push(match op {
                            Operator::Delete => 'd',
                            Operator::Yank => 'y',
                            Operator::Change => 'c',
                            Operator::Indent => '>',
                            Operator::Dedent => '<',
                            Operator::Uppercase => 'U',
                            Operator::Lowercase => 'u',
                        });
                    }
                    s
                } else {
                    String::new()
                }
            }
            VimMode::Insert => "-- INSERT --".to_string(),
            VimMode::Visual(kind) => {
                let label = match kind {
                    super::VisualKind::Char => "VISUAL",
                    super::VisualKind::Line => "VISUAL LINE",
                    super::VisualKind::Block => "VISUAL BLOCK",
                };
                format!("-- {} --", label)
            }
        };
    }

    // --- System clipboard ---

    pub fn copy_to_system_clipboard(&self, text: &str) {
        // Try xclip first, then xsel, then wl-copy (Wayland)
        let cmds: &[(&str, &[&str])] = &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ];
        for (cmd, args) in cmds {
            if let Ok(mut child) = std::process::Command::new(cmd)
                .args(*args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    use std::io::Write;
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
                return;
            }
        }
    }

    pub fn paste_from_system_clipboard(&mut self) {
        let cmds: &[(&str, &[&str])] = &[
            ("wl-paste", &["--no-newline"]),
            ("xclip", &["-selection", "clipboard", "-o"]),
            ("xsel", &["--clipboard", "--output"]),
        ];
        for (cmd, args) in cmds {
            if let Ok(output) = std::process::Command::new(cmd)
                .args(*args)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output()
                && output.status.success() {
                    if let Ok(text) = String::from_utf8(output.stdout)
                        && !text.is_empty() {
                            self.save_undo();
                            let col = (self.cursor_col + 1).min(self.current_line_len());
                            // Insert text at cursor
                            if text.contains('\n') {
                                let parts: Vec<&str> = text.split('\n').collect();
                                let after = self.lines[self.cursor_row][col..].to_string();
                                self.lines[self.cursor_row].truncate(col);
                                self.lines[self.cursor_row].push_str(parts[0]);
                                for (i, part) in parts[1..].iter().enumerate() {
                                    self.lines.insert(self.cursor_row + 1 + i, part.to_string());
                                }
                                let last_row = self.cursor_row + parts.len() - 1;
                                self.lines[last_row].push_str(&after);
                                self.cursor_row = last_row;
                                self.cursor_col = self.lines[last_row].len() - after.len();
                            } else {
                                self.lines[self.cursor_row].insert_str(col, &text);
                                self.cursor_col = col + text.len() - 1;
                            }
                            self.modified = true;
                        }
                    return;
                }
        }
    }
}
