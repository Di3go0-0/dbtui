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

    // group_creating no longer reserves a bottom search bar — the input is
    // rendered inline in the tree itself (oil-style).
    let (tree_area, search_area) = if state.sidebar.tree_state.search_active {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let block = Block::default()
        .title(" Explorer ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(tree_area);
    frame.render_widget(block, tree_area);

    render_tree_items(frame, state, theme, inner);

    if let Some(search_rect) = search_area {
        let query = &state.sidebar.tree_state.search_query;
        let match_count = state.sidebar.tree_state.search_matches.len();
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

/// Public entry point for oil navigator: renders the tree into any area without block/search bar.
pub fn render_tree(
    frame: &mut Frame,
    state: &mut AppState,
    theme: &Theme,
    area: Rect,
    _is_focused: bool,
) {
    render_tree_items(frame, state, theme, area);
}

/// Shared tree rendering logic used by both the sidebar and the oil navigator.
fn render_tree_items(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    let inner_height = area.height as usize;
    state.sidebar.tree_state.visible_height = inner_height.max(1);

    let visible = state.visible_tree();
    let offset = state.sidebar.tree_state.offset;
    let cursor = state.sidebar.tree_state.cursor;

    let mut items: Vec<ListItem> = visible
        .iter()
        .enumerate()
        .skip(offset)
        .take(inner_height)
        .map(|(vis_idx, (_, node, conn_name))| {
            let depth = node.depth();
            const MAX_INDENT: &str = "                                ";
            let indent = &MAX_INDENT[..depth.min(16) * 2];
            let is_selected = vis_idx == cursor;
            let is_search_match = state.sidebar.tree_state.search_active
                && state.sidebar.tree_state.search_matches.contains(&vis_idx);

            let row_bg = if is_selected {
                theme.tree_selected_bg
            } else if is_search_match {
                Color::Rgb(60, 50, 20)
            } else {
                Color::Reset
            };

            let line = match node {
                TreeNode::Group { expanded, name, .. } => {
                    if state
                        .dialogs
                        .group_renaming
                        .as_ref()
                        .is_some_and(|rn| rn == name)
                    {
                        let rename_line = format!("{indent}■ {}█", state.dialogs.group_rename_buf);
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
                            Span::styled(indent, Style::default().bg(row_bg)),
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
                            Span::styled("■ ", Style::default().fg(theme.accent).bg(row_bg)),
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
                    // Inline rename mode for connections (oil-style)
                    if state
                        .dialogs
                        .conn_renaming
                        .as_ref()
                        .is_some_and(|rn| rn == name)
                    {
                        let rename_line = format!("{indent}● {}█", state.dialogs.conn_rename_buf);
                        Line::from(Span::styled(
                            rename_line,
                            Style::default()
                                .fg(theme.conn_connecting)
                                .bg(row_bg)
                                .add_modifier(Modifier::BOLD),
                        ))
                    } else {
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
                            Span::styled(indent, Style::default().bg(row_bg)),
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
                }
                TreeNode::Schema { expanded, name, .. } => {
                    let icon = if *expanded { "▼ " } else { "▶ " };
                    let is_own_schema = state
                        .engine
                        .metadata_indexes
                        .get(*conn_name)
                        .and_then(|idx| idx.current_schema())
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
                        Span::styled(indent, Style::default().bg(row_bg)),
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
                        Span::styled(indent, Style::default().bg(row_bg)),
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
                    name,
                    kind,
                    valid,
                    privilege,
                    schema,
                    ..
                } => {
                    let (icon, base_color) = match kind {
                        LeafKind::Table => ("T ", theme.tree_table),
                        LeafKind::View => ("V ", theme.tree_view),
                        LeafKind::MaterializedView => ("M ", theme.tree_view),
                        LeafKind::Index => ("I ", theme.tree_table),
                        LeafKind::Sequence => ("S ", theme.tree_function),
                        LeafKind::Type => ("⊤ ", theme.tree_package),
                        LeafKind::Trigger => ("⚡", theme.tree_procedure),
                        LeafKind::Package => ("P ", theme.tree_package),
                        LeafKind::Procedure => ("ƒ ", theme.tree_procedure),
                        LeafKind::Function => ("λ ", theme.tree_function),
                        LeafKind::Event => ("E ", theme.tree_function),
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
                        .engine
                        .metadata_indexes
                        .get(*conn_name)
                        .and_then(|idx| idx.current_schema())
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
                        Span::styled(indent, Style::default().bg(row_bg)),
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
                TreeNode::Empty => Line::from(vec![
                    Span::styled(indent, Style::default().bg(row_bg)),
                    Span::styled(
                        "(empty)",
                        Style::default()
                            .fg(theme.dim)
                            .bg(row_bg)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]),
            };

            let conn_name_for_hint = conn_name;

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

    // Inline "create new group" input — rendered at the bottom of the visible
    // tree, not as a bottom search bar, so it feels like creating a script.
    if state.dialogs.group_creating {
        let input_line = Line::from(vec![
            Span::styled(
                "■ ",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                state.dialogs.group_rename_buf.as_str(),
                Style::default()
                    .fg(theme.conn_connecting)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("█", Style::default().fg(theme.accent)),
        ]);
        items.push(ListItem::new(input_line));
    }

    let selected_in_view = if cursor >= offset && cursor < offset + inner_height {
        Some(cursor - offset)
    } else {
        None
    };

    let mut list_state = ListState::default();
    list_state.select(selected_in_view);

    let list = List::new(items).highlight_symbol("▸ ");
    frame.render_stateful_widget(list, area, &mut list_state);
}
