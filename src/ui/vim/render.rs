use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::buffer::VimEditor;
use super::{VimMode, VisualKind};
use crate::ui::theme::Theme;

/// Visual selection range: ((start_row, start_col), (end_row, end_col))
type VisualRange = Option<((usize, usize), (usize, usize))>;

const SQL_KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "INSERT", "INTO", "UPDATE", "DELETE", "SET",
    "JOIN", "LEFT", "RIGHT", "INNER", "OUTER", "FULL", "CROSS", "ON",
    "AND", "OR", "NOT", "IN", "IS", "NULL", "LIKE", "BETWEEN", "EXISTS",
    "AS", "ORDER", "BY", "GROUP", "HAVING", "LIMIT", "OFFSET", "DISTINCT",
    "UNION", "ALL", "CREATE", "ALTER", "DROP", "TABLE", "INDEX", "VIEW",
    "BEGIN", "END", "COMMIT", "ROLLBACK", "DECLARE", "CURSOR", "FETCH",
    "CASE", "WHEN", "THEN", "ELSE", "ASC", "DESC", "COUNT", "SUM", "AVG",
    "MAX", "MIN", "PROCEDURE", "FUNCTION", "PACKAGE", "BODY", "REPLACE",
    "VALUES", "WITH", "RECURSIVE", "TRIGGER", "GRANT", "REVOKE",
    "TYPE", "RETURN", "IF", "ELSIF", "LOOP", "FOR", "WHILE", "EXIT",
    "EXCEPTION", "RAISE", "PRAGMA", "EXECUTE", "IMMEDIATE", "BULK",
    "COLLECT", "FORALL", "OPEN", "CLOSE", "DBMS_OUTPUT", "PUT_LINE",
];

pub fn render(
    frame: &mut Frame,
    editor: &mut VimEditor,
    focused: bool,
    theme: &Theme,
    area: Rect,
    title: &str,
) {
    render_with_options(frame, editor, focused, theme, area, title, None);
}

