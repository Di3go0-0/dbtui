use super::buffer::VimEditor;
use super::{Register, VimMode, VisualKind};

impl VimEditor {
    /// Enter visual mode
    pub fn enter_visual(&mut self, kind: VisualKind) {
        if !self.config.visual_allowed {
            return;
        }
        self.visual_anchor = Some((self.cursor_row, self.cursor_col));
        self.mode = VimMode::Visual(kind);
    }

    /// Get the ordered (start, end) of the visual selection
    pub fn visual_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.visual_anchor?;
        let cursor = (self.cursor_row, self.cursor_col);

        let (start, end) = if anchor <= cursor {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        };

        Some((start, end))
    }

    /// Delete visual selection
    pub fn visual_delete(&mut self) {
        if let Some(kind) = self.visual_kind() {
            self.save_undo();

            match kind {
                VisualKind::Line => {
                    if let Some(((sr, _), (er, _))) = self.visual_range() {
                        let text = self.delete_lines(sr, er - sr + 1);
                        self.unnamed_register = Register {
                            content: text,
                            linewise: true,
                        };
                    }
                }
                VisualKind::Char => {
                    if let Some(((sr, sc), (er, ec))) = self.visual_range() {
                        let ec = (ec + 1).min(
                            self.lines
                                .get(er)
                                .map(|l| l.len())
                                .unwrap_or(0),
                        );
                        if sr == er {
                            let text = self.delete_range(sc, ec, sr);
                            self.unnamed_register = Register {
                                content: text,
                                linewise: false,
                            };
                            self.cursor_col = sc;
                        } else {
                            // Multi-line char delete
                            let mut text = String::new();
                            text.push_str(&self.lines[sr][sc..]);
                            for row in (sr + 1)..er {
                                text.push('\n');
                                text.push_str(&self.lines[row]);
                            }
                            text.push('\n');
                            text.push_str(&self.lines[er][..ec.min(self.lines[er].len())]);

                            let after = self.lines[er][ec.min(self.lines[er].len())..].to_string();
                            self.lines[sr].truncate(sc);
                            self.lines[sr].push_str(&after);
                            if er > sr {
                                self.lines.drain((sr + 1)..=er);
                            }

                            self.unnamed_register = Register {
                                content: text,
                                linewise: false,
                            };
                            self.cursor_row = sr;
                            self.cursor_col = sc;
                            self.modified = true;
                        }
                    }
                }
                VisualKind::Block => {
                    // Block delete: remove columns from each line in the range
                    if let Some(((sr, sc), (er, ec))) = self.visual_range() {
                        let left = sc.min(ec);
                        let right = sc.max(ec) + 1;
                        let mut text = String::new();
                        for row in sr..=er.min(self.lines.len().saturating_sub(1)) {
                            let line_len = self.lines[row].len();
                            let l = left.min(line_len);
                            let r = right.min(line_len);
                            if l < r {
                                if !text.is_empty() {
                                    text.push('\n');
                                }
                                text.push_str(&self.lines[row][l..r]);
                                self.lines[row] = format!(
                                    "{}{}",
                                    &self.lines[row][..l],
                                    &self.lines[row][r..]
                                );
                            }
                        }
                        self.unnamed_register = Register {
                            content: text,
                            linewise: false,
                        };
                        self.cursor_row = sr;
                        self.cursor_col = left;
                        self.modified = true;
                    }
                }
            }

            // Always copy deleted text to system clipboard
            self.copy_to_system_clipboard(&self.unnamed_register.content.clone());
            self.exit_visual();
            self.clamp_cursor();
        }
    }

    /// Yank visual selection
    pub fn visual_yank(&mut self) {
        if let Some(kind) = self.visual_kind() {
            match kind {
                VisualKind::Line => {
                    if let Some(((sr, _), (er, _))) = self.visual_range() {
                        let end = er.min(self.lines.len().saturating_sub(1));
                        let text: Vec<&str> = self.lines[sr..=end]
                            .iter()
                            .map(|s| s.as_str())
                            .collect();
                        self.unnamed_register = Register {
                            content: text.join("\n"),
                            linewise: true,
                        };
                    }
                }
                VisualKind::Char => {
                    if let Some(((sr, sc), (er, ec))) = self.visual_range() {
                        let ec = (ec + 1).min(
                            self.lines.get(er).map(|l| l.len()).unwrap_or(0),
                        );
                        if sr == er {
                            let line = &self.lines[sr];
                            let s = sc.min(line.len());
                            let e = ec.min(line.len());
                            self.unnamed_register = Register {
                                content: line[s..e].to_string(),
                                linewise: false,
                            };
                        } else {
                            let mut text = String::new();
                            text.push_str(&self.lines[sr][sc..]);
                            for row in (sr + 1)..er {
                                text.push('\n');
                                text.push_str(&self.lines[row]);
                            }
                            text.push('\n');
                            text.push_str(&self.lines[er][..ec.min(self.lines[er].len())]);
                            self.unnamed_register = Register {
                                content: text,
                                linewise: false,
                            };
                        }
                    }
                }
                VisualKind::Block => {
                    if let Some(((sr, sc), (er, ec))) = self.visual_range() {
                        let left = sc.min(ec);
                        let right = sc.max(ec) + 1;
                        let mut text = String::new();
                        for row in sr..=er.min(self.lines.len().saturating_sub(1)) {
                            let line_len = self.lines[row].len();
                            let l = left.min(line_len);
                            let r = right.min(line_len);
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            if l < r {
                                text.push_str(&self.lines[row][l..r]);
                            }
                        }
                        self.unnamed_register = Register {
                            content: text,
                            linewise: false,
                        };
                    }
                }
            }
            // Always copy to system clipboard
            self.copy_to_system_clipboard(&self.unnamed_register.content.clone());
            self.use_system_clipboard = false;
            self.exit_visual();
        }
    }

    /// Indent visual selection
    pub fn visual_indent(&mut self) {
        if let Some(((sr, _), (er, _))) = self.visual_range() {
            self.save_undo();
            let end = er.min(self.lines.len().saturating_sub(1));
            for row in sr..=end {
                self.indent_line(row);
            }
            self.exit_visual();
        }
    }

    /// Dedent visual selection
    pub fn visual_dedent(&mut self) {
        if let Some(((sr, _), (er, _))) = self.visual_range() {
            self.save_undo();
            let end = er.min(self.lines.len().saturating_sub(1));
            for row in sr..=end {
                self.dedent_line(row);
            }
            self.exit_visual();
        }
    }

    /// Exit visual mode
    pub fn exit_visual(&mut self) {
        self.visual_anchor = None;
        self.mode = super::VimMode::Normal;
    }

    fn visual_kind(&self) -> Option<VisualKind> {
        match &self.mode {
            VimMode::Visual(kind) => Some(kind.clone()),
            _ => None,
        }
    }
}
