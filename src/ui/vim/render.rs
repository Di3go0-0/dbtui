use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::buffer::VimEditor;
use super::{VimMode, VisualKind};
use crate::ui::theme::Theme;

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
    // Update visible height based on area (minus borders and command line)
    editor.visible_height = area.height.saturating_sub(3) as usize;

    let border_color = if !focused {
        theme.border_unfocused
    } else {
        match editor.mode {
            VimMode::Insert => theme.border_insert,
            _ => theme.border_focused,
        }
    };

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

    // Render lines
    let line_count_width = format!("{}", editor.lines.len()).len().max(3);
    let mut rendered_lines: Vec<Line> = Vec::with_capacity(content_height);

    for screen_row in 0..content_height {
        let line_idx = editor.scroll_offset + screen_row;
        if line_idx >= editor.lines.len() {
            // Tilde for empty lines past end of file
            let mut spans = vec![
                Span::styled(
                    format!("{:>width$}  ", "~", width = line_count_width),
                    Style::default().fg(theme.dim),
                ),
            ];
            rendered_lines.push(Line::from(spans));
            continue;
        }

        let is_cursor_line = line_idx == editor.cursor_row && focused;
        let line_text = &editor.lines[line_idx];

        // Relative line numbers (like nvim set relativenumber + number)
        let line_num = if is_cursor_line {
            // Current line shows absolute number
            format!("{:>width$}  ", line_idx + 1, width = line_count_width)
        } else {
            // Other lines show distance from cursor
            let distance = if line_idx > editor.cursor_row {
                line_idx - editor.cursor_row
            } else {
                editor.cursor_row - line_idx
            };
            format!("{:>width$}  ", distance, width = line_count_width)
        };
        let num_style = if is_cursor_line {
            Style::default()
                .fg(theme.editor_line_nr_active)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.editor_line_nr)
        };

        let mut spans: Vec<Span> = vec![Span::styled(line_num, num_style)];

        // Check if this line has visual selection
        let line_visual = compute_line_visual(
            line_idx,
            line_text.len(),
            &visual_range,
            &visual_kind,
        );

        if let Some((vis_start, vis_end)) = line_visual {
            // Render with visual highlight
            render_line_with_visual(line_text, vis_start, vis_end, theme, &mut spans);
        } else {
            // Normal syntax highlight
            highlight_sql_line(line_text, theme, &mut spans);
        }

        // Cursor rendering (if on this line and focused)
        if is_cursor_line && focused {
            // We set the cursor position for the terminal
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
        .style(Style::default().bg(theme.editor_bg));
    let content_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(1),
    };
    frame.render_widget(content, content_area);

    // Command line
    let cmd_spans = if !editor.command_line.is_empty() {
        vec![Span::styled(
            &editor.command_line,
            Style::default().fg(theme.accent),
        )]
    } else {
        let pos = format!(
            " {}:{} ",
            editor.cursor_row + 1,
            editor.cursor_col + 1
        );
        vec![Span::styled(pos, Style::default().fg(theme.dim))]
    };
    let cmd_line = Paragraph::new(Line::from(cmd_spans))
        .style(Style::default().bg(theme.editor_bg));
    frame.render_widget(cmd_line, cmd_area);
}

fn compute_line_visual(
    line_idx: usize,
    line_len: usize,
    visual_range: &Option<((usize, usize), (usize, usize))>,
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
