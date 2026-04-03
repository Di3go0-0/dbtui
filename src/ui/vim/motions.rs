use super::buffer::VimEditor;
use super::MotionRange;

impl VimEditor {
    // --- Basic movement ---

    pub fn move_left(&mut self, count: usize) {
        self.cursor_col = self.cursor_col.saturating_sub(count);
    }

    pub fn move_right(&mut self, count: usize) {
        let max = match self.mode {
            super::VimMode::Insert => self.current_line_len(),
            _ => self.current_line_len().saturating_sub(1),
        };
        self.cursor_col = (self.cursor_col + count).min(max);
    }

    pub fn move_down(&mut self, count: usize) {
        self.cursor_row = (self.cursor_row + count).min(self.lines.len().saturating_sub(1));
        self.clamp_cursor();
    }

    pub fn move_up(&mut self, count: usize) {
        self.cursor_row = self.cursor_row.saturating_sub(count);
        self.clamp_cursor();
    }

    // --- Line position ---

    pub fn move_to_line_start(&mut self) {
        self.cursor_col = 0;
    }

    pub fn move_to_first_non_blank(&mut self) {
        let line = self.current_line();
        self.cursor_col = line.len() - line.trim_start().len();
    }

    pub fn move_to_line_end(&mut self) {
        let len = self.current_line_len();
        self.cursor_col = match self.mode {
            super::VimMode::Insert => len,
            _ => len.saturating_sub(1),
        };
    }

    // --- File position ---

    pub fn move_to_top(&mut self) {
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.clamp_cursor();
    }

    pub fn move_to_bottom(&mut self) {
        self.cursor_row = self.lines.len().saturating_sub(1);
        self.clamp_cursor();
    }

    pub fn move_to_line(&mut self, line_num: usize) {
        if line_num > 0 {
            self.cursor_row = (line_num - 1).min(self.lines.len().saturating_sub(1));
        }
        self.clamp_cursor();
    }

    // --- Scroll ---

    pub fn half_page_down(&mut self) {
        let half = self.visible_height / 2;
        self.scroll_offset = self.scroll_offset.saturating_add(half);
        self.cursor_row = (self.cursor_row + half).min(self.lines.len().saturating_sub(1));
        self.clamp_cursor();
    }

    pub fn half_page_up(&mut self) {
        let half = self.visible_height / 2;
        self.scroll_offset = self.scroll_offset.saturating_sub(half);
        self.cursor_row = self.cursor_row.saturating_sub(half);
        self.clamp_cursor();
    }

    // --- Word motions ---

    pub fn move_word_forward(&mut self, count: usize, big_word: bool) {
        for _ in 0..count {
            self.next_word_start(big_word);
        }
    }

    pub fn move_word_end(&mut self, count: usize, big_word: bool) {
        for _ in 0..count {
            self.next_word_end(big_word);
        }
    }

    pub fn move_word_back(&mut self, count: usize, big_word: bool) {
        for _ in 0..count {
            self.prev_word_start(big_word);
        }
    }

    fn is_word_char(c: char, big_word: bool) -> bool {
        if big_word {
            !c.is_whitespace()
        } else {
            c.is_alphanumeric() || c == '_'
        }
    }

    fn next_word_start(&mut self, big_word: bool) {
        let line = self.current_line().to_string();
        let line_len = line.len();
        let mut col = self.cursor_col;

        if col < line_len {
            let chars: Vec<char> = line.chars().collect();
            // Skip current word
            let in_word = col < chars.len() && Self::is_word_char(chars[col], big_word);
            if in_word {
                while col < chars.len() && Self::is_word_char(chars[col], big_word) {
                    col += 1;
                }
            } else {
                while col < chars.len()
                    && !Self::is_word_char(chars[col], big_word)
                    && !chars[col].is_whitespace()
                {
                    col += 1;
                }
            }
            // Skip whitespace
            while col < chars.len() && chars[col].is_whitespace() {
                col += 1;
            }
        }

        if col >= line_len {
            // Move to next line
            if self.cursor_row + 1 < self.lines.len() {
                self.cursor_row += 1;
                let next_line = self.current_line().to_string();
                let chars: Vec<char> = next_line.chars().collect();
                col = 0;
                while col < chars.len() && chars[col].is_whitespace() {
                    col += 1;
                }
                self.cursor_col = col;
            }
        } else {
            self.cursor_col = col;
        }
    }

