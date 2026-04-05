use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::core::models::ObjectPrivilege;
use crate::ui::state::{AppState, ConnStatus, Focus, LeafKind, TreeNode};
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    let is_focused = state.focus == Focus::Sidebar;
    let border_style = theme.border_style(is_focused, &state.mode);

    let (tree_area, search_area) = if state.tree_state.search_active || state.group_creating {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let title = " Explorer ".to_string();

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

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
            let is_selected = vis_idx == cursor;
            let is_search_match = state.tree_state.search_active
                && state.tree_state.search_matches.contains(&vis_idx);

            let row_bg = if is_selected {
                theme.tree_selected_bg
            } else if is_search_match {
                Color::Rgb(60, 50, 20)
            } else {
                Color::Reset
            };

            let line = match node {
                TreeNode::Group { expanded, name, .. } => {
                    // Inline rename mode
                    if state
                        .group_renaming
                        .as_ref()
                        .is_some_and(|rn| rn == name)
                    {
                        let rename_line =
                            format!("{indent}■ {}█", state.group_rename_buf);
                        Line::from(Span::styled(
                            rename_line,
                            Style::default()
                                .fg(theme.conn_connecting)
                                .bg(row_bg)
                                .add_modifier(Modifier::BOLD),
                        ))
                    } else {
                        let icon = if *expanded { "▼ " } else { "▶ " };
                        let name_fg = if is_selected {
                            theme.tree_selected_fg
                        } else {
                            theme.dim
                        };
                        Line::from(vec![
                            Span::styled(indent.clone(), Style::default().bg(row_bg)),
                            Span::styled(
                                icon,
                                Style::default()
                                    .fg(if *expanded {
                                        theme.tree_expanded
                                    } else {
                                        theme.tree_collapsed
                                    })
                                    .bg(row_bg),
                            ),
                            Span::styled(
                                "■ ",
                                Style::default().fg(theme.accent).bg(row_bg),
                            ),
                            Span::styled(
                                name.as_str(),
                                Style::default()
                                    .fg(name_fg)
                                    .bg(row_bg)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ])
                    }
                }
                TreeNode::Connection {
                    expanded,
                    name,
                    status,
                    ..
                } => {
                    let icon = if *expanded { "▼ " } else { "▶ " };
                    let (status_icon, status_color) = match status {
                        ConnStatus::Connected => ("● ", theme.conn_connected),
                        ConnStatus::Disconnected => ("○ ", theme.dim),
                        ConnStatus::Connecting => ("◐ ", theme.conn_connecting),
                        ConnStatus::Failed => ("✗ ", theme.error_fg),
                    };
                    let name_fg = if is_selected {
                        theme.tree_selected_fg
                    } else {
                        theme.tree_connection
                    };
                    Line::from(vec![
                        Span::styled(indent.clone(), Style::default().bg(row_bg)),
                        Span::styled(icon, Style::default().fg(theme.tree_expanded).bg(row_bg)),
                        Span::styled(status_icon, Style::default().fg(status_color).bg(row_bg)),
                        Span::styled(
                            name.as_str(),
                            Style::default()
                                .fg(name_fg)
                                .bg(row_bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ])
                }
                TreeNode::Schema { expanded, name, .. } => {
                    let icon = if *expanded { "▼ " } else { "▶ " };
                    let is_own_schema = state
                        .current_schema
                        .as_ref()
                        .is_some_and(|cs| cs.eq_ignore_ascii_case(name));
                    let name_fg = if is_selected {
                        theme.tree_selected_fg
                    } else {
                        theme.tree_schema
                    };
                    let (schema_icon, icon_color, name_style) = if is_own_schema {
                        (
                            "◉ ",
                            Color::Green,
                            Style::default()
                                .fg(name_fg)
                                .bg(row_bg)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        (
                            "◇ ",
                            theme.tree_schema,
                            Style::default().fg(name_fg).bg(row_bg),
                        )
                    };
                    Line::from(vec![
                        Span::styled(indent.clone(), Style::default().bg(row_bg)),
                        Span::styled(
                            icon,
                            Style::default()
                                .fg(if *expanded {
                                    theme.tree_expanded
                                } else {
                                    theme.tree_collapsed
                                })
                                .bg(row_bg),
                        ),
                        Span::styled(schema_icon, Style::default().fg(icon_color).bg(row_bg)),
                        Span::styled(name.as_str(), name_style),
                    ])
                }
                TreeNode::Category {
                    expanded, label, ..
                } => {
                    let icon = if *expanded { "▼ " } else { "▶ " };
                    let label_fg = if is_selected {
                        theme.tree_selected_fg
                    } else {
                        theme.tree_category
                    };
                    Line::from(vec![
                        Span::styled(indent.clone(), Style::default().bg(row_bg)),
                        Span::styled(
                            icon,
                            Style::default()
                                .fg(if *expanded {
                                    theme.tree_expanded
                                } else {
                                    theme.tree_collapsed
                                })
                                .bg(row_bg),
                        ),
                        Span::styled(
                            label.as_str(),
                            Style::default()
                                .fg(label_fg)
                                .bg(row_bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ])
                }
                TreeNode::Leaf {
                    name, kind, valid, privilege, schema, ..
                } => {
                    let (icon, base_color) = match kind {
                        LeafKind::Table => ("T ", theme.tree_table),
                        LeafKind::View => ("V ", theme.tree_view),
                        LeafKind::Package => ("P ", theme.tree_package),
                        LeafKind::Procedure => ("ƒ ", theme.tree_procedure),
                        LeafKind::Function => ("λ ", theme.tree_function),
                    };

                    let (name_color, icon_color) = if !valid {
                        (Color::Rgb(220, 80, 80), Color::Rgb(220, 80, 80))
                    } else if is_selected {
                        (theme.tree_selected_fg, base_color)
                    } else {
                        (base_color, base_color)
                    };

                    let invalid_marker = if !valid {
                        Span::styled(
                            " ✗",
                            Style::default()
                                .fg(Color::Rgb(220, 80, 80))
                                .bg(row_bg)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        Span::styled("", Style::default().bg(row_bg))
                    };

                    let is_own = state
                        .current_schema
                        .as_ref()
                        .is_some_and(|cs| cs.eq_ignore_ascii_case(schema));
                    let priv_span = if is_own {
                        Span::styled("", Style::default().bg(row_bg))
                    } else {
                        match privilege {
                            ObjectPrivilege::Full => {
                                Span::styled("🔓", Style::default().fg(Color::Green).bg(row_bg))
                            }
                            ObjectPrivilege::ReadOnly => {
                                Span::styled("🔒", Style::default().fg(Color::Yellow).bg(row_bg))
                            }
                            ObjectPrivilege::Execute => {
                                Span::styled("⚡", Style::default().fg(Color::Cyan).bg(row_bg))
                            }
                            ObjectPrivilege::Unknown => {
                                Span::styled("", Style::default().bg(row_bg))
                            }
                        }
                    };

                    Line::from(vec![
                        Span::styled(indent.clone(), Style::default().bg(row_bg)),
                        priv_span,
                        Span::styled(
                            icon,
                            Style::default()
                                .fg(icon_color)
                                .bg(row_bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(name.as_str(), Style::default().fg(name_color).bg(row_bg)),
                        invalid_marker,
                    ])
                }
            };

            // Determine connection name for this node (for scoped filter hints)
            let conn_name_for_hint = match node {
                TreeNode::Connection { name, .. } => name.as_str(),
                _ => {
                    // Walk backwards in visible tree to find parent connection
                    visible[..=vis_idx]
                        .iter()
                        .rev()
                        .find_map(|(_, n)| match n {
                            TreeNode::Connection { name, .. } => Some(name.as_str()),
                            _ => None,
                        })
                        .unwrap_or("")
                }
            };

            // Append filter hint as suffix on the same line
            let line = if let Some(hint_msg) = state.filter_hint_for(node, conn_name_for_hint) {
                let mut spans = line.spans;
                spans.push(Span::styled(
                    format!("  {hint_msg}"),
                    Style::default()
                        .fg(theme.dim)
                        .bg(row_bg)
                        .add_modifier(Modifier::ITALIC),
                ));
                Line::from(spans)
            } else {
                line
            };

            ListItem::new(line)
        })
        .collect();

    let selected_in_view = if cursor >= offset && cursor < offset + inner_height {
        Some(cursor - offset)
    } else {
        None
    };

    let mut list_state = ListState::default();
    list_state.select(selected_in_view);

    // No highlight_style on List - we handle it manually per-span above
    let list = List::new(items).block(block).highlight_symbol("▸ ");

    frame.render_stateful_widget(list, tree_area, &mut list_state);

    if state.group_creating {
        if let Some(rect) = search_area {
            let line = Line::from(vec![
                Span::styled(
                    "New group: ",
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(state.group_rename_buf.as_str()),
                Span::styled("█", Style::default().fg(theme.accent)),
            ]);
            let bar = Paragraph::new(line).style(Style::default().bg(theme.status_bg));
            frame.render_widget(bar, rect);
        }
        return;
    }

    if let Some(search_rect) = search_area {
        let query = &state.tree_state.search_query;
        let match_count = state.tree_state.search_matches.len();
        let match_info = if query.is_empty() {
            String::new()
        } else {
            format!(" ({match_count} matches)")
        };

        let line = Line::from(vec![
            Span::styled(
                "/",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(query.as_str()),
            Span::styled("█", Style::default().fg(theme.accent)),
            Span::styled(match_info, Style::default().fg(theme.dim)),
        ]);
        let bar = Paragraph::new(line).style(Style::default().bg(theme.status_bg));
        frame.render_widget(bar, search_rect);
    }
}
