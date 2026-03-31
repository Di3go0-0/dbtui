use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::ui::state::{AppState, CenterTab, LeafKind, Overlay};
use crate::ui::theme::Theme;
use crate::ui::widgets;

const SIDEBAR_MIN_WIDTH: u16 = 22;

pub fn render(frame: &mut Frame, state: &mut AppState, theme: &Theme) {
    let area = frame.size();

    // Root: top bar (1) + main content (fill) + status bar (1)
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);

    render_topbar(frame, state, theme, root[0]);

    // Main: sidebar + center
    let sidebar_width = (area.width / 5).max(SIDEBAR_MIN_WIDTH).min(area.width / 3);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(sidebar_width),
            Constraint::Min(20),
        ])
        .split(root[1]);

    widgets::sidebar::render(frame, &mut *state, theme, main[0]);
    render_center(frame, state, theme, main[1]);

    widgets::statusbar::render(frame, state, theme, root[2]);

    // Render overlays on top
    match &state.overlay {
        Some(Overlay::ConnectionDialog) => {
            widgets::connection_dialog::render(
                frame,
                &state.connection_form,
                &state.saved_connections,
                theme,
            );
        }
        Some(Overlay::Help) => {
            widgets::help::render(frame, theme);
        }
        Some(Overlay::ConnectionMenu) => {
            widgets::conn_menu::render(frame, &state.conn_menu, theme);
        }
        Some(Overlay::ObjectFilter) => {
            widgets::schema_filter::render(frame, &mut state.object_filter, theme);
        }
        _ => {}
    }
}

fn render_topbar(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    let (conn_icon, conn_style) = theme.connection_indicator(state.connected);
    let conn_name = state
        .connection_name
        .as_deref()
        .unwrap_or("not connected");
    let db_label = state
        .db_type
        .as_ref()
        .map(|t| t.to_string())
        .unwrap_or_default();
    let schema = state.current_schema.as_deref().unwrap_or("");

    let status_text = if state.connected {
        "CONNECTED"
    } else {
        "DISCONNECTED"
    };

    let sep = Span::styled(" │ ", Style::default().fg(theme.separator));

    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled(conn_icon, conn_style),
        Span::raw(" "),
        Span::styled(
            conn_name,
            Style::default()
                .fg(theme.topbar_fg)
                .add_modifier(Modifier::BOLD),
        ),
        sep.clone(),
        Span::styled(
            &db_label,
            Style::default().fg(theme.accent),
        ),
        sep.clone(),
        Span::styled(
            schema,
            Style::default()
                .fg(theme.tree_schema)
                .add_modifier(Modifier::BOLD),
        ),
        sep,
        Span::styled(
            status_text,
            if state.connected {
                Style::default().fg(theme.conn_connected)
            } else {
                Style::default().fg(theme.conn_disconnected)
            },
        ),
    ]);

    let bar = Paragraph::new(line).style(Style::default().bg(theme.topbar_bg));
    frame.render_widget(bar, area);
}

fn render_center(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    if state.show_editor {
        // Split: content on top (60%), editor on bottom (40%)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);

        render_content_area(frame, state, theme, chunks[0]);
        widgets::query_editor::render(frame, state, theme, chunks[1]);
    } else {
        render_content_area(frame, state, theme, area);
    }
}

fn render_content_area(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    // Tab bar (1 line) + content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3)])
        .split(area);

    render_tab_bar(frame, state, theme, chunks[0]);

    match state.active_tab {
        CenterTab::Data => widgets::data_grid::render(frame, state, theme, chunks[1]),
        CenterTab::Properties => widgets::properties::render(frame, state, theme, chunks[1]),
        CenterTab::Declaration | CenterTab::Body => {
            widgets::package_viewer::render(frame, state, theme, chunks[1])
        }
        CenterTab::DDL => {
            let block = ratatui::widgets::Block::default()
                .title(" DDL ")
                .borders(ratatui::widgets::Borders::ALL)
                .border_style(theme.border_style(false, &state.mode));
            let p = ratatui::widgets::Paragraph::new("DDL view (not yet implemented)").block(block);
            frame.render_widget(p, chunks[1]);
        }
    }
}

fn render_tab_bar(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    let tabs = available_tabs(state);

    let spans: Vec<Span> = tabs
        .iter()
        .flat_map(|tab| {
            let is_active = *tab == state.active_tab;
            vec![
                Span::raw(" "),
                Span::styled(tab_label(tab), theme.tab_style(is_active)),
                Span::raw(" │"),
            ]
        })
        .collect();

    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(Style::default().bg(theme.status_bg));
    frame.render_widget(bar, area);
}

fn available_tabs(state: &mut AppState) -> Vec<CenterTab> {
    let selected_idx = state.selected_tree_index();
    let selected = selected_idx.and_then(|idx| state.tree.get(idx));
    match selected {
        Some(crate::ui::state::TreeNode::Leaf {
            kind: LeafKind::Package,
            ..
        }) => vec![CenterTab::Declaration, CenterTab::Body],
        Some(crate::ui::state::TreeNode::Leaf {
            kind: LeafKind::Table | LeafKind::View,
            ..
        }) => vec![CenterTab::Data, CenterTab::Properties, CenterTab::DDL],
        _ => vec![CenterTab::Data, CenterTab::Properties],
    }
}

fn tab_label(tab: &CenterTab) -> &str {
    match tab {
        CenterTab::Data => "Data",
        CenterTab::Properties => "Properties",
        CenterTab::DDL => "DDL",
        CenterTab::Declaration => "Declaration",
        CenterTab::Body => "Body",
    }
}
