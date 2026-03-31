use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::core::models::ConnectionConfig;
use crate::ui::state::ConnectionFormState;
use crate::ui::theme::Theme;

pub fn render(
    frame: &mut Frame,
    form: &ConnectionFormState,
    saved: &[ConnectionConfig],
    theme: &Theme,
) {
    if form.show_saved_list {
        render_saved_list(frame, form, saved, theme);
    } else {
        render_form(frame, form, theme);
    }
}

fn render_saved_list(
    frame: &mut Frame,
    form: &ConnectionFormState,
    saved: &[ConnectionConfig],
    theme: &Theme,
) {
    let area = frame.size();
    let height = (saved.len() as u16 + 7).min(area.height - 4).max(10);
    let dialog = centered_rect(55, height, area);

    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .title(" Saved Connections ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.dialog_bg));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(inner);

    // Connection list
    let mut items: Vec<ListItem> = saved
        .iter()
        .map(|config| {
            let db_icon = match config.db_type {
                crate::core::models::DatabaseType::Oracle => "O",
                crate::core::models::DatabaseType::PostgreSQL => "P",
                crate::core::models::DatabaseType::MySQL => "M",
            };
            let db_color = match config.db_type {
                crate::core::models::DatabaseType::Oracle => theme.tree_package,
                crate::core::models::DatabaseType::PostgreSQL => theme.tree_view,
                crate::core::models::DatabaseType::MySQL => theme.tree_table,
            };
            ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    db_icon,
                    Style::default().fg(db_color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    config.name.as_str(),
                    Style::default()
                        .fg(theme.topbar_fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}@{}", config.username, config.host),
                    Style::default().fg(theme.dim),
                ),
            ]))
        })
        .collect();

    // Add "New Connection" option at the end
    items.push(ListItem::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "+",
            Style::default()
                .fg(theme.conn_connected)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            "New Connection...",
            Style::default()
                .fg(theme.conn_connected)
                .add_modifier(Modifier::BOLD),
        ),
    ])));

    let mut list_state = ListState::default();
    list_state.select(Some(form.saved_cursor));

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(theme.tree_selected_bg)
                .fg(theme.tree_selected_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    // Hints
    let hints = Line::from(vec![
        Span::raw("  "),
        Span::styled(" Enter ", Style::default().bg(theme.conn_connected).fg(theme.dialog_bg).add_modifier(Modifier::BOLD)),
        Span::styled(" Connect ", Style::default().fg(theme.dim)),
        Span::styled(" n ", Style::default().bg(theme.dim).fg(theme.dialog_bg).add_modifier(Modifier::BOLD)),
        Span::styled(" New ", Style::default().fg(theme.dim)),
        Span::styled(" d ", Style::default().bg(theme.error_fg).fg(theme.dialog_bg).add_modifier(Modifier::BOLD)),
        Span::styled(" Delete ", Style::default().fg(theme.dim)),
        Span::styled(" Esc ", Style::default().bg(theme.dim).fg(theme.dialog_bg).add_modifier(Modifier::BOLD)),
        Span::styled(" Cancel", Style::default().fg(theme.dim)),
    ]);
    frame.render_widget(Paragraph::new(hints), chunks[1]);
}

fn render_form(frame: &mut Frame, form: &ConnectionFormState, theme: &Theme) {
    let area = frame.size();
    let dialog = centered_rect(58, 20, area);

    frame.render_widget(Clear, dialog);

    let title = if form.connecting {
        " Connecting... "
    } else {
        " New Connection "
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.dialog_bg));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    let fields = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(2),
        ])
        .split(inner);

    let field_names = ["Name", "Type", "Host", "Port", "Username", "Password", "Database"];
    let password_display = if form.password_visible {
        form.password.clone()
    } else {
        "*".repeat(form.password.len())
    };
    let field_values = [
        form.name.as_str(),
        form.db_type_label(),
        form.host.as_str(),
        form.port.as_str(),
        form.username.as_str(),
        &password_display,
        form.database.as_str(),
    ];

    for (i, (name, value)) in field_names.iter().zip(field_values.iter()).enumerate() {
        let is_selected = i == form.selected_field;
        let label_color = if is_selected {
            theme.dialog_field_active
        } else {
            theme.dialog_field_inactive
        };

        let label_style = Style::default()
            .fg(label_color)
            .add_modifier(if is_selected {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });

        let value_str = if is_selected {
            format!(" {value}█")
        } else {
            format!(" {value}")
        };

        let bracket_style = if is_selected {
            Style::default().fg(theme.accent)
        } else {
            Style::default().fg(theme.dim)
        };

        let pw_hint = if i == 5 {
            let vis_label = if form.password_visible {
                "hide"
            } else {
                "show"
            };
            Span::styled(
                format!(" [C-p]{vis_label}"),
                Style::default().fg(theme.dim),
            )
        } else if i == 1 {
            Span::styled(" [C-t]switch", Style::default().fg(theme.dim))
        } else {
            Span::raw("")
        };

        let line = Line::from(vec![
            Span::styled(format!("  {name:<10}"), label_style),
            Span::styled("[", bracket_style),
            Span::styled(value_str, Style::default().fg(theme.topbar_fg)),
            Span::styled("]", bracket_style),
            pw_hint,
        ]);
        frame.render_widget(Paragraph::new(line), fields[i]);
    }

    let bottom = fields[7];
    let mut bottom_lines = vec![];

    if !form.error_message.is_empty() {
        bottom_lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!(" {} ", form.error_message),
                Style::default().fg(theme.error_fg).bg(theme.error_bg),
            ),
        ]));
    }

    if form.connecting {
        bottom_lines.push(Line::from(Span::styled(
            "  Connecting...",
            Style::default()
                .fg(theme.conn_connecting)
                .add_modifier(Modifier::BOLD),
        )));
    } else {
        bottom_lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                " Enter ",
                Style::default()
                    .fg(theme.dialog_bg)
                    .bg(theme.conn_connected)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Connect  ", Style::default().fg(theme.dim)),
            Span::styled(
                " Esc ",
                Style::default()
                    .fg(theme.dialog_bg)
                    .bg(theme.dim)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Cancel  ", Style::default().fg(theme.dim)),
            Span::styled(
                " Tab ",
                Style::default()
                    .fg(theme.dialog_bg)
                    .bg(theme.dim)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Next", Style::default().fg(theme.dim)),
        ]));
    }

    frame.render_widget(Paragraph::new(bottom_lines), bottom);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
