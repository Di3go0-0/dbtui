use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::ui::state::{AppState, Focus, Mode};
use crate::ui::theme::Theme;
use crate::ui::vim::VimMode;

pub fn render(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    // Determine effective mode from active editor if in tab content
    let effective_mode = if state.focus == Focus::TabContent {
        if let Some(tab) = state.active_tab() {
            if let Some(editor) = tab.active_editor() {
                match &editor.mode {
                    VimMode::Normal => Mode::Normal,
                    VimMode::Insert => Mode::Insert,
                    VimMode::Visual(_) => Mode::Visual,
                }
            } else {
                Mode::Normal
            }
        } else {
            Mode::Normal
        }
    } else {
        state.mode.clone()
    };

    let mode_label = match effective_mode {
        Mode::Normal => " NORMAL ",
        Mode::Insert => " INSERT ",
        Mode::Visual => " VISUAL ",
    };
    let mode_style = theme.mode_style(&effective_mode);

    let panel_icon = match state.focus {
        Focus::Sidebar => "  Explorer",
        Focus::ScriptsPanel => "  Scripts",
        Focus::TabContent => {
            if let Some(tab) = state.active_tab() {
                match &tab.kind {
                    crate::ui::tabs::TabKind::Script { .. } => "  Script",
                    crate::ui::tabs::TabKind::Table { .. } => "  Table",
                    crate::ui::tabs::TabKind::Package { .. } => "  Package",
                    crate::ui::tabs::TabKind::Function { .. } => "  Function",
                    crate::ui::tabs::TabKind::Procedure { .. } => "  Procedure",
                }
            } else {
                "  Workspace"
            }
        }
    };

    let hints = match state.focus {
        Focus::Sidebar => "q:quit  /:filter  ?:help  n:new script",
        Focus::ScriptsPanel => "Enter:open  d:delete  D:duplicate  r:rename  n:new",
        Focus::TabContent => match effective_mode {
            Mode::Insert => "Esc:normal",
            Mode::Visual => "Esc:normal  d:delete  y:yank",
            Mode::Normal => "Spc-bd:close  Spc-c:connection  {/}:sub-view  [/]:tabs",
        },
    };

    // Show script-specific connection if active tab is a script with one assigned
    let (script_conn, has_script_conn) = if let Some(tab) = state.active_tab() {
        if let crate::ui::tabs::TabKind::Script { conn_name: Some(cn), .. } = &tab.kind {
            (cn.as_str(), true)
        } else {
            ("", false)
        }
    } else {
        ("", false)
    };

    let (conn_icon, conn_style) = if has_script_conn {
        theme.connection_indicator(true)
    } else {
        theme.connection_indicator(state.connected)
    };
    let conn_name = if has_script_conn {
        script_conn
    } else {
        state.connection_name.as_deref().unwrap_or("no connection")
    };

    let sep = Span::styled(" \u{2502} ", Style::default().fg(theme.separator));

    let status_color = if state.status_message.starts_with("Error") {
        theme.error_fg
    } else if state.loading {
        theme.conn_connecting
    } else {
        theme.dim
    };

    let version = Span::styled(" v0.1.0 ", Style::default().fg(theme.dim));

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
        Span::raw("  "),
        version,
    ]);

    let bar = Paragraph::new(line).style(Style::default().bg(theme.status_bg));
    frame.render_widget(bar, area);
}
