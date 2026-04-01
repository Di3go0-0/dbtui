use super::buffer::VimEditor;

impl VimEditor {
    /// Start search mode
    pub fn start_search(&mut self, forward: bool) {
        self.search.active = true;
        self.search.forward = forward;
        self.search.input_buffer.clear();
    }

    /// Commit the search and jump to first match
    pub fn commit_search(&mut self) {
        self.search.active = false;
        self.search.pattern = self.search.input_buffer.clone();
        if !self.search.pattern.is_empty() {
            self.jump_to_next_match();
        }
    }

    /// Cancel search
    pub fn cancel_search(&mut self) {
        self.search.active = false;
        self.search.input_buffer.clear();
    }

    /// Jump to next match (n)
    pub fn jump_to_next_match(&mut self) {
        if self.search.pattern.is_empty() {
            return;
        }
        let pattern = self.search.pattern.to_lowercase();

        if self.search.forward {
            self.find_forward(&pattern);
        } else {
            self.find_backward(&pattern);
        }
    }

    /// Jump to previous match (N)
    pub fn jump_to_prev_match(&mut self) {
        if self.search.pattern.is_empty() {
            return;
        }
        let pattern = self.search.pattern.to_lowercase();

        if self.search.forward {
            self.find_backward(&pattern);
        } else {
            self.find_forward(&pattern);
        }
    }

    fn find_forward(&mut self, pattern: &str) {
        let total = self.lines.len();
        // Search from cursor position forward, wrapping around
        let start_col = self.cursor_col + 1;

        // Check current line after cursor
        let line_lower = self.lines[self.cursor_row].to_lowercase();
        if let Some(pos) = line_lower[start_col.min(line_lower.len())..].find(pattern) {
            self.cursor_col = start_col.min(line_lower.len()) + pos;
            self.ensure_cursor_visible();
            return;
        }

        // Check subsequent lines
        for offset in 1..=total {
            let row = (self.cursor_row + offset) % total;
            let line_lower = self.lines[row].to_lowercase();
            if let Some(pos) = line_lower.find(pattern) {
                self.cursor_row = row;
                self.cursor_col = pos;
                self.ensure_cursor_visible();
                return;
            }
        }
    }

    fn find_backward(&mut self, pattern: &str) {
        let total = self.lines.len();

        // Check current line before cursor
        if self.cursor_col > 0 {
            let line_lower = self.lines[self.cursor_row].to_lowercase();
            let search_area = &line_lower[..self.cursor_col.min(line_lower.len())];
            if let Some(pos) = search_area.rfind(pattern) {
                self.cursor_col = pos;
                self.ensure_cursor_visible();
                return;
            }
        }

        // Check previous lines
        for offset in 1..=total {
            let row = (self.cursor_row + total - offset) % total;
            let line_lower = self.lines[row].to_lowercase();
            if let Some(pos) = line_lower.rfind(pattern) {
                self.cursor_row = row;
                self.cursor_col = pos;
                self.ensure_cursor_visible();
                return;
            }
        }
    }
}
