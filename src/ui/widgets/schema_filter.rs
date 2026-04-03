use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::ui::state::ObjectFilterState;
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, filter: &mut ObjectFilterState, theme: &Theme) {
    let area = frame.size();
    let height = 20u16.min(area.height - 4);
    let dialog = centered_rect(50, height, area);

    frame.render_widget(Clear, dialog);

    let title = format!(" Filter: {} ", filter.current_key);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.dialog_bg));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    if filter.all_items.is_empty() {
        let p = Paragraph::new("  No items to filter.").style(Style::default().fg(theme.dim));
        frame.render_widget(p, inner);
        return;
    }

    let (list_area, search_area, hints_area) = if filter.search_active {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(2),
            ])
            .split(inner);
        (chunks[1], Some(chunks[0]), chunks[2])
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(2)])
            .split(inner);
        (chunks[0], None, chunks[1])
    };

    let vh = list_area.height as usize;
    filter.visible_height = vh.max(1);

    let display = filter.display_list();
    let display_count = display.len();

    let items: Vec<ListItem> = display
        .iter()
        .skip(filter.offset)
        .take(vh)
        .map(|(_real_idx, name)| {
            let enabled = filter.is_item_enabled(name);
            let checkbox = if enabled { "[x]" } else { "[ ]" };
            let check_style = if enabled {
                Style::default()
                    .fg(theme.conn_connected)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(100, 100, 120))
            };
            let name_style = if enabled {
                Style::default().fg(theme.tree_schema)
            } else {
                Style::default().fg(Color::Rgb(100, 100, 120))
            };

            ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(checkbox, check_style),
                Span::raw(" "),
                Span::styled(*name, name_style),
            ]))
        })
        .collect();

    let selected_in_view = if filter.cursor >= filter.offset && filter.cursor < filter.offset + vh {
        Some(filter.cursor - filter.offset)
    } else {
        None
    };

    let mut list_state = ListState::default();
    list_state.select(selected_in_view);

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(theme.tree_selected_bg)
                .fg(theme.tree_selected_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, list_area, &mut list_state);

    if let Some(search_rect) = search_area {
        let line = Line::from(vec![
            Span::styled(
                "/",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(filter.search_query.as_str()),
            Span::styled("█", Style::default().fg(theme.accent)),
            Span::styled(
                format!(" ({display_count} shown)"),
                Style::default().fg(theme.dim),
            ),
        ]);
        let bar = Paragraph::new(line).style(Style::default().bg(theme.status_bg));
        frame.render_widget(bar, search_rect);
    }

    let filter_count = filter
        .filters
        .get(&filter.current_key)
        .map(|s| s.len())
        .unwrap_or(0);
    let info = if filter_count > 0 {
        format!("{}/{} enabled", filter_count, filter.all_items.len())
    } else {
        "All visible".to_string()
    };

    let hints = vec![
        Line::from(Span::styled(&info, Style::default().fg(theme.dim))),
        Line::from(vec![
            Span::styled(
                " Space ",
                Style::default().bg(theme.dim).fg(theme.dialog_bg),
            ),
            Span::styled(" toggle ", Style::default().fg(theme.dim)),
            Span::styled(" a ", Style::default().bg(theme.dim).fg(theme.dialog_bg)),
            Span::styled(" all ", Style::default().fg(theme.dim)),
            Span::styled(" / ", Style::default().bg(theme.dim).fg(theme.dialog_bg)),
            Span::styled(" search ", Style::default().fg(theme.dim)),
            Span::styled(
                " Enter ",
                Style::default().bg(theme.dim).fg(theme.dialog_bg),
            ),
            Span::styled(" apply", Style::default().fg(theme.dim)),
        ]),
    ];
    frame.render_widget(Paragraph::new(hints), hints_area);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
