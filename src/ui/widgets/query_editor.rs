use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::ui::state::{AppState, Mode, Panel};
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
];

pub fn render(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    let is_focused = state.active_panel == Panel::QueryEditor;
    let border_style = theme.border_style(is_focused, &state.mode);

    let conn_label = state
        .conn.name
        .as_deref()
        .unwrap_or("no connection");

    let mode_hint = if state.mode == Mode::Insert && is_focused {
        " [INSERT]"
    } else {
        ""
    };

    let title = format!(" Query [{conn_label}]{mode_hint} ");

    if state.editor_content.is_empty() && state.mode == Mode::Normal {
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(Style::default().bg(theme.editor_bg));
        let hint = Paragraph::new(Line::from(vec![
            Span::styled("  Press ", Style::default().fg(theme.dim)),
            Span::styled("i", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
            Span::styled(" to start writing SQL  ", Style::default().fg(theme.dim)),
            Span::styled("Ctrl+Enter", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
            Span::styled(" to execute", Style::default().fg(theme.dim)),
        ]))
        .block(block);
        frame.render_widget(hint, area);
        return;
    }

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(theme.editor_bg));

    let lines: Vec<Line> = state
        .editor_content
        .split('\n')
        .enumerate()
        .map(|(i, line_text)| {
            let line_num = format!("{:>3} ", i + 1);
            let is_cursor_line = i == state.editor_cursor_row;

            let num_style = if is_cursor_line {
                Style::default()
                    .fg(theme.editor_line_nr_active)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.editor_line_nr)
            };

            let mut spans = vec![Span::styled(line_num, num_style)];

            // Syntax highlight the line
            highlight_sql_line(line_text, theme, &mut spans);

            Line::from(spans)
        })
        .collect();

    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(p, area);
}

fn highlight_sql_line<'a>(line: &'a str, theme: &Theme, spans: &mut Vec<Span<'a>>) {
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
            Style::default().fg(theme.sql_comment).add_modifier(Modifier::ITALIC),
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