    fn next_word_end(&mut self, big_word: bool) {
        let mut col = self.cursor_col + 1;
        let line = self.current_line().to_string();
        let chars: Vec<char> = line.chars().collect();

        // Skip whitespace first
        while col < chars.len() && chars[col].is_whitespace() {
            col += 1;
        }

        if col >= chars.len() {
            if self.cursor_row + 1 < self.lines.len() {
                self.cursor_row += 1;
                let next = self.current_line().to_string();
                let nc: Vec<char> = next.chars().collect();
                col = 0;
                while col < nc.len() && nc[col].is_whitespace() {
                    col += 1;
                }
                // Find end of word
                if col < nc.len() {
                    let in_word = Self::is_word_char(nc[col], big_word);
                    while col + 1 < nc.len() {
                        let next_is_word = Self::is_word_char(nc[col + 1], big_word);
                        if in_word != next_is_word {
                            break;
                        }
                        col += 1;
                    }
                }
                self.cursor_col = col;
            }
            return;
        }

        // Find end of current word
        let in_word = Self::is_word_char(chars[col], big_word);
        while col + 1 < chars.len() {
            let next_is_word = Self::is_word_char(chars[col + 1], big_word);
            if in_word != next_is_word {
                break;
            }
            col += 1;
        }
        self.cursor_col = col;
    }

    fn prev_word_start(&mut self, big_word: bool) {
        if self.cursor_col == 0 {
            if self.cursor_row > 0 {
                self.cursor_row -= 1;
                self.cursor_col = self.current_line_len().saturating_sub(1);
            }
            return;
        }

        let line = self.current_line().to_string();
        let chars: Vec<char> = line.chars().collect();
        let mut col = self.cursor_col.min(chars.len()).saturating_sub(1);

        // Skip whitespace
        while col > 0 && chars[col].is_whitespace() {
            col -= 1;
        }

        // Skip to start of word
        let in_word = col < chars.len() && Self::is_word_char(chars[col], big_word);
        while col > 0 {
            let prev_is_word = Self::is_word_char(chars[col - 1], big_word);
            if in_word != prev_is_word {
                break;
            }
            col -= 1;
        }

        self.cursor_col = col;
    }

    // --- Find char motions (f/F/t/T) ---

    pub fn find_char_forward(&mut self, target: char, before: bool) {
        let line = self.current_line().to_string();
        let start = self.cursor_col + 1;
        for (i, c) in line.char_indices().skip(start) {
            if c == target {
                self.cursor_col = if before { i.saturating_sub(1).max(self.cursor_col) } else { i };
                return;
            }
        }
    }

    pub fn find_char_backward(&mut self, target: char, after: bool) {
        let line = self.current_line().to_string();
        let col = self.cursor_col;
        if col == 0 { return; }
        for i in (0..col).rev() {
            if line.as_bytes().get(i) == Some(&(target as u8)) {
                self.cursor_col = if after { (i + 1).min(col) } else { i };
                return;
            }
        }
    }

    // --- Motion range computation (for operators) ---

