use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph};

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
    let area = frame.area();
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
        Span::styled(
            " Enter ",
            Style::default()
                .bg(theme.conn_connected)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Connect ", Style::default().fg(theme.dim)),
        Span::styled(
            " n ",
            Style::default()
                .bg(theme.dim)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" New ", Style::default().fg(theme.dim)),
        Span::styled(
            " d ",
            Style::default()
                .bg(theme.error_fg)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Delete ", Style::default().fg(theme.dim)),
        Span::styled(
            " Esc ",
            Style::default()
                .bg(theme.dim)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Cancel", Style::default().fg(theme.dim)),
    ]);
    frame.render_widget(Paragraph::new(hints), chunks[1]);
}

// -------------- Connection dialog — Proposal B layout --------------
//
// Grouped sections with right-aligned labels, a vertical `│` separator
// between the label column and the value column, and a dynamic title
// that shows the connection Name in the top-right corner as you type.
// Adds a live connecting spinner with elapsed seconds.

/// Labels in the visual order used by the dialog. Maps 1:1 to
/// `CONN_FIELD_VISUAL_ORDER` in `ui::state`.
fn label_for_field(idx: usize) -> &'static str {
    match idx {
        0 => "Name",
        1 => "Type",
        2 => "Host",
        3 => "Port",
        4 => "Username",
        5 => "Password",
        6 => "Database",
        7 => "Group",
        _ => "",
    }
}

/// Returns the coloured span sequence for the value column of `field`.
/// Handles text fields, the Type selector (with inline ◀ ▶ hints when
/// focused), the Group selector, and the Password mask with a trailing
/// visibility badge.
fn value_spans(
    field: usize,
    form: &ConnectionFormState,
    theme: &Theme,
    is_selected: bool,
) -> Vec<Span<'static>> {
    let val_style = Style::default().fg(theme.topbar_fg);
    let hint_style = Style::default().fg(theme.dim);
    let cursor = if is_selected { "█" } else { "" };

    match field {
        // Name / Host / Port / Database / Username — plain text
        0 | 2 | 3 | 4 | 6 => {
            let s = match field {
                0 => form.name.clone(),
                2 => form.host.clone(),
                3 => form.port.clone(),
                4 => form.username.clone(),
                6 => form.database.clone(),
                _ => String::new(),
            };
            vec![
                Span::styled(s, val_style),
                Span::styled(cursor.to_string(), Style::default().fg(theme.accent)),
            ]
        }
        // Type — selector with cycling arrows when focused
        1 => {
            let mut spans = vec![Span::styled(form.db_type_label().to_string(), val_style)];
            if is_selected {
                spans.push(Span::raw("  "));
                spans.push(Span::styled("◀ ▶", Style::default().fg(theme.accent)));
                spans.push(Span::styled("  C-t cycle".to_string(), hint_style));
            }
            spans
        }
        // Password — mask + visibility badge
        5 => {
            let display = if form.password_visible {
                form.password.clone()
            } else {
                "•".repeat(form.password.chars().count())
            };
            let badge_text = if form.password_visible {
                " ◉ visible "
            } else {
                " ⊘ hidden  "
            };
            let badge_style = if form.password_visible {
                Style::default()
                    .fg(theme.dialog_bg)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.dim)
            };
            let mut spans = vec![
                Span::styled(display, val_style),
                Span::styled(cursor.to_string(), Style::default().fg(theme.accent)),
            ];
            if is_selected {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(badge_text.to_string(), badge_style));
                spans.push(Span::styled("  C-p toggle".to_string(), hint_style));
            } else {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(badge_text.to_string(), badge_style));
            }
            spans
        }
        // Group — selector
        7 => {
            let mut spans = vec![Span::styled(form.group.clone(), val_style)];
            if is_selected {
                spans.push(Span::raw("  "));
                spans.push(Span::styled("◀ ▶", Style::default().fg(theme.accent)));
                spans.push(Span::styled("  C-g cycle".to_string(), hint_style));
            }
            spans
        }
        _ => vec![],
    }
}

/// Build a single label/value row. Labels are right-aligned in a 10-wide
/// column, followed by ` │ ` and the value spans.
fn field_row(field: usize, form: &ConnectionFormState, theme: &Theme) -> Line<'static> {
    let is_selected = field == form.selected_field;
    let label_fg = if is_selected {
        theme.dialog_field_active
    } else {
        theme.dialog_field_inactive
    };
    let sep_fg = if is_selected { theme.accent } else { theme.dim };
    let label_style = Style::default().fg(label_fg).add_modifier(if is_selected {
        Modifier::BOLD
    } else {
        Modifier::empty()
    });

    let label = label_for_field(field);
    let mut spans = vec![
        Span::styled(format!("  {label:>10}  "), label_style),
        Span::styled(
            "│  ".to_string(),
            Style::default().fg(sep_fg).add_modifier(if is_selected {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        ),
    ];
    spans.extend(value_spans(field, form, theme, is_selected));
    Line::from(spans)
}

/// Section separator line: "─ {title} ───────────"
fn section_header(title: &str, width: u16, theme: &Theme) -> Line<'static> {
    let inner_width = width.saturating_sub(6) as usize;
    let title_text = format!(" {title} ");
    let dashes = inner_width.saturating_sub(title_text.len() + 1);
    let rule: String = "─".repeat(dashes);
    Line::from(vec![
        Span::raw("  "),
        Span::styled("─", Style::default().fg(theme.dim)),
        Span::styled(title_text, Style::default().fg(theme.accent)),
        Span::styled(rule, Style::default().fg(theme.dim)),
    ])
}

