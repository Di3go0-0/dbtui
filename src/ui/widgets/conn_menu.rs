use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState};

use crate::ui::state::{ConnMenuAction, ConnMenuState};
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, menu: &ConnMenuState, theme: &Theme) {
    let area = frame.area();
    let dialog = centered_rect(35, 10, area);

    frame.render_widget(Clear, dialog);

    let title = format!(" {} ", menu.conn_name);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.dialog_bg));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    let actions = ConnMenuAction::all();
    let items: Vec<ListItem> = actions
        .iter()
        .map(|action| {
            let available = match action {
                ConnMenuAction::Connect => !menu.is_connected,
                ConnMenuAction::Disconnect => menu.is_connected,
                ConnMenuAction::Restart => menu.is_connected,
                _ => true,
            };

            let icon_color = match action {
                ConnMenuAction::Connect => theme.conn_connected,
                ConnMenuAction::Disconnect => theme.conn_disconnected,
                ConnMenuAction::Restart => theme.conn_connecting,
                ConnMenuAction::Delete => Color::Rgb(220, 80, 80),
                _ => theme.accent,
            };

            let (fg, modifier) = if available {
                (theme.topbar_fg, Modifier::empty())
            } else {
                (theme.dim, Modifier::DIM)
            };

            ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{} ", action.icon()),
                    Style::default().fg(icon_color).add_modifier(modifier),
                ),
                Span::styled(
                    action.label(),
                    Style::default().fg(fg).add_modifier(modifier),
                ),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(menu.cursor));

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(theme.tree_selected_bg)
                .fg(theme.tree_selected_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, inner, &mut list_state);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