    /// Compute the range that a motion covers from current position.
    /// Used by operators like d, y, c.
    pub fn compute_motion_range(
        &self,
        motion: &Motion,
        count: usize,
    ) -> Option<MotionRange> {
        let sr = self.cursor_row;
        let sc = self.cursor_col;

        match motion {
            Motion::Left => Some(MotionRange {
                start_row: sr,
                start_col: sc.saturating_sub(count),
                end_row: sr,
                end_col: sc,
                linewise: false,
            }),
            Motion::Right => {
                let max = self.current_line_len();
                Some(MotionRange {
                    start_row: sr,
                    start_col: sc,
                    end_row: sr,
                    end_col: (sc + count).min(max),
                    linewise: false,
                })
            }
            Motion::Down => {
                let er = (sr + count).min(self.lines.len().saturating_sub(1));
                Some(MotionRange {
                    start_row: sr,
                    start_col: 0,
                    end_row: er,
                    end_col: 0,
                    linewise: true,
                })
            }
            Motion::Up => {
                let er = sr.saturating_sub(count);
                Some(MotionRange {
                    start_row: er,
                    start_col: 0,
                    end_row: sr,
                    end_col: 0,
                    linewise: true,
                })
            }
            Motion::LineStart => Some(MotionRange {
                start_row: sr,
                start_col: 0,
                end_row: sr,
                end_col: sc,
                linewise: false,
            }),
            Motion::LineEnd => Some(MotionRange {
                start_row: sr,
                start_col: sc,
                end_row: sr,
                end_col: self.current_line_len(),
                linewise: false,
            }),
            Motion::WordForward => {
                // Clone to simulate
                let mut sim = self.clone_position();
                sim.move_word_forward(count, false);
                if sim.cursor_row == sr {
                    Some(MotionRange {
                        start_row: sr,
                        start_col: sc,
                        end_row: sr,
                        end_col: sim.cursor_col,
                        linewise: false,
                    })
                } else {
                    Some(MotionRange {
                        start_row: sr,
                        start_col: sc,
                        end_row: sim.cursor_row,
                        end_col: sim.cursor_col,
                        linewise: false,
                    })
                }
            }
            Motion::WordEnd => {
                let mut sim = self.clone_position();
                sim.move_word_end(count, false);
                Some(MotionRange {
                    start_row: sr,
                    start_col: sc,
                    end_row: sim.cursor_row,
                    end_col: (sim.cursor_col + 1).min(
                        sim.lines
                            .get(sim.cursor_row)
                            .map(|l| l.len())
                            .unwrap_or(0),
                    ),
                    linewise: false,
                })
            }
            Motion::WordBack => {
                let mut sim = self.clone_position();
                sim.move_word_back(count, false);
                Some(MotionRange {
                    start_row: sim.cursor_row,
                    start_col: sim.cursor_col,
                    end_row: sr,
                    end_col: sc,
                    linewise: false,
                })
            }
            Motion::BigWordForward => {
                let mut sim = self.clone_position();
                sim.move_word_forward(count, true);
                Some(MotionRange {
                    start_row: sr,
                    start_col: sc,
                    end_row: sim.cursor_row,
                    end_col: sim.cursor_col,
                    linewise: false,
                })
            }
            Motion::BigWordEnd => {
                let mut sim = self.clone_position();
                sim.move_word_end(count, true);
                Some(MotionRange {
                    start_row: sr,
                    start_col: sc,
                    end_row: sim.cursor_row,
                    end_col: (sim.cursor_col + 1).min(
                        sim.lines
                            .get(sim.cursor_row)
                            .map(|l| l.len())
                            .unwrap_or(0),
                    ),
                    linewise: false,
                })
            }
            Motion::BigWordBack => {
                let mut sim = self.clone_position();
                sim.move_word_back(count, true);
                Some(MotionRange {
                    start_row: sim.cursor_row,
                    start_col: sim.cursor_col,
                    end_row: sr,
                    end_col: sc,
                    linewise: false,
                })
            }
            Motion::Line => Some(MotionRange {
                start_row: sr,
                start_col: 0,
                end_row: (sr + count).saturating_sub(1).min(self.lines.len().saturating_sub(1)),
                end_col: 0,
                linewise: true,
            }),
            Motion::ToTop => Some(MotionRange {
                start_row: 0,
                start_col: 0,
                end_row: sr,
                end_col: 0,
                linewise: true,
            }),
            Motion::ToBottom => Some(MotionRange {
                start_row: sr,
                start_col: 0,
                end_row: self.lines.len().saturating_sub(1),
                end_col: 0,
                linewise: true,
            }),
            Motion::FirstNonBlank => {
                let line = self.current_line();
                let first_non_blank = line.len() - line.trim_start().len();
                let (s, e) = if first_non_blank <= sc {
                    (first_non_blank, sc)
                } else {
                    (sc, first_non_blank)
                };
                Some(MotionRange {
                    start_row: sr,
                    start_col: s,
                    end_row: sr,
                    end_col: e,
                    linewise: false,
                })
            }
            Motion::InnerWord => {
                let (start, end) = self.find_inner_word();
                Some(MotionRange {
                    start_row: sr,
                    start_col: start,
                    end_row: sr,
                    end_col: end,
                    linewise: false,
                })
            }
            Motion::InnerQuote(quote_char) => {
                if let Some((start, end)) = self.find_inner_delimited(*quote_char, *quote_char) {
                    Some(MotionRange {
                        start_row: sr,
                        start_col: start,
                        end_row: sr,
                        end_col: end,
                        linewise: false,
                    })
                } else {
                    None
                }
            }
            Motion::InnerParen(open, close) => {
                if let Some((start, end)) = self.find_inner_delimited(*open, *close) {
                    Some(MotionRange {
                        start_row: sr,
                        start_col: start,
                        end_row: sr,
                        end_col: end,
                        linewise: false,
                    })
                } else {
                    None
                }
            }
            Motion::FindCharForward(c) => {
                let line = self.current_line().to_string();
                let start = self.cursor_col + 1;
                for (i, ch) in line.char_indices().skip(start) {
                    if ch == *c {
                        return Some(MotionRange {
                            start_row: sr,
                            start_col: sc,
                            end_row: sr,
                            end_col: i + 1, // inclusive for delete
                            linewise: false,
                        });
                    }
                }
                None
            }
            Motion::FindCharBefore(c) => {
                let line = self.current_line().to_string();
                let start = self.cursor_col + 1;
                for (i, ch) in line.char_indices().skip(start) {
                    if ch == *c {
                        return Some(MotionRange {
                            start_row: sr,
                            start_col: sc,
                            end_row: sr,
                            end_col: i,
                            linewise: false,
                        });
                    }
                }
                None
            }
            Motion::FindCharBackward(c) => {
                let line = self.current_line().to_string();
                if sc == 0 { return None; }
                for i in (0..sc).rev() {
                    if line.as_bytes().get(i) == Some(&(*c as u8)) {
                        return Some(MotionRange {
                            start_row: sr,
                            start_col: i,
                            end_row: sr,
                            end_col: sc,
                            linewise: false,
                        });
                    }
                }
                None
            }
            Motion::FindCharAfter(c) => {
                let line = self.current_line().to_string();
                if sc == 0 { return None; }
                for i in (0..sc).rev() {
                    if line.as_bytes().get(i) == Some(&(*c as u8)) {
                        return Some(MotionRange {
                            start_row: sr,
                            start_col: (i + 1).min(sc),
                            end_row: sr,
                            end_col: sc,
                            linewise: false,
                        });
                    }
                }
                None
            }
        }
    }