/// Animated spinner + elapsed timer for the "connecting" state.
fn connecting_line(form: &ConnectionFormState, theme: &Theme) -> Line<'static> {
    const FRAMES: [&str; 4] = ["◜", "◝", "◞", "◟"];
    let frame_idx = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        / 150) as usize
        % FRAMES.len();
    let elapsed = form
        .connecting_since
        .map(|s| s.elapsed().as_secs_f64())
        .unwrap_or(0.0);
    let target = if form.host.is_empty() {
        form.db_type_label().to_string()
    } else {
        format!("{}:{}", form.host, form.port)
    };
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            FRAMES[frame_idx].to_string(),
            Style::default()
                .fg(theme.conn_connecting)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!("Connecting to {target}..."),
            Style::default().fg(theme.conn_connecting),
        ),
        Span::raw("  "),
        Span::styled(format!("{elapsed:.1}s"), Style::default().fg(theme.dim)),
    ])
}

fn footer_nav_line(theme: &Theme) -> Line<'static> {
    let key = |label: &str, fg: Color, bg: Color| {
        Span::styled(
            format!(" {label} "),
            Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
        )
    };
    Line::from(vec![
        Span::raw("  "),
        key("Enter", Color::Black, theme.conn_connected),
        Span::styled(" Connect   ", Style::default().fg(theme.dim)),
        key("Esc", Color::Black, theme.dim),
        Span::styled(" Cancel   ", Style::default().fg(theme.dim)),
        key("Tab", Color::Black, theme.dim),
        Span::styled(" Next field", Style::default().fg(theme.dim)),
    ])
}

fn render_form(frame: &mut Frame, form: &ConnectionFormState, theme: &Theme) {
    let area = frame.area();

    // Dialog sizing. Base height covers all field rows, section headers,
    // padding, and the footer. Extra lines grow for multi-line errors.
    let err_lines = if form.error_message.is_empty() {
        0
    } else {
        form.error_message.lines().count() as u16
    };
    let base_height: u16 = 20;
    let dialog_height = (base_height + err_lines).min(area.height.saturating_sub(2));
    let width: u16 = 66;
    let dialog = centered_rect(width, dialog_height, area);
    frame.render_widget(Clear, dialog);

    // Title: left = dialog kind, right = [name] as you type.
    let left_title = if form.read_only {
        " Connection [READ ONLY] "
    } else if form.editing_name.is_some() {
        " Edit Connection "
    } else {
        " New Connection "
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(left_title)
        .title_alignment(Alignment::Left)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.dialog_bg));
    if !form.name.is_empty() {
        let right_title = Line::from(vec![
            Span::styled(" ", Style::default().fg(theme.dim)),
            Span::styled(
                format!("[{}]", form.name),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default().fg(theme.dim)),
        ]);
        block = block.title_top(right_title.right_aligned());
    }
    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    // Build every line of the dialog body in order and render them as a
    // single Paragraph so vertical spacing "just works" without layout
    // chunks for every row.
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(""),
        // Header block: Name / Type / Group
        field_row(0, form, theme),
        field_row(1, form, theme),
        field_row(7, form, theme),
        Line::from(""),
        section_header("Connection", width, theme),
        field_row(2, form, theme),
        field_row(3, form, theme),
        field_row(6, form, theme),
        Line::from(""),
        section_header("Authentication", width, theme),
        field_row(4, form, theme),
        field_row(5, form, theme),
        Line::from(""),
    ];

    // Error block (headline + detail + hint)
    if !form.error_message.is_empty() {
        let mut msg_lines = form.error_message.lines();
        if let Some(headline) = msg_lines.next() {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!(" {headline} "),
                    Style::default()
                        .fg(theme.error_fg)
                        .bg(theme.error_bg)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        for line in msg_lines {
            let is_hint = line.starts_with("Hint:");
            let fg = if is_hint { theme.accent } else { theme.dim };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(line.to_string(), Style::default().fg(fg)),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Status / footer lines: spinner OR nav hints
    if form.connecting {
        lines.push(connecting_line(form, theme));
    } else {
        lines.push(footer_nav_line(theme));
    }

    // Render as a single paragraph inside the block's inner area.
    let content = Paragraph::new(lines);
    let body_rect = Rect::new(inner.x, inner.y, inner.width, inner.height);
    frame.render_widget(content, body_rect);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