pub fn render_with_options(
    frame: &mut Frame,
    editor: &mut VimEditor,
    focused: bool,
    theme: &Theme,
    area: Rect,
    title: &str,
    border_override: Option<ratatui::style::Color>,
) {
    // Update visible height based on area (minus borders and command line)
    editor.visible_height = area.height.saturating_sub(3) as usize;

    let default_border = if !focused {
        theme.border_unfocused
    } else {
        match editor.mode {
            VimMode::Insert => theme.border_insert,
            _ => theme.border_focused,
        }
    };
    let border_color = border_override.unwrap_or(default_border);

    let block = Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme.editor_bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 2 {
        return;
    }

    // Content area and command line area
    let content_height = inner.height.saturating_sub(1) as usize;
    let full_width = inner.width as usize;
    let cmd_area = Rect {
        x: inner.x,
        y: inner.y + inner.height - 1,
        width: inner.width,
        height: 1,
    };

    // Compute visual selection range
    let visual_range = if let VimMode::Visual(_) = &editor.mode {
        editor.visual_range()
    } else {
        None
    };
    let visual_kind = match &editor.mode {
        VimMode::Visual(k) => Some(k.clone()),
        _ => None,
    };

    // Render lines — each line is padded to full widget width to prevent ghosting
    let line_count_width = format!("{}", editor.lines.len()).len().max(3);
    let bg_style = Style::default().bg(theme.editor_bg);
    let num_col_width = line_count_width + 2; // digits + 2 spaces
    let available_text_width = full_width.saturating_sub(num_col_width);

    // Pre-truncate lines that exceed available width so their storage
    // outlives the spans that borrow from them.
    let mut truncated_cache: Vec<Option<String>> = Vec::with_capacity(content_height);
    for screen_row in 0..content_height {
        let line_idx = editor.scroll_offset + screen_row;
        if line_idx < editor.lines.len() {
            let line_text = &editor.lines[line_idx];
            let tw = UnicodeWidthStr::width(line_text.as_str());
            if tw > available_text_width {
                truncated_cache.push(Some(truncate_to_width(line_text, available_text_width)));
            } else {
                truncated_cache.push(None);
            }
        } else {
            truncated_cache.push(None);
        }
    }

    let mut rendered_lines: Vec<Line> = Vec::with_capacity(content_height);

    for (screen_row, cached) in truncated_cache.iter().enumerate() {
        let line_idx = editor.scroll_offset + screen_row;
        if line_idx >= editor.lines.len() {
            // Tilde for empty lines past end of file
            let prefix = format!("{:>width$}  ", "~", width = line_count_width);
            let used = prefix.len();
            let mut spans = vec![
                Span::styled(prefix, Style::default().fg(theme.dim)),
            ];
            // Pad to fill entire width
            if used < full_width {
                spans.push(Span::styled(" ".repeat(full_width - used), bg_style));
            }
            rendered_lines.push(Line::from(spans));
            continue;
        }

        let is_cursor_line = line_idx == editor.cursor_row && focused;

        // Use truncated text if the line exceeds viewport width
        let render_text: &str = match cached {
            Some(t) => t.as_str(),
            None => editor.lines[line_idx].as_str(),
        };

        // Relative line numbers (like nvim set relativenumber + number)
        let line_num = if is_cursor_line {
            format!("{:>width$}  ", line_idx + 1, width = line_count_width)
        } else {
            let distance = line_idx.abs_diff(editor.cursor_row);
            format!("{:>width$}  ", distance, width = line_count_width)
        };
        let num_style = if is_cursor_line {
            Style::default()
                .fg(theme.editor_line_nr_active)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.editor_line_nr)
        };

        let num_len = line_num.len();
        let mut spans: Vec<Span> = vec![Span::styled(line_num, num_style)];

        // Check if this line has visual selection
        let line_visual = compute_line_visual(
            line_idx,
            render_text.len(),
            &visual_range,
            &visual_kind,
        );

        if let Some((vis_start, vis_end)) = line_visual {
            render_line_with_visual(render_text, vis_start, vis_end, theme, &mut spans);
        } else {
            highlight_sql_line(render_text, theme, &mut spans);
        }

        // Pad to fill entire width — every line MUST cover full_width
        let used = num_len + UnicodeWidthStr::width(render_text);
        if used < full_width {
            spans.push(Span::styled(" ".repeat(full_width - used), bg_style));
        }

        // Cursor rendering (if on this line and focused)
        if is_cursor_line && focused {
            let cursor_screen_col =
                (line_count_width + 2 + editor.cursor_col) as u16;
            let cursor_screen_row = screen_row as u16;
            frame.set_cursor(
                inner.x + cursor_screen_col,
                inner.y + cursor_screen_row,
            );
        }

        rendered_lines.push(Line::from(spans));
    }

    let content = Paragraph::new(rendered_lines)
        .style(bg_style);
    let content_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(1),
    };
    frame.render_widget(Clear, content_area);
    frame.render_widget(content, content_area);

    // Command line (padded to full width)
    let cmd_text = if !editor.command_line.is_empty() {
        editor.command_line.clone()
    } else {
        format!(
            " {}:{} ",
            editor.cursor_row + 1,
            editor.cursor_col + 1
        )
    };
    let cmd_style = if !editor.command_line.is_empty() {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.dim)
    };
    let cmd_used = UnicodeWidthStr::width(cmd_text.as_str());
    let mut cmd_spans = vec![Span::styled(cmd_text, cmd_style)];
    if cmd_used < full_width {
        cmd_spans.push(Span::styled(" ".repeat(full_width - cmd_used), bg_style));
    }
    let cmd_line = Paragraph::new(Line::from(cmd_spans))
        .style(bg_style);
    frame.render_widget(Clear, cmd_area);
    frame.render_widget(cmd_line, cmd_area);
}

fn compute_line_visual(
    line_idx: usize,
    line_len: usize,
    visual_range: &VisualRange,
    visual_kind: &Option<VisualKind>,
) -> Option<(usize, usize)> {
    let ((sr, sc), (er, ec)) = (*visual_range)?;
    let kind = visual_kind.as_ref()?;

    if line_idx < sr || line_idx > er {
        return None;
    }

    match kind {
        VisualKind::Line => Some((0, line_len)),
        VisualKind::Char => {
            if sr == er {
                Some((sc, (ec + 1).min(line_len)))
            } else if line_idx == sr {
                Some((sc, line_len))
            } else if line_idx == er {
                Some((0, (ec + 1).min(line_len)))
            } else {
                Some((0, line_len))
            }
        }
        VisualKind::Block => {
            let left = sc.min(ec);
            let right = (sc.max(ec) + 1).min(line_len);
            if left < right {
                Some((left, right))
            } else {
                None
            }
        }
    }
}