    fn clone_position(&self) -> PositionSim<'_> {
        PositionSim {
            lines: &self.lines,
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
        }
    }

    fn find_inner_word(&self) -> (usize, usize) {
        let line = self.current_line();
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            return (0, 0);
        }
        let col = self.cursor_col.min(chars.len().saturating_sub(1));
        let c = chars[col];
        let is_word = c.is_alphanumeric() || c == '_';

        let mut start = col;
        let mut end = col;

        if is_word {
            while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
                start -= 1;
            }
            while end + 1 < chars.len()
                && (chars[end + 1].is_alphanumeric() || chars[end + 1] == '_')
            {
                end += 1;
            }
        } else if c.is_whitespace() {
            while start > 0 && chars[start - 1].is_whitespace() {
                start -= 1;
            }
            while end + 1 < chars.len() && chars[end + 1].is_whitespace() {
                end += 1;
            }
        } else {
            while start > 0
                && !chars[start - 1].is_alphanumeric()
                && chars[start - 1] != '_'
                && !chars[start - 1].is_whitespace()
            {
                start -= 1;
            }
            while end + 1 < chars.len()
                && !chars[end + 1].is_alphanumeric()
                && chars[end + 1] != '_'
                && !chars[end + 1].is_whitespace()
            {
                end += 1;
            }
        }

        (start, end + 1)
    }

    fn find_inner_delimited(&self, open: char, close: char) -> Option<(usize, usize)> {
        let line = self.current_line();
        let chars: Vec<char> = line.chars().collect();
        let col = self.cursor_col.min(chars.len().saturating_sub(1));

        // Find opening delimiter backward
        let mut start = None;
        for i in (0..=col).rev() {
            if chars[i] == open {
                start = Some(i + 1);
                break;
            }
        }

        let start = start?;

        // Find closing delimiter forward
        let mut end = None;
        for (i, &ch) in chars.iter().enumerate().skip(col.max(start)) {
            if ch == close {
                end = Some(i);
                break;
            }
        }

        let end = end?;

        if start <= end {
            Some((start, end))
        } else {
            None
        }
    }
}

/// Lightweight position simulator for computing motion ranges without cloning the buffer
struct PositionSim<'a> {
    lines: &'a [String],
    cursor_row: usize,
    cursor_col: usize,
}

