use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, theme: &Theme) {
    let area = frame.area();
    let dialog = centered_rect(64, 38, area);

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
            Span::styled("  h/j/k/l        ", key),
            Span::styled("Move cursor", desc),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+h/l       ", key),
            Span::styled("Switch panels", desc),
        ]),
        Line::from(vec![
            Span::styled("  Tab / S-Tab    ", key),
            Span::styled("Prev / next tab", desc),
        ]),
        Line::from(vec![
            Span::styled("  ] / [          ", key),
            Span::styled("Prev / next sub-view", desc),
        ]),
        Line::from(vec![
            Span::styled("  g / G          ", key),
            Span::styled("Top / bottom", desc),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+d / u     ", key),
            Span::styled("Half page down / up", desc),
        ]),
        Line::from(vec![
            Span::styled("  / / n / N      ", key),
            Span::styled("Search / next / prev match", desc),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Editor", header)),
        Line::from(vec![
            Span::styled("  i / a / o      ", key),
            Span::styled("Enter insert mode", desc),
        ]),
        Line::from(vec![
            Span::styled("  Esc            ", key),
            Span::styled("Back to normal mode", desc),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+S         ", key),
            Span::styled("Save / validate", desc),
        ]),
        Line::from(vec![
            Span::styled("  Space Space s  ", key),
            Span::styled("Compile to database", desc),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+Enter     ", key),
            Span::styled("Execute query", desc),
        ]),
        Line::from(vec![
            Span::styled("  v / V          ", key),
            Span::styled("Visual / visual line mode", desc),
        ]),
        Line::from(vec![
            Span::styled("  u / Ctrl+r     ", key),
            Span::styled("Undo / redo", desc),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Tabs & Views", header)),
        Line::from(vec![
            Span::styled("  Enter          ", key),
            Span::styled("Open object from tree", desc),
        ]),
        Line::from(vec![
            Span::styled("  Space b d      ", key),
            Span::styled("Close buffer", desc),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Scripts Panel (Oil-style)", header)),
        Line::from(vec![
            Span::styled("  i / o          ", key),
            Span::styled("New (name/ = folder)", desc),
        ]),
        Line::from(vec![
            Span::styled("  dd             ", key),
            Span::styled("Delete", desc),
        ]),
        Line::from(vec![
            Span::styled("  r              ", key),
            Span::styled("Rename", desc),
        ]),
        Line::from(vec![
            Span::styled("  yy             ", key),
            Span::styled("Yank (copy)", desc),
        ]),
        Line::from(vec![
            Span::styled("  p              ", key),
            Span::styled("Paste (move)", desc),
        ]),
        Line::from(vec![
            Span::styled("  l / Enter      ", key),
            Span::styled("Open / expand", desc),
        ]),
        Line::from(vec![
            Span::styled("  h              ", key),
            Span::styled("Collapse", desc),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Diagnostics", header)),
        Line::from(vec![
            Span::styled("  ]d             ", key),
            Span::styled("Next error", desc),
        ]),
        Line::from(vec![
            Span::styled("  [d             ", key),
            Span::styled("Previous error", desc),
        ]),
        Line::from(vec![
            Span::styled("  K              ", key),
            Span::styled("Show error details", desc),
        ]),
        Line::from(vec![
            Span::styled("  Space x        ", key),
            Span::styled("Toggle error list", desc),
        ]),
        Line::from(vec![
            Span::styled("  gcc            ", key),
            Span::styled("Toggle line comment", desc),
        ]),
        Line::from(vec![
            Span::styled("  gc (visual)    ", key),
            Span::styled("Toggle block comment", desc),
        ]),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Global", header)),
        Line::from(vec![
            Span::styled("  a              ", key),
            Span::styled("Add connection", desc),
        ]),
        Line::from(vec![
            Span::styled("  F              ", key),
            Span::styled("Filter objects", desc),
        ]),
        Line::from(vec![
            Span::styled("  ?              ", key),
            Span::styled("Toggle this help", desc),
        ]),
        Line::from(vec![
            Span::styled("  q              ", key),
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