fn render_line_with_visual<'a>(
    line: &'a str,
    vis_start: usize,
    vis_end: usize,
    theme: &Theme,
    spans: &mut Vec<Span<'a>>,
) {
    let visual_style = Style::default()
        .bg(theme.tree_selected_bg)
        .fg(theme.tree_selected_fg);

    let len = line.len();
    let vs = vis_start.min(len);
    let ve = vis_end.min(len);

    if vs > 0 {
        highlight_sql_segment(&line[..vs], theme, spans);
    }
    if vs < ve {
        spans.push(Span::styled(&line[vs..ve], visual_style));
    }
    if ve < len {
        highlight_sql_segment(&line[ve..], theme, spans);
    }
    if line.is_empty() {
        // Show at least a highlighted space for empty selected lines
        spans.push(Span::styled(" ", visual_style));
    }
}

fn highlight_sql_segment<'a>(text: &'a str, theme: &Theme, spans: &mut Vec<Span<'a>>) {
    highlight_sql_line(text, theme, spans);
}

pub fn highlight_sql_line<'a>(line: &'a str, theme: &Theme, spans: &mut Vec<Span<'a>>) {
    if line.is_empty() {
        return;
    }

    // Check for line comment
    if let Some(comment_pos) = line.find("--") {
        if comment_pos > 0 {
            highlight_tokens(&line[..comment_pos], theme, spans);
        }
        spans.push(Span::styled(
            &line[comment_pos..],
            Style::default()
                .fg(theme.sql_comment)
                .add_modifier(Modifier::ITALIC),
        ));
        return;
    }

    highlight_tokens(line, theme, spans);
}

fn highlight_tokens<'a>(text: &'a str, theme: &Theme, spans: &mut Vec<Span<'a>>) {
    let mut remaining = text;

    while !remaining.is_empty() {
        // Skip leading whitespace
        if remaining.starts_with(|c: char| c.is_whitespace()) {
            let ws_end = remaining
                .find(|c: char| !c.is_whitespace())
                .unwrap_or(remaining.len());
            spans.push(Span::raw(&remaining[..ws_end]));
            remaining = &remaining[ws_end..];
            continue;
        }

        // String literal
        if remaining.starts_with('\'') {
            let end = remaining[1..]
                .find('\'')
                .map(|p| p + 2)
                .unwrap_or(remaining.len());
            spans.push(Span::styled(
                &remaining[..end],
                Style::default().fg(theme.sql_string),
            ));
            remaining = &remaining[end..];
            continue;
        }

        // Number
        if remaining.starts_with(|c: char| c.is_ascii_digit()) {
            let end = remaining
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(remaining.len());
            spans.push(Span::styled(
                &remaining[..end],
                Style::default().fg(theme.sql_number),
            ));
            remaining = &remaining[end..];
            continue;
        }

        // Word (potential keyword or identifier)
        if remaining.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
            let end = remaining
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(remaining.len());
            let word = &remaining[..end];
            let upper = word.to_uppercase();

            if SQL_KEYWORDS.contains(&upper.as_str()) {
                spans.push(Span::styled(
                    word,
                    Style::default()
                        .fg(theme.sql_keyword)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw(word));
            }
            remaining = &remaining[end..];
            continue;
        }

        // Operators and punctuation
        let end = remaining
            .find(|c: char| c.is_alphanumeric() || c == '_' || c == '\'' || c.is_whitespace())
            .unwrap_or(remaining.len())
            .max(1);
        spans.push(Span::styled(
            &remaining[..end],
            Style::default().fg(theme.sql_operator),
        ));
        remaining = &remaining[end..];
    }
}

/// Truncate a string to fit within `max_width` display cells.
/// Respects multi-byte character boundaries.
fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut width = 0;
    let mut end = 0;
    for (i, c) in s.char_indices() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if width + cw > max_width {
            break;
        }
        width += cw;
        end = i + c.len_utf8();
    }
    s[..end].to_string()
}