impl<'a> PositionSim<'a> {
    fn current_line_len(&self) -> usize {
        self.lines.get(self.cursor_row).map(|l| l.len()).unwrap_or(0)
    }

    fn move_word_forward(&mut self, count: usize, big_word: bool) {
        for _ in 0..count {
            let line = self.lines.get(self.cursor_row).map(|s| s.as_str()).unwrap_or("");
            let chars: Vec<char> = line.chars().collect();
            let mut col = self.cursor_col;

            if col < chars.len() {
                let in_word = VimEditor::is_word_char(chars[col], big_word);
                if in_word {
                    while col < chars.len() && VimEditor::is_word_char(chars[col], big_word) {
                        col += 1;
                    }
                } else {
                    while col < chars.len()
                        && !VimEditor::is_word_char(chars[col], big_word)
                        && !chars[col].is_whitespace()
                    {
                        col += 1;
                    }
                }
                while col < chars.len() && chars[col].is_whitespace() {
                    col += 1;
                }
            }

            if col >= line.len() && self.cursor_row + 1 < self.lines.len() {
                self.cursor_row += 1;
                let next = self.lines.get(self.cursor_row).map(|s| s.as_str()).unwrap_or("");
                let nc: Vec<char> = next.chars().collect();
                col = 0;
                while col < nc.len() && nc[col].is_whitespace() {
                    col += 1;
                }
                self.cursor_col = col;
            } else {
                self.cursor_col = col.min(self.current_line_len());
            }
        }
    }

    fn move_word_end(&mut self, count: usize, big_word: bool) {
        for _ in 0..count {
            let line = self.lines.get(self.cursor_row).map(|s| s.as_str()).unwrap_or("");
            let chars: Vec<char> = line.chars().collect();
            let mut col = self.cursor_col + 1;

            while col < chars.len() && chars[col].is_whitespace() {
                col += 1;
            }

            if col >= chars.len() {
                if self.cursor_row + 1 < self.lines.len() {
                    self.cursor_row += 1;
                    let next = self.lines.get(self.cursor_row).map(|s| s.as_str()).unwrap_or("");
                    let nc: Vec<char> = next.chars().collect();
                    col = 0;
                    while col < nc.len() && nc[col].is_whitespace() {
                        col += 1;
                    }
                    if col < nc.len() {
                        let in_word = VimEditor::is_word_char(nc[col], big_word);
                        while col + 1 < nc.len() {
                            if VimEditor::is_word_char(nc[col + 1], big_word) != in_word {
                                break;
                            }
                            col += 1;
                        }
                    }
                    self.cursor_col = col;
                }
                continue;
            }

            let in_word = VimEditor::is_word_char(chars[col], big_word);
            while col + 1 < chars.len() {
                if VimEditor::is_word_char(chars[col + 1], big_word) != in_word {
                    break;
                }
                col += 1;
            }
            self.cursor_col = col;
        }
    }

    fn move_word_back(&mut self, count: usize, big_word: bool) {
        for _ in 0..count {
            if self.cursor_col == 0 {
                if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                    self.cursor_col = self.current_line_len().saturating_sub(1);
                }
                continue;
            }

            let line = self.lines.get(self.cursor_row).map(|s| s.as_str()).unwrap_or("");
            let chars: Vec<char> = line.chars().collect();
            let mut col = self.cursor_col.min(chars.len()).saturating_sub(1);

            while col > 0 && chars[col].is_whitespace() {
                col -= 1;
            }

            let in_word = col < chars.len() && VimEditor::is_word_char(chars[col], big_word);
            while col > 0 && VimEditor::is_word_char(chars[col - 1], big_word) == in_word {
                col -= 1;
            }

            self.cursor_col = col;
        }
    }
}

/// Motions that can be combined with operators
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordForward,
    WordEnd,
    WordBack,
    BigWordForward,
    BigWordEnd,
    BigWordBack,
    LineStart,
    LineEnd,
    FirstNonBlank,
    Line,      // dd, yy, cc (whole line)
    ToTop,     // gg
    ToBottom,  // G
    InnerWord, // iw
    InnerQuote(char),        // i" i'
    InnerParen(char, char),  // i( i) i{ i}
    FindCharForward(char),   // f
    FindCharBefore(char),    // t
    FindCharBackward(char),  // F
    FindCharAfter(char),     // T
}
