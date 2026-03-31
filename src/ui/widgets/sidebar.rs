use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::ui::state::{AppState, LeafKind, Panel, TreeNode};
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    let is_focused = state.active_panel == Panel::Sidebar;
    let border_style = theme.border_style(is_focused, &state.mode);

    // Split area for search bar if active
    let (tree_area, search_area) = if state.tree_state.search_active {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let title = if state.object_filter.has_filter("schemas") {
        let schemas = state.all_schema_names();
        let enabled = state.object_filter.filters.get("schemas")
            .map(|s| s.len()).unwrap_or(0);
        format!(" Explorer ({}/{} schemas) ", enabled, schemas.len())
    } else {
        " Explorer ".to_string()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    // Update visible height for scrolloff calculations
    let inner_height = tree_area.height.saturating_sub(2) as usize;
    state.tree_state.visible_height = inner_height.max(1);

    let visible = state.visible_tree();
    let offset = state.tree_state.offset;
    let cursor = state.tree_state.cursor;

    let items: Vec<ListItem> = visible
        .iter()
        .enumerate()
        .skip(offset)
        .take(inner_height)
        .map(|(vis_idx, (_, node))| {
            let depth = node.depth();
            let indent = "  ".repeat(depth);
            let is_search_match = state.tree_state.search_active
                && state.tree_state.search_matches.contains(&vis_idx);

            let match_bg = if is_search_match {
                Color::Rgb(60, 50, 20) // gold tint for matches
            } else {
                Color::Reset
            };

            let line = match node {
                TreeNode::Connection { expanded, name, .. } => {
                    let icon = if *expanded { "▼ " } else { "▶ " };
                    Line::from(vec![
                        Span::raw(indent.clone()),
                        Span::styled(icon, Style::default().fg(theme.tree_expanded)),
                        Span::styled("⊛ ", Style::default().fg(theme.tree_connection)),
                        Span::styled(
                            name.as_str(),
                            Style::default()
                                .fg(theme.tree_connection)
                                .bg(match_bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ])
                }
                TreeNode::Schema { expanded, name, .. } => {
                    let icon = if *expanded { "▼ " } else { "▶ " };
                    Line::from(vec![
                        Span::raw(indent.clone()),
                        Span::styled(
                            icon,
                            Style::default().fg(if *expanded {
                                theme.tree_expanded
                            } else {
                                theme.tree_collapsed
                            }),
                        ),
                        Span::styled("◈ ", Style::default().fg(theme.tree_schema)),
                        Span::styled(
                            name.as_str(),
                            Style::default()
                                .fg(theme.tree_schema)
                                .bg(match_bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ])
                }
                TreeNode::Category {
                    expanded, label, ..
                } => {
                    let icon = if *expanded { "▼ " } else { "▶ " };
                    Line::from(vec![
                        Span::raw(indent.clone()),
                        Span::styled(
                            icon,
                            Style::default().fg(if *expanded {
                                theme.tree_expanded
                            } else {
                                theme.tree_collapsed
                            }),
                        ),
                        Span::styled(
                            label.as_str(),
                            Style::default()
                                .fg(theme.tree_category)
                                .bg(match_bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ])
                }
                TreeNode::Leaf {
                    name, kind, valid, ..
                } => {
                    let (icon, color) = match kind {
                        LeafKind::Table => ("T ", theme.tree_table),
                        LeafKind::View => ("V ", theme.tree_view),
                        LeafKind::Package => ("P ", theme.tree_package),
                        LeafKind::Procedure => ("ƒ ", theme.tree_procedure),
                        LeafKind::Function => ("λ ", theme.tree_function),
                    };

                    let (name_color, invalid_marker) = if *valid {
                        (color, Span::raw(""))
                    } else {
                        (
                            Color::Rgb(220, 80, 80), // Red for invalid
                            Span::styled(
                                " ✗",
                                Style::default()
                                    .fg(Color::Rgb(220, 80, 80))
                                    .add_modifier(Modifier::BOLD),
                            ),
                        )
                    };

                    let icon_color = if *valid { color } else { Color::Rgb(220, 80, 80) };

                    Line::from(vec![
                        Span::raw(indent.clone()),
                        Span::styled(
                            icon,
                            Style::default()
                                .fg(icon_color)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            name.as_str(),
                            Style::default().fg(name_color).bg(match_bg),
                        ),
                        invalid_marker,
                    ])
                }
            };
            ListItem::new(line)
        })
        .collect();

    // Calculate selection relative to the rendered slice
    let selected_in_view = if cursor >= offset && cursor < offset + inner_height {
        Some(cursor - offset)
    } else {
        None
    };

    let mut list_state = ListState::default();
    list_state.select(selected_in_view);

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(theme.tree_selected_bg)
                .fg(theme.tree_selected_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, tree_area, &mut list_state);

    // Render search bar
    if let Some(search_rect) = search_area {
        let query = &state.tree_state.search_query;
        let match_count = state.tree_state.search_matches.len();
        let match_info = if query.is_empty() {
            String::new()
        } else {
            format!(" ({match_count} matches)")
        };

        let line = Line::from(vec![
            Span::styled("/", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
            Span::raw(query.as_str()),
            Span::styled("█", Style::default().fg(theme.accent)),
            Span::styled(match_info, Style::default().fg(theme.dim)),
        ]);
        let bar = Paragraph::new(line).style(Style::default().bg(theme.status_bg));
        frame.render_widget(bar, search_rect);
    }
}
