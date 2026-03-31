use ratatui::layout::{Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::ui::state::{AppState, CenterTab, Panel};
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    let is_focused = state.active_panel == Panel::PackageView;
    let border_style = theme.border_style(is_focused, &state.mode);

    let content = match &state.package_content {
        Some(c) => c,
        None => {
            let block = Block::default()
                .title(" Package ")
                .borders(Borders::ALL)
                .border_style(border_style);
            let p = Paragraph::new("No package selected").block(block);
            frame.render_widget(p, area);
            return;
        }
    };

    match state.active_tab {
        CenterTab::Declaration => {
            let block = Block::default()
                .title(" Declaration ")
                .borders(Borders::ALL)
                .border_style(border_style);
            let p = Paragraph::new(content.declaration.as_str())
                .block(block)
                .wrap(Wrap { trim: false });
            frame.render_widget(p, area);
        }
        CenterTab::Body => {
            let text = content
                .body
                .as_deref()
                .unwrap_or("-- No body available");
            let block = Block::default()
                .title(" Body ")
                .borders(Borders::ALL)
                .border_style(border_style);
            let p = Paragraph::new(text)
                .block(block)
                .wrap(Wrap { trim: false });
            frame.render_widget(p, area);
        }
        _ => {
            // Split view: declaration on top, body on bottom
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    ratatui::layout::Constraint::Percentage(50),
                    ratatui::layout::Constraint::Percentage(50),
                ])
                .split(area);

            let decl_block = Block::default()
                .title(" Declaration ")
                .borders(Borders::ALL)
                .border_style(border_style);
            let decl = Paragraph::new(content.declaration.as_str())
                .block(decl_block)
                .wrap(Wrap { trim: false });
            frame.render_widget(decl, chunks[0]);

            let body_text = content
                .body
                .as_deref()
                .unwrap_or("-- No body available");
            let body_block = Block::default()
                .title(" Body ")
                .borders(Borders::ALL)
                .border_style(border_style);
            let body = Paragraph::new(body_text)
                .block(body_block)
                .wrap(Wrap { trim: false });
            frame.render_widget(body, chunks[1]);
        }
    }
}
