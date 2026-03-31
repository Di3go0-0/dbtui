use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, theme: &Theme) {
    let area = frame.size();
    let dialog = centered_rect(60, 22, area);

    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .title(" Help - Keybindings ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused))
        .style(Style::default().bg(Color::Rgb(25, 25, 35)));

    let header = Style::default()
        .fg(theme.tab_active_fg)
        .add_modifier(Modifier::BOLD);
    let key = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let desc = Style::default().fg(theme.status_fg);

    let lines = vec![
        Line::from(Span::styled(" Navigation", header)),
        Line::from(vec![
            Span::styled("  h/j/k/l       ", key),
            Span::styled("Move within panel", desc),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+h/j/k/l  ", key),
            Span::styled("Switch panels", desc),
        ]),
        Line::from(vec![
            Span::styled("  Enter         ", key),
            Span::styled("Expand/select", desc),
        ]),
        Line::from(vec![
            Span::styled("  Tab           ", key),
            Span::styled("Cycle center tabs", desc),
        ]),
        Line::from(vec![
            Span::styled("  g / G         ", key),
            Span::styled("Top / bottom", desc),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Editor", header)),
        Line::from(vec![
            Span::styled("  e             ", key),
            Span::styled("Open query editor", desc),
        ]),
        Line::from(vec![
            Span::styled("  i             ", key),
            Span::styled("Enter insert mode", desc),
        ]),
        Line::from(vec![
            Span::styled("  Esc           ", key),
            Span::styled("Back to normal mode", desc),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+Enter    ", key),
            Span::styled("Execute query", desc),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Global", header)),
        Line::from(vec![
            Span::styled("  c             ", key),
            Span::styled("New connection", desc),
        ]),
        Line::from(vec![
            Span::styled("  ?             ", key),
            Span::styled("Toggle this help", desc),
        ]),
        Line::from(vec![
            Span::styled("  q             ", key),
            Span::styled("Quit", desc),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            " Press Esc or ? to close",
            Style::default().fg(theme.grid_null),
        )),
    ];

    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(p, dialog);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
