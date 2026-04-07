//! Renderer for the experimental oil-style inline connection editor
//! (Proposal D). Shows a floating buffer-like panel where each field is
//! a single line (e.g. `host:     oradb01.uni.edu.co`). The active line
//! is highlighted; the editor's Normal/Insert mode is shown in the
//! bottom status bar of the panel.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::ui::state::{
    AppState, INLINE_CONN_ROWS, InlineConnEditor, InlineConnField, InlineConnMode,
};
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, state: &AppState, theme: &Theme) {
    let ed = match state.dialogs.inline_conn_editor.as_ref() {
        Some(e) => e,
        None => return,
    };

    let area = frame.area();
    let width: u16 = 60;
    // Rows: 1 blank + 8 field rows + 1 blank + error (0 or 1) + 1 blank +
    // 1 status/footer + borders.
    let err_lines = if ed.error_message.is_empty() { 0 } else { 1 };
    let height: u16 = (1 + INLINE_CONN_ROWS.len() as u16 + 1 + err_lines + 2 + 2).min(area.height);
    let dialog = centered_rect(width, height, area);
    frame.render_widget(Clear, dialog);

    // Border: accent in Normal, brighter (conn_connected) in Insert so the
    // mode change is obvious at a glance.
    let border_color = match ed.mode {
        InlineConnMode::Normal => theme.accent,
        InlineConnMode::Insert => theme.conn_connected,
    };
    let title = match ed.mode {
        InlineConnMode::Normal => " inline-conn [NORMAL]  (experimental) ",
        InlineConnMode::Insert => " inline-conn [INSERT]  (experimental) ",
    };
    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme.dialog_bg));
    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    // Build all body lines as a Vec<Line> and render as a single Paragraph.
    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    for (row_idx, field) in INLINE_CONN_ROWS.iter().enumerate() {
        lines.push(field_line(ed, row_idx, *field, theme));
    }
    lines.push(Line::from(""));

    // Error message (single line, dim red)
    if !ed.error_message.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!(" ⚠ {} ", ed.error_message),
                Style::default()
                    .fg(theme.error_fg)
                    .bg(theme.error_bg)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));
    }

    // Status / footer
    lines.push(footer_line(ed, theme));

    let body = Paragraph::new(lines);
    frame.render_widget(body, inner);
}

fn field_line(
    ed: &InlineConnEditor,
    row_idx: usize,
    field: InlineConnField,
    theme: &Theme,
) -> Line<'static> {
    let is_active = row_idx == ed.cursor_row;
    let in_insert = ed.mode == InlineConnMode::Insert;

    // Line-number gutter dot (`▸` for active row, space otherwise).
    let gutter = if is_active {
        Span::styled(
            "▸ ".to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    };

    // Label (dim when inactive).
    let label_style = if is_active {
        Style::default()
            .fg(theme.dialog_field_active)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.dialog_field_inactive)
    };
    let label = format!("{:>6}: ", field.label());
    let label_span = Span::styled(label, label_style);

    // Value
    let val_style = Style::default().fg(theme.topbar_fg);
    let mut spans: Vec<Span<'static>> = vec![gutter, label_span];

    match field {
        InlineConnField::Type => {
            spans.push(Span::styled(ed.db_type_label().to_string(), val_style));
            if is_active {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    "◀ ▶".to_string(),
                    Style::default().fg(theme.accent),
                ));
                spans.push(Span::styled(
                    "  (Tab / l / h)".to_string(),
                    Style::default().fg(theme.dim),
                ));
            }
        }
        InlineConnField::Group => {
            spans.push(Span::styled(ed.group.clone(), val_style));
            if is_active {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    "◀ ▶".to_string(),
                    Style::default().fg(theme.accent),
                ));
            }
        }
        InlineConnField::Password => {
            let shown = if ed.password_visible {
                ed.password.clone()
            } else {
                "•".repeat(ed.password.chars().count())
            };
            spans.push(Span::styled(shown, val_style));
            if is_active && in_insert {
                spans.push(Span::styled(
                    "█".to_string(),
                    Style::default().fg(theme.accent),
                ));
            }
            let badge = if ed.password_visible {
                "  ◉ visible  C-p"
            } else {
                "  ⊘ hidden   C-p"
            };
            spans.push(Span::styled(
                badge.to_string(),
                Style::default().fg(theme.dim),
            ));
        }
        _ => {
            // Free-text fields: Name, Host, Port, Username, Database
            let val = match field {
                InlineConnField::Name => ed.name.clone(),
                InlineConnField::Host => ed.host.clone(),
                InlineConnField::Port => ed.port.clone(),
                InlineConnField::Username => ed.username.clone(),
                InlineConnField::Database => ed.database.clone(),
                _ => String::new(),
            };
            spans.push(Span::styled(val, val_style));
            if is_active && in_insert {
                spans.push(Span::styled(
                    "█".to_string(),
                    Style::default().fg(theme.accent),
                ));
            }
        }
    }

    Line::from(spans)
}

fn footer_line(ed: &InlineConnEditor, theme: &Theme) -> Line<'static> {
    let make_key = |label: &str, fg: Color, bg: Color| {
        Span::styled(
            format!(" {label} "),
            Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
        )
    };

    if ed.connecting {
        const FRAMES: [&str; 4] = ["◜", "◝", "◞", "◟"];
        let idx = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            / 150) as usize
            % FRAMES.len();
        let elapsed = ed
            .connecting_since
            .map(|s| s.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        return Line::from(vec![
            Span::raw("  "),
            Span::styled(
                FRAMES[idx].to_string(),
                Style::default()
                    .fg(theme.conn_connecting)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                "connecting...".to_string(),
                Style::default().fg(theme.conn_connecting),
            ),
            Span::raw("  "),
            Span::styled(format!("{elapsed:.1}s"), Style::default().fg(theme.dim)),
        ]);
    }

    match ed.mode {
        InlineConnMode::Normal => Line::from(vec![
            Span::raw("  "),
            make_key("j/k", Color::Black, theme.dim),
            Span::styled(" move  ", Style::default().fg(theme.dim)),
            make_key("i", Color::Black, theme.dim),
            Span::styled(" insert  ", Style::default().fg(theme.dim)),
            make_key("Enter", Color::Black, theme.conn_connected),
            Span::styled(" connect  ", Style::default().fg(theme.dim)),
            make_key("Esc", Color::Black, theme.dim),
            Span::styled(" cancel", Style::default().fg(theme.dim)),
        ]),
        InlineConnMode::Insert => Line::from(vec![
            Span::raw("  "),
            Span::styled(
                " -- INSERT -- ",
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.conn_connected)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            make_key("Esc", Color::Black, theme.dim),
            Span::styled(" normal  ", Style::default().fg(theme.dim)),
            make_key("Tab", Color::Black, theme.dim),
            Span::styled(" next field", Style::default().fg(theme.dim)),
        ]),
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
