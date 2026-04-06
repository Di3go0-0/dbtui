use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Clear};
use ratatui::Frame;

use crate::ui::state::{AppState, OilPane};
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, state: &mut AppState, theme: &Theme) {
    let area = frame.area();
    let oil = match state.oil.as_ref() {
        Some(o) => o,
        None => return,
    };

    // 80% width, 60% height, centered
    let w = (area.width * 80 / 100).max(40);
    let h = (area.height * 60 / 100).max(10);
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let modal = Rect::new(x, y, w, h);

    // Clear background area so content behind doesn't bleed through borders
    frame.render_widget(Clear, modal);

    // Outer block: rounded borders with accent color, transparent bg
    let outer_block = Block::default()
        .title(" Navigator ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(Color::Reset));

    let inner = outer_block.inner(modal);
    frame.render_widget(outer_block, modal);

    let current_pane = oil.pane;

    // Split 50/50: left = Explorer (connections tree), right = Scripts
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    render_explorer_pane(frame, state, theme, panes[0], current_pane == OilPane::Explorer);
    render_scripts_pane(frame, state, theme, panes[1], current_pane == OilPane::Scripts);
}

fn render_explorer_pane(
    frame: &mut Frame,
    state: &mut AppState,
    theme: &Theme,
    area: Rect,
    is_focused: bool,
) {
    let border_color = if is_focused {
        theme.accent
    } else {
        theme.border_unfocused
    };

    let block = Block::default()
        .title(" Explorer ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(Color::Reset));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Reuse sidebar tree render
    super::sidebar::render_tree(frame, state, theme, inner, is_focused);
}

fn render_scripts_pane(
    frame: &mut Frame,
    state: &mut AppState,
    theme: &Theme,
    area: Rect,
    is_focused: bool,
) {
    // Reuse scripts panel render with focus override
    crate::ui::layout::render_scripts_panel_with_focus(frame, state, theme, area, is_focused);
}
