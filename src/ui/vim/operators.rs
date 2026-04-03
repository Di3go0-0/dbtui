use super::buffer::VimEditor;
use super::motions::Motion;
use super::{Operator, Register};

impl VimEditor {
    /// Execute operator + motion combination
    pub fn execute_operator(&mut self, op: &Operator, motion: &Motion, count: usize) {
        let range = match self.compute_motion_range(motion, count) {
            Some(r) => r,
            None => return,
        };

        match op {
            Operator::Delete => self.op_delete(range.start_row, range.start_col, range.end_row, range.end_col, range.linewise),
            Operator::Yank => self.op_yank(range.start_row, range.start_col, range.end_row, range.end_col, range.linewise),
            Operator::Change => {
                self.op_delete(range.start_row, range.start_col, range.end_row, range.end_col, range.linewise);
                if self.config.insert_allowed {
                    self.mode = super::VimMode::Insert;
                }
            }
            Operator::Indent => self.op_indent(range.start_row, range.end_row),
            Operator::Dedent => self.op_dedent(range.start_row, range.end_row),
            Operator::Uppercase => self.op_case(range.start_row, range.start_col, range.end_row, range.end_col, true),
            Operator::Lowercase => self.op_case(range.start_row, range.start_col, range.end_row, range.end_col, false),
        }
    }

    fn op_delete(
        &mut self,
        start_row: usize,
        start_col: usize,
        end_row: usize,
        end_col: usize,
        linewise: bool,
    ) {
        self.save_undo();

        if linewise {
            let text = self.delete_lines(start_row, end_row - start_row + 1);
            self.unnamed_register = Register {
                content: text,
                linewise: true,
            };
            self.cursor_row = start_row.min(self.lines.len().saturating_sub(1));
            self.clamp_cursor();
        } else if start_row == end_row {
            let text = self.delete_range(start_col, end_col, start_row);
            self.unnamed_register = Register {
                content: text,
                linewise: false,
            };
            self.cursor_col = start_col;
            self.clamp_cursor();
        } else {
            // Multi-line non-linewise delete
            let mut text = String::new();
            // Get text from start to end of first line
            let first_part = if start_col < self.lines[start_row].len() {
                self.lines[start_row][start_col..].to_string()
            } else {
                String::new()
            };
            text.push_str(&first_part);
            text.push('\n');

            // Get full middle lines
            for row in (start_row + 1)..end_row {
                text.push_str(&self.lines[row]);
                text.push('\n');
            }

            // Get text from start of last line to end_col
            let last_part = if end_col <= self.lines[end_row].len() {
                self.lines[end_row][..end_col].to_string()
            } else {
                self.lines[end_row].clone()
            };
            text.push_str(&last_part);

            // Now perform the actual deletion
            let after_end = if end_col <= self.lines[end_row].len() {
                self.lines[end_row][end_col..].to_string()
            } else {
                String::new()
            };
            self.lines[start_row].truncate(start_col);
            self.lines[start_row].push_str(&after_end);

            // Remove middle + end lines
            if end_row > start_row {
                self.lines.drain((start_row + 1)..=end_row);
            }

            self.unnamed_register = Register {
                content: text,
                linewise: false,
            };
            self.cursor_row = start_row;
            self.cursor_col = start_col;
            self.clamp_cursor();
            self.modified = true;
        }

        // Always copy deleted text to system clipboard
        self.copy_to_system_clipboard(&self.unnamed_register.content.clone());
    }

    fn op_yank(
        &mut self,
        start_row: usize,
        start_col: usize,
        end_row: usize,
        end_col: usize,
        linewise: bool,
    ) {
        self.use_system_clipboard = false;
        if linewise {
            let text: Vec<&str> = self.lines[start_row..=end_row.min(self.lines.len().saturating_sub(1))]
                .iter()
                .map(|s| s.as_str())
                .collect();
            self.unnamed_register = Register {
                content: text.join("\n"),
                linewise: true,
            };
        } else if start_row == end_row {
            let line = &self.lines[start_row];
            let s = start_col.min(line.len());
            let e = end_col.min(line.len());
            self.unnamed_register = Register {
                content: line[s..e].to_string(),
                linewise: false,
            };
        } else {
            let mut text = String::new();
            text.push_str(&self.lines[start_row][start_col..]);
            for row in (start_row + 1)..end_row {
                text.push('\n');
                text.push_str(&self.lines[row]);
            }
            text.push('\n');
            let e = end_col.min(self.lines[end_row].len());
            text.push_str(&self.lines[end_row][..e]);
            self.unnamed_register = Register {
                content: text,
                linewise: false,
            };
        }

        // Always copy to system clipboard
        self.copy_to_system_clipboard(&self.unnamed_register.content.clone());
    }

    fn op_indent(&mut self, start_row: usize, end_row: usize) {
        self.save_undo();
        let end = end_row.min(self.lines.len().saturating_sub(1));
        for row in start_row..=end {
            self.indent_line(row);
        }
    }

    fn op_dedent(&mut self, start_row: usize, end_row: usize) {
        self.save_undo();
        let end = end_row.min(self.lines.len().saturating_sub(1));
        for row in start_row..=end {
            self.dedent_line(row);
        }
    }

    fn op_case(
        &mut self,
        start_row: usize,
        start_col: usize,
        end_row: usize,
        end_col: usize,
        to_upper: bool,
    ) {
        self.save_undo();
        if start_row == end_row {
            let line = &self.lines[start_row];
            let s = start_col.min(line.len());
            let e = end_col.min(line.len());
            let changed: String = if to_upper {
                line[s..e].to_uppercase()
            } else {
                line[s..e].to_lowercase()
            };
            self.lines[start_row] = format!("{}{}{}", &line[..s], changed, &line[e..]);
            self.modified = true;
        }
        // Multi-line case change not commonly needed, keep simple for now
    }

    /// Toggle case of character at cursor (~)
    pub fn toggle_case_at_cursor(&mut self) {
        if self.cursor_row < self.lines.len() && self.cursor_col < self.lines[self.cursor_row].len()
        {
            self.save_undo();
            let line = &self.lines[self.cursor_row];
            let ch = line.chars().nth(self.cursor_col);
            if let Some(c) = ch {
                let toggled: String = if c.is_uppercase() {
                    c.to_lowercase().to_string()
                } else {
                    c.to_uppercase().to_string()
                };
                let col = self.cursor_col;
                // Find byte position
                let byte_start: usize = line.chars().take(col).map(|c| c.len_utf8()).sum();
                let byte_end = byte_start + c.len_utf8();
                self.lines[self.cursor_row] =
                    format!("{}{}{}", &line[..byte_start], toggled, &line[byte_end..]);
                self.cursor_col = (col + 1).min(self.current_line_len().saturating_sub(1));
                self.modified = true;
            }
        }
    }

    /// Replace character at cursor (r)
    #[allow(dead_code)]
    pub fn replace_char(&mut self, new_char: char) {
        if self.cursor_row < self.lines.len() && self.cursor_col < self.lines[self.cursor_row].len()
        {
            self.save_undo();
            let line = &self.lines[self.cursor_row];
            let col = self.cursor_col;
            let byte_start: usize = line.chars().take(col).map(|c| c.len_utf8()).sum();
            if let Some(old_char) = line.chars().nth(col) {
                let byte_end = byte_start + old_char.len_utf8();
                self.lines[self.cursor_row] = format!(
                    "{}{}{}",
                    &line[..byte_start],
                    new_char,
                    &line[byte_end..]
                );
                self.modified = true;
            }
        }
    }
}
