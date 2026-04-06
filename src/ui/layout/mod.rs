mod overlays;
mod tabs;
use overlays::*;
use tabs::*;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::core::virtual_fs::SyncState;
use crate::ui::state::{AppState, Focus, Mode, Overlay};
use crate::ui::tabs::{SubView, WorkspaceTab};
use crate::ui::theme::Theme;
use crate::ui::widgets;

const SIDEBAR_MIN_WIDTH: u16 = 22;

/// Write explicit space characters to every cell in the area.
/// Prevents ghosting: ratatui's diff always has real content to compare.
fn fill_bg(frame: &mut Frame, area: Rect, style: Style) {
    let fill = " ".repeat(area.width as usize);
    let lines: Vec<Line> = (0..area.height)
        .map(|_| Line::from(Span::styled(fill.clone(), style)))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

pub fn render(frame: &mut Frame, state: &mut AppState, theme: &Theme) {
    let area = frame.area();

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
        .constraints([Constraint::Length(sidebar_width), Constraint::Min(20)])
        .split(root[1]);

    // Split sidebar area: 2/3 explorer + 1/3 scripts
    let sidebar_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
        .split(main[0]);

    widgets::sidebar::render(frame, &mut *state, theme, sidebar_split[0]);
    render_scripts_panel(frame, state, theme, sidebar_split[1]);
    render_center(frame, state, theme, main[1]);

    widgets::statusbar::render(frame, state, theme, root[2]);

    // Render overlays on top
    match &state.overlay {
        Some(Overlay::ConnectionDialog) => {
            widgets::connection_dialog::render(
                frame,
                &state.dialogs.connection_form,
                &state.dialogs.saved_connections,
                theme,
            );
        }
        Some(Overlay::Help) => {
            widgets::help::render(frame, theme);
        }
        Some(Overlay::ConnectionMenu) => {
            widgets::conn_menu::render(frame, &state.dialogs.conn_menu, theme);
        }
        Some(Overlay::GroupMenu) => {
            widgets::group_menu::render(frame, &state.dialogs.group_menu, theme);
        }
        Some(Overlay::ObjectFilter) => {
            widgets::schema_filter::render(frame, &mut state.sidebar.object_filter, theme);
        }
        Some(Overlay::ConfirmDeleteConnection { name }) => {
            render_confirm_delete_connection(frame, theme, area, name);
        }
        Some(Overlay::ConfirmClose) => {
            render_confirm_close(frame, theme, area);
        }
        Some(Overlay::ConfirmQuit) => {
            render_confirm_quit(frame, state, theme, area);
        }
        Some(Overlay::SaveScriptName) => {
            render_save_script_name(frame, state, theme, area);
        }
        Some(Overlay::ScriptConnection) => {
            render_script_conn_picker(frame, state, theme, area);
        }
        Some(Overlay::ThemePicker) => {
            render_theme_picker(frame, state, theme, area);
        }
        Some(Overlay::BindVariables) => {
            render_bind_variables(frame, state, theme, area);
        }
        Some(Overlay::SaveGridChanges) => {
            render_save_grid_confirm(frame, state, theme, area);
        }
        Some(Overlay::ConfirmDropObject) => {
            render_confirm_drop(frame, state, theme, area);
        }
        Some(Overlay::RenameObject) => {
            render_rename_object(frame, state, theme, area);
        }
        Some(Overlay::ConfirmCompile) => {
            render_confirm_compile(frame, state, theme, area);
        }
        Some(Overlay::ExportDialog) => {
            render_export_dialog(frame, state, theme, area);
        }
        Some(Overlay::ImportDialog) => {
            render_import_dialog(frame, state, theme, area);
        }
        _ => {}
    }

    // Leader help hint (non-blocking, bottom-right corner)
    if state.leader.help_visible {
        let level = if state.leader.b_pending {
            2
        } else if state.leader.leader_pending {
            3
        } else if state.leader.w_pending {
            4
        } else if state.leader.s_pending {
            5
        } else {
            1
        };
        render_leader_help(frame, theme, area, level);
    }
}

/// Render the leader help popup. `level`: 1=root, 2=after b, 3=after <leader>
fn render_scripts_panel(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    use crate::ui::state::{ScriptNode, ScriptsMode};

    let is_focused = state.focus == Focus::ScriptsPanel;
    let border_style = theme.border_style(is_focused, &state.mode);

    let script_count = state
        .scripts.tree
        .iter()
        .filter(|n| matches!(n, ScriptNode::Script { .. }))
        .count();

    // Show mode hint in title
    let mode_hint = match &state.scripts.mode {
        ScriptsMode::PendingD => " [d]",
        ScriptsMode::PendingY => " [y]",
        _ => "",
    };
    let title = format!(" Scripts ({script_count}){mode_hint} ");

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(theme.editor_bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let visible_height = inner.height as usize;

    let visible: Vec<(usize, ScriptNode)> = state
        .scripts.visible_scripts()
        .into_iter()
        .map(|(i, n)| (i, n.clone()))
        .collect();

    if state.scripts.cursor < state.scripts.offset {
        state.scripts.offset = state.scripts.cursor;
    }
    if state.scripts.cursor >= state.scripts.offset + visible_height {
        state.scripts.offset = state.scripts.cursor - visible_height + 1;
    }

    if state.scripts.tree.is_empty() && !matches!(state.scripts.mode, ScriptsMode::Insert { .. }) {
        let lines = vec![
            Line::from(Span::styled(
                "  (no scripts)",
                Style::default().fg(theme.dim),
            )),
            Line::from(Span::styled(
                "  press i to create",
                Style::default().fg(theme.dim),
            )),
        ];
        let content = Paragraph::new(lines);
        frame.render_widget(content, inner);
        return;
    }

    let inner_width = inner.width as usize;

    let mut lines: Vec<Line> = visible
        .iter()
        .enumerate()
        .skip(state.scripts.offset)
        .take(visible_height)
        .map(|(vi, (_tree_idx, node))| {
            let is_selected = vi == state.scripts.cursor && is_focused;

            // Check for confirm delete on this item
            if let ScriptsMode::ConfirmDelete { path } = &state.scripts.mode {
                let node_path = match node {
                    ScriptNode::Collection { name, .. } => name.as_str(),
                    ScriptNode::Script { file_path, .. } => file_path.as_str(),
                };
                if path == node_path {
                    let display = match node {
                        ScriptNode::Collection { name, .. } => format!("{name}/"),
                        ScriptNode::Script { name, .. } => name.clone(),
                    };
                    return Line::from(vec![
                        Span::styled(
                            format!("  Delete '{display}'? "),
                            Style::default()
                                .fg(theme.error_fg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            "y",
                            Style::default()
                                .fg(theme.conn_connected)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("/", Style::default().fg(theme.dim)),
                        Span::styled(
                            "n",
                            Style::default()
                                .fg(theme.error_fg)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]);
                }
            }

            // Check for rename on this item
            if let ScriptsMode::Rename { buf, original_path } = &state.scripts.mode {
                let node_path = match node {
                    ScriptNode::Collection { name, .. } => name.as_str(),
                    ScriptNode::Script { file_path, .. } => file_path.as_str(),
                };
                if original_path == node_path {
                    let indent = match node {
                        ScriptNode::Collection { .. } => "  ",
                        ScriptNode::Script { collection, .. } => {
                            if collection.is_some() {
                                "    "
                            } else {
                                "  "
                            }
                        }
                    };
                    return Line::from(Span::styled(
                        format!("{indent}{buf}█"),
                        Style::default()
                            .fg(theme.conn_connecting)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
            }

            match node {
                ScriptNode::Collection { name, expanded } => {
                    let icon = if *expanded { "▼" } else { "▶" };
                    let text = format!("  {icon} {name}/");
                    let style = if is_selected {
                        Style::default()
                            .bg(theme.tree_selected_bg)
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD)
                    };
                    let display_w = UnicodeWidthStr::width(text.as_str());
                    let padded = if display_w < inner_width {
                        format!("{}{}", text, " ".repeat(inner_width - display_w))
                    } else {
                        text
                    };
                    Line::from(Span::styled(padded, style))
                }
                ScriptNode::Script {
                    name, collection, ..
                } => {
                    let indent = if collection.is_some() { "    " } else { "  " };
                    let text = format!("{indent}{name}");
                    let style = if is_selected {
                        Style::default()
                            .bg(theme.tree_selected_bg)
                            .fg(theme.tree_selected_fg)
                    } else {
                        Style::default()
                    };
                    let display_w = UnicodeWidthStr::width(text.as_str());
                    let padded = if display_w < inner_width {
                        format!("{}{}", text, " ".repeat(inner_width - display_w))
                    } else {
                        text
                    };
                    Line::from(Span::styled(padded, style))
                }
            }
        })
        .collect();

    // Insert mode: show input line at the cursor position (inside current collection)
    if let ScriptsMode::Insert { buf } = &state.scripts.mode {
        let indent = match state.scripts.current_collection() {
            Some(_) => "    ",
            None => "  ",
        };
        let input_line = Line::from(Span::styled(
            format!("{indent}> {buf}█"),
            Style::default()
                .fg(theme.conn_connecting)
                .add_modifier(Modifier::BOLD),
        ));
        // Insert after the current cursor position within the visible lines
        let insert_pos = if state.scripts.cursor >= state.scripts.offset {
            (state.scripts.cursor - state.scripts.offset + 1).min(lines.len())
        } else {
            0
        };
        lines.insert(insert_pos, input_line);
    }

    // Show yank indicator
    if state.scripts.yank.is_some() {
        let remaining = visible_height.saturating_sub(lines.len());
        if remaining > 0 {
            lines.push(Line::from(Span::styled(
                "  [yanked — p to paste]",
                Style::default().fg(theme.dim),
            )));
        }
    }

    let content = Paragraph::new(lines);
    frame.render_widget(content, inner);
}

fn render_topbar(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    let (conn_icon, conn_style) = theme.connection_indicator(state.conn.connected);
    let conn_name = state.conn.name.as_deref().unwrap_or("not connected");
    let db_label = state
        .conn.db_type
        .as_ref()
        .map(|t| t.to_string())
        .unwrap_or_default();
    let schema = state.conn.current_schema.as_deref().unwrap_or("");

    let status_text = if state.conn.connected {
        "CONNECTED"
    } else {
        "DISCONNECTED"
    };

    let sep = Span::styled(" \u{2502} ", Style::default().fg(theme.separator));

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
        Span::styled(&db_label, Style::default().fg(theme.accent)),
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
            if state.conn.connected {
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
    fill_bg(frame, area, Style::default().bg(theme.editor_bg));

    if state.tabs.is_empty() {
        render_empty_workspace(frame, theme, area);
        return;
    }

    // Tab bar (1 line) + optional sub-view bar (1 line) + content
    let has_sub_views = state
        .active_tab()
        .map(|t| !t.available_sub_views().is_empty())
        .unwrap_or(false);

    let constraints = if has_sub_views {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
        ]
    } else {
        vec![Constraint::Length(1), Constraint::Min(3)]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    render_tab_bar(frame, state, theme, chunks[0]);

    let content_area = if has_sub_views {
        render_sub_view_bar(frame, state, theme, chunks[1]);
        render_tab_content(frame, state, theme, chunks[2]);
        chunks[2]
    } else {
        render_tab_content(frame, state, theme, chunks[1]);
        chunks[1]
    };

    // Render diagnostic underlines on the editor (skip for PL/SQL tabs)
    if !state.engine.diagnostics.is_empty() {
        let is_plsql = state.active_tab().is_some_and(|t| {
            matches!(
                t.kind,
                crate::ui::tabs::TabKind::Package { .. }
                    | crate::ui::tabs::TabKind::Function { .. }
                    | crate::ui::tabs::TabKind::Procedure { .. }
                    | crate::ui::tabs::TabKind::DbType { .. }
                    | crate::ui::tabs::TabKind::Trigger { .. }
            )
        });
        if !is_plsql {
            render_diagnostic_underlines(frame, state, theme, content_area);
        }
    }

    // Render completion popup on top of everything
    if state.engine.completion.is_some() {
        render_completion_popup(frame, state, theme, content_area);
    }
}

fn render_empty_workspace(frame: &mut Frame, theme: &Theme, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_unfocused))
        .style(Style::default().bg(theme.editor_bg));
    let text = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            "  No tabs open",
            Style::default().fg(theme.dim),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  i  - New script (in scripts panel)",
            Style::default().fg(theme.dim),
        )),
        Line::from(Span::styled(
            "  l  - Open selected object",
            Style::default().fg(theme.dim),
        )),
        Line::from(Span::styled(
            "  a  - Add connection",
            Style::default().fg(theme.dim),
        )),
        Line::from(Span::styled("  ?  - Help", Style::default().fg(theme.dim))),
    ])
    .block(block);
    frame.render_widget(text, area);
}

// ---------------------------------------------------------------------------
// Export / Import dialog rendering
// ---------------------------------------------------------------------------
