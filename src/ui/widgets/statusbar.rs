use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::ui::state::{AppState, Mode, Panel};
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    let mode_label = match state.mode {
        Mode::Normal => " NORMAL ",
        Mode::Insert => " INSERT ",
    };
    let mode_style = theme.mode_style(&state.mode);

    let panel_icon = match state.active_panel {
        Panel::Sidebar => "  Explorer",
        Panel::DataGrid => "  Data",
        Panel::Properties => "  Properties",
        Panel::PackageView => "  Package",
        Panel::QueryEditor => "  Editor",
    };

    let hints = match state.active_panel {
        Panel::Sidebar => "q:quit  /:filter  ?:help  e:editor  c:connect",
        Panel::DataGrid => "q:quit  hjkl:nav  C-d/u:page  Tab:switch  ?:help",
        Panel::QueryEditor => match state.mode {
            Mode::Insert => "Esc:normal  C-Enter:execute  C-d:clear",
            Mode::Normal => "i:insert  C-Enter:execute  q:close  C-d:clear",
        },
        _ => "q:quit  Tab:switch  ?:help",
    };

    let (conn_icon, conn_style) = theme.connection_indicator(state.connected);
    let conn_name = state
        .connection_name
        .as_deref()
        .unwrap_or("no connection");

    let sep = Span::styled(" │ ", Style::default().fg(theme.separator));

    let status_color = if state.status_message.starts_with("Error") {
        theme.error_fg
    } else if state.loading {
        theme.conn_connecting
    } else {
        theme.dim
    };

    let line = Line::from(vec![
        Span::styled(mode_label, mode_style),
        Span::raw(" "),
        Span::styled(
            panel_icon,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        sep.clone(),
        Span::styled(&state.status_message, Style::default().fg(status_color)),
        sep.clone(),
        Span::styled(hints, Style::default().fg(theme.dim)),
        sep,
        Span::styled(conn_icon, conn_style),
        Span::raw(" "),
        Span::styled(conn_name, Style::default().fg(theme.status_fg)),
    ]);

    let bar = Paragraph::new(line).style(Style::default().bg(theme.status_bg));
    frame.render_widget(bar, area);
}
