use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
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
        Some(Overlay::GroupMenu) => {
            widgets::group_menu::render(frame, &state.group_menu, theme);
        }
        Some(Overlay::ObjectFilter) => {
            widgets::schema_filter::render(frame, &mut state.object_filter, theme);
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
        _ => {}
    }

    // Leader help hint (non-blocking, bottom-right corner)
    if state.leader_help_visible {
        let level = if state.leader_b_pending {
            2
        } else if state.leader_leader_pending {
            3
        } else if state.leader_w_pending {
            4
        } else if state.leader_s_pending {
            5
        } else {
            1
        };
        render_leader_help(frame, theme, area, level);
    }
}

/// Render the leader help popup. `level`: 1=root, 2=after b, 3=after <leader>
fn render_leader_help(frame: &mut Frame, theme: &Theme, area: Rect, level: usize) {
    use ratatui::style::Color;

    let key_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(theme.status_fg);
    let header_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);

    let (title, entries) = match level {
        2 => ("Leader > b", vec![("d", "close buffer")]),
        3 => ("Leader > Leader", vec![("s", "compile to DB")]),
        4 => ("Leader > w", vec![("d", "close result tab")]),
        5 => ("Leader > s", vec![("s", "SELECT template")]),
        _ => (
            "Leader (Space)",
            vec![
                ("Enter", "execute query"),
                ("/", "execute → new tab"),
                ("c", "connection"),
                ("t", "theme"),
                ("s", "+snippets..."),
                ("b", "+buffer..."),
                ("w", "+result..."),
                ("Spc", "+compile..."),
            ],
        ),
    };

    let mut lines = vec![
        Line::from(Span::styled(format!(" {title}"), header_style)),
        Line::from(""),
    ];
    for (key, desc) in &entries {
        lines.push(Line::from(vec![
            Span::styled(format!("  {key:<8}  "), key_style),
            Span::styled(*desc, desc_style),
        ]));
    }

    let height = (lines.len() as u16 + 2).min(area.height);
    let width = 28_u16.min(area.width);
    let x = area.width.saturating_sub(width + 1);
    let y = area.height.saturating_sub(height + 2);
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(ratatui::widgets::Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused))
        .style(Style::default().bg(Color::Rgb(25, 25, 35)));

    let content = Paragraph::new(lines).block(block);
    frame.render_widget(content, popup);
}

fn render_script_conn_picker(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    use crate::ui::state::PickerItem;

    let picker = match &state.script_conn_picker {
        Some(p) => p,
        None => return,
    };

    let visible = picker.visible_items();
    let count = visible.len();
    let height = (count as u16 + 2).min(14).min(area.height);
    let width = 38_u16.min(area.width);
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(ratatui::widgets::Clear, popup);

    let block = Block::default()
        .title(" Select Connection ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.dialog_bg));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let items: Vec<ratatui::widgets::ListItem> = visible
        .iter()
        .map(|item| match item {
            PickerItem::Active(name) => ratatui::widgets::ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled("● ", Style::default().fg(theme.conn_connected)),
                Span::styled(name.as_str(), Style::default().fg(theme.topbar_fg)),
            ])),
            PickerItem::OthersHeader => {
                let arrow = if picker.others_expanded { "▼" } else { "▶" };
                ratatui::widgets::ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("{arrow} Others"),
                        Style::default()
                            .fg(theme.dim)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]))
            }
            PickerItem::Other(name) => ratatui::widgets::ListItem::new(Line::from(vec![
                Span::raw("    "),
                Span::styled("○ ", Style::default().fg(theme.dim)),
                Span::styled(name.as_str(), Style::default().fg(theme.dim)),
            ])),
        })
        .collect();

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(picker.cursor));

    let list = ratatui::widgets::List::new(items)
        .highlight_style(
            Style::default()
                .bg(theme.tree_selected_bg)
                .fg(theme.tree_selected_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, inner, &mut list_state);
}

fn render_scripts_panel(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    let is_focused = state.focus == Focus::ScriptsPanel;
    let border_style = theme.border_style(is_focused, &state.mode);

    let count = state.scripts_list.len();
    let title = format!(" Scripts ({count}) ");

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

    // Adjust offset to keep cursor visible
    if state.scripts_cursor < state.scripts_offset {
        state.scripts_offset = state.scripts_cursor;
    }
    if state.scripts_cursor >= state.scripts_offset + visible_height {
        state.scripts_offset = state.scripts_cursor - visible_height + 1;
    }

    if state.scripts_list.is_empty() {
        let lines = vec![
            Line::from(Span::styled(
                "  (no scripts)",
                Style::default().fg(theme.dim),
            )),
            Line::from(Span::styled(
                "  press n to create",
                Style::default().fg(theme.dim),
            )),
        ];
        let content = Paragraph::new(lines);
        frame.render_widget(content, inner);
        return;
    }

    let lines: Vec<Line> = state
        .scripts_list
        .iter()
        .enumerate()
        .skip(state.scripts_offset)
        .take(visible_height)
        .map(|(i, name)| {
            let is_selected = i == state.scripts_cursor && is_focused;
            let display = name.strip_suffix(".sql").unwrap_or(name);

            // Check if confirming delete for this item
            if let Some(ref deleting) = state.scripts_confirm_delete
                && deleting == name
            {
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

            // Check if renaming this item
            if let Some(ref renaming) = state.scripts_renaming
                && renaming == name
            {
                let rename_line = format!("  S  {}█", state.scripts_rename_buf);
                return Line::from(Span::styled(
                    rename_line,
                    Style::default()
                        .fg(theme.conn_connecting)
                        .add_modifier(Modifier::BOLD),
                ));
            }

            let style = if is_selected {
                Style::default()
                    .bg(theme.tree_selected_bg)
                    .fg(theme.tree_selected_fg)
            } else {
                Style::default()
            };
            let text = format!("  S  {display}");
            let display_w = UnicodeWidthStr::width(text.as_str());
            let inner_width = inner.width as usize;
            let padded = if display_w < inner_width {
                format!("{}{}", text, " ".repeat(inner_width - display_w))
            } else {
                text
            };
            Line::from(Span::styled(padded, style))
        })
        .collect();

    let content = Paragraph::new(lines);
    frame.render_widget(content, inner);
}

fn render_loading(frame: &mut Frame, theme: &Theme, area: Rect, title: &str) {
    let border_style = Style::default().fg(theme.border_unfocused);
    let block = Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(theme.editor_bg));

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Loading...",
            Style::default()
                .fg(theme.conn_connecting)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    let content = Paragraph::new(lines).block(block);
    frame.render_widget(content, area);
}

fn render_confirm_close(frame: &mut Frame, theme: &Theme, area: Rect) {
    let width = 44_u16;
    let height = 5_u16;
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width.min(area.width), height.min(area.height));

    let block = Block::default()
        .title(" Unsaved Changes ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.conn_connecting))
        .style(Style::default().bg(theme.dialog_bg));

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Save before closing? "),
            Span::styled(
                "y",
                Style::default()
                    .fg(theme.conn_connected)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("/"),
            Span::styled(
                "n",
                Style::default()
                    .fg(theme.error_fg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("/"),
            Span::styled("Esc", Style::default().fg(theme.dim)),
        ]),
    ];

    // Clear area behind popup
    let clear = Paragraph::new("").style(Style::default().bg(theme.dialog_bg));
    frame.render_widget(clear, popup);

    let content = Paragraph::new(text).block(block);
    frame.render_widget(content, popup);
}

fn render_confirm_quit(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    // Collect unsaved tab names
    let unsaved: Vec<String> = state
        .tabs
        .iter()
        .filter(|t| {
            t.editor.as_ref().is_some_and(|e| e.modified)
                || t.body_editor.as_ref().is_some_and(|e| e.modified)
                || t.decl_editor.as_ref().is_some_and(|e| e.modified)
        })
        .map(|t| {
            format!(
                "  {} {} ({})",
                t.kind.icon(),
                t.kind.display_name(),
                t.kind.kind_label()
            )
        })
        .collect();

    let list_height = unsaved.len() as u16;
    let width = 50_u16;
    // 3 = border top + empty line before list; 2 = empty line + hint line; 1 = border bottom
    let height = 3 + list_height + 2 + 1;
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width.min(area.width), height.min(area.height));

    let block = Block::default()
        .title(" Unsaved Changes ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.conn_connecting))
        .style(Style::default().bg(theme.dialog_bg));

    let mut lines = vec![Line::from("")];

    for name in &unsaved {
        lines.push(Line::from(Span::styled(
            name.as_str(),
            Style::default().fg(theme.error_fg),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  Quit anyway? "),
        Span::styled(
            "y",
            Style::default()
                .fg(theme.conn_connected)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("/"),
        Span::styled(
            "n",
            Style::default()
                .fg(theme.error_fg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("/"),
        Span::styled("Esc", Style::default().fg(theme.dim)),
    ]));

    let clear = Paragraph::new("").style(Style::default().bg(theme.dialog_bg));
    frame.render_widget(clear, popup);

    let content = Paragraph::new(lines).block(block);
    frame.render_widget(content, popup);
}

fn render_save_script_name(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    let width = 44_u16;
    let height = 5_u16;
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width.min(area.width), height.min(area.height));

    let block = Block::default()
        .title(" Save Script As ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.conn_connecting))
        .style(Style::default().bg(theme.dialog_bg));

    let name_buf = state.scripts_save_name.as_deref().unwrap_or("");

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Name: "),
            Span::styled(
                format!("{name_buf}█"),
                Style::default()
                    .fg(theme.conn_connecting)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    let clear = Paragraph::new("").style(Style::default().bg(theme.dialog_bg));
    frame.render_widget(clear, popup);

    let content = Paragraph::new(text).block(block);
    frame.render_widget(content, popup);
}

fn render_topbar(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    let (conn_icon, conn_style) = theme.connection_indicator(state.connected);
    let conn_name = state.connection_name.as_deref().unwrap_or("not connected");
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

    // Render completion popup on top of the editor area
    if state.completion.is_some() {
        render_completion_popup(frame, state, theme, content_area);
    }

    // Render diagnostic underlines on the editor
    if !state.diagnostics.is_empty() {
        render_diagnostic_underlines(frame, state, theme, content_area);
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
            "  n  - New script",
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

fn render_tab_bar(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    let mut spans: Vec<Span> = Vec::new();

    for (idx, tab) in state.tabs.iter().enumerate() {
        let is_active = idx == state.active_tab_idx;
        let icon = tab.kind.icon();
        let name = tab.kind.display_name();
        let conn = tab.kind.conn_name();

        // Check editor modified state
        let is_modified = tab.editor.as_ref().map(|e| e.modified).unwrap_or(false)
            || tab
                .body_editor
                .as_ref()
                .map(|e| e.modified)
                .unwrap_or(false)
            || tab
                .decl_editor
                .as_ref()
                .map(|e| e.modified)
                .unwrap_or(false);

        // Build label based on sync state (VFS) or editor modified state
        let (label, style_override) = match &tab.sync_state {
            Some(SyncState::Dirty) => (format!(" {icon} {name}(*) "), None),
            Some(SyncState::LocalSaved) => {
                (
                    format!(" {icon} {name}(!) "),
                    Some(
                        Style::default()
                            .fg(theme.conn_connecting) // yellow
                            .add_modifier(Modifier::BOLD),
                    ),
                )
            }
            Some(SyncState::ValidationError(_)) => {
                (
                    format!(" {icon} {name}(\u{2717}) "),
                    Some(
                        Style::default()
                            .fg(theme.error_fg) // red
                            .add_modifier(Modifier::BOLD),
                    ),
                )
            }
            Some(SyncState::Clean) => (format!(" {icon} {name} "), None),
            None => {
                // No VFS state (scripts, tables): use editor modified flag
                if is_modified {
                    (format!(" {icon} {name}(*) "), None)
                } else {
                    (format!(" {icon} {name} "), None)
                }
            }
        };

        let tab_style = style_override.unwrap_or_else(|| theme.tab_style(is_active));

        spans.push(Span::raw(" "));
        spans.push(Span::styled(label, tab_style));
        // Show connection name on active tab
        if is_active && let Some(cn) = conn {
            spans.push(Span::styled(
                format!("[{cn}]"),
                Style::default().fg(theme.dim),
            ));
        }
        spans.push(Span::styled(
            "\u{2502}",
            Style::default().fg(theme.separator),
        ));
    }

    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(Style::default().bg(theme.status_bg));
    frame.render_widget(bar, area);
}

fn render_sub_view_bar(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    let mut spans: Vec<Span> = Vec::new();

    if let Some(tab) = state.active_tab() {
        let views = tab.available_sub_views();
        for sv in &views {
            let is_active = tab.active_sub_view.as_ref() == Some(sv);
            let label = format!(" {} ", sv.label());

            spans.push(Span::raw(" "));
            if is_active {
                spans.push(Span::styled(
                    label,
                    Style::default()
                        .fg(theme.tab_active_fg)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    label,
                    Style::default().fg(theme.tab_inactive_fg),
                ));
            }
            spans.push(Span::styled(
                "\u{2502}",
                Style::default().fg(theme.separator),
            ));
        }
    }

    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(Style::default().bg(theme.status_bg));
    frame.render_widget(bar, area);
}

fn render_tab_content(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return;
    }

    let focused = state.focus == Focus::TabContent;
    let mode = state.mode.clone();

    let sub_view = state.tabs[tab_idx].active_sub_view.clone();

    match sub_view {
        Some(SubView::TableData) => {
            let tab = &mut state.tabs[tab_idx];
            widgets::data_grid::render_for_tab(frame, tab, focused, theme, area, &mode);
        }
        Some(SubView::TableProperties) => {
            let tab = &state.tabs[tab_idx];
            widgets::properties::render_for_tab(frame, tab, focused, theme, area, &mode);
        }
        Some(SubView::TableDDL) => {
            let tab = &mut state.tabs[tab_idx];
            if let Some(editor) = tab.ddl_editor.as_mut() {
                vimltui::render::render(
                    frame,
                    editor,
                    focused,
                    &theme.vim_theme(),
                    &crate::ui::sql_highlighter::SqlHighlighter::from_theme(theme),
                    area,
                    "DDL",
                );
            } else {
                render_loading(frame, theme, area, "DDL");
            }
        }
        Some(SubView::PackageDeclaration) => {
            let tab = &mut state.tabs[tab_idx];
            if let Some(editor) = tab.decl_editor.as_mut() {
                if editor.lines.len() == 1 && editor.lines[0].is_empty() && state.loading {
                    render_loading(frame, theme, area, "Declaration");
                } else {
                    vimltui::render::render(
                        frame,
                        editor,
                        focused,
                        &theme.vim_theme(),
                        &crate::ui::sql_highlighter::SqlHighlighter::from_theme(theme),
                        area,
                        "Declaration",
                    );
                }
            } else {
                render_loading(frame, theme, area, "Declaration");
            }
        }
        Some(SubView::PackageBody) => {
            let tab = &mut state.tabs[tab_idx];
            if let Some(editor) = tab.body_editor.as_mut() {
                if editor.lines.len() == 1 && editor.lines[0].is_empty() && state.loading {
                    render_loading(frame, theme, area, "Body");
                } else {
                    vimltui::render::render(
                        frame,
                        editor,
                        focused,
                        &theme.vim_theme(),
                        &crate::ui::sql_highlighter::SqlHighlighter::from_theme(theme),
                        area,
                        "Body",
                    );
                }
            } else {
                render_loading(frame, theme, area, "Body");
            }
        }
        Some(SubView::PackageFunctions) => {
            render_package_list(frame, state, theme, area, focused, true);
        }
        Some(SubView::PackageProcedures) => {
            render_package_list(frame, state, theme, area, focused, false);
        }
        None => {
            // Script / Function / Procedure
            let tab = &mut state.tabs[tab_idx];
            let title = tab.kind.display_name().to_string();
            let is_source = matches!(
                tab.kind,
                crate::ui::tabs::TabKind::Function { .. }
                    | crate::ui::tabs::TabKind::Procedure { .. }
            );
            let has_results = tab.query_result.is_some();
            let has_result_tabs = !tab.result_tabs.is_empty();

            if has_results || has_result_tabs {
                render_script_with_results(frame, tab, focused, theme, area, &mode, &title);
            } else if let Some(editor) = tab.editor.as_mut() {
                if is_source
                    && editor.lines.len() == 1
                    && editor.lines[0].is_empty()
                    && state.loading
                {
                    render_loading(frame, theme, area, &title);
                } else {
                    vimltui::render::render(
                        frame,
                        editor,
                        focused,
                        &theme.vim_theme(),
                        &crate::ui::sql_highlighter::SqlHighlighter::from_theme(theme),
                        area,
                        &title,
                    );
                }
            } else {
                render_loading(frame, theme, area, &title);
            }
        }
    }
}

/// Render the completion popup below the cursor.
fn render_completion_popup(frame: &mut Frame, state: &AppState, theme: &Theme, editor_area: Rect) {
    let cmp = match &state.completion {
        Some(c) if !c.items.is_empty() => c,
        _ => return,
    };

    let tab = match state.tabs.get(state.active_tab_idx) {
        Some(t) => t,
        None => return,
    };
    let editor = match tab.active_editor() {
        Some(e) => e,
        None => return,
    };

    // Calculate gutter width (same logic as vimltui render)
    let line_count_width = format!("{}", editor.lines.len()).len().max(3);
    let num_col_width = line_count_width + 2;

    // Cursor screen position relative to editor_area
    // editor_area includes the border (1px each side)
    let cursor_screen_row = editor.cursor_row.saturating_sub(editor.scroll_offset);
    let popup_x = editor_area.x + 1 + num_col_width as u16 + cmp.origin_col as u16;
    let popup_y = editor_area.y + 2 + cursor_screen_row as u16; // +2: border + line below cursor

    // Popup dimensions (max 4 visible items + "..." indicator)
    let max_visible = 4_u16;
    let item_count = cmp.items.len() as u16;
    let has_more = item_count > max_visible;
    let visible_rows = item_count.min(max_visible);
    let height = visible_rows + if has_more { 1 } else { 0 } + 2; // +2 for borders

    // Find max label width for sizing
    let max_label = cmp
        .items
        .iter()
        .map(|i| i.label.len() + i.kind.tag().len() + 3) // " label  tag "
        .max()
        .unwrap_or(10) as u16;
    let width = (max_label + 2).min(40); // +2 for borders

    // Clamp to screen bounds
    let x = popup_x.min(editor_area.right().saturating_sub(width));
    let available_below = editor_area.bottom().saturating_sub(popup_y);
    let (y, h) = if available_below >= height {
        (popup_y, height)
    } else {
        // Show above cursor if not enough space below
        let above_y = (editor_area.y + 1 + cursor_screen_row as u16).saturating_sub(height);
        (above_y, height)
    };

    let popup_rect = Rect::new(x, y, width, h);

    // Clear area behind popup
    frame.render_widget(ratatui::widgets::Clear, popup_rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused))
        .style(Style::default().bg(theme.dialog_bg));

    let inner = block.inner(popup_rect);
    frame.render_widget(block, popup_rect);

    // Scroll offset for long lists
    let visible_count = max_visible as usize;
    let scroll = if cmp.cursor >= visible_count {
        cmp.cursor - visible_count + 1
    } else {
        0
    };

    for (i, item) in cmp
        .items
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_count)
    {
        let row_y = inner.y + (i - scroll) as u16;
        let is_selected = i == cmp.cursor;

        let tag = item.kind.tag();
        let tag_width = tag.len();
        let label_max = inner.width as usize - tag_width - 2;
        let label = if item.label.len() > label_max {
            &item.label[..label_max]
        } else {
            &item.label
        };

        let padding = inner.width as usize - label.len() - tag_width - 1;

        let (bg, fg) = if is_selected {
            (theme.border_focused, theme.dialog_bg)
        } else {
            (theme.dialog_bg, theme.status_fg)
        };

        let tag_fg = if is_selected {
            theme.dialog_bg
        } else {
            theme.dim
        };

        let line = ratatui::text::Line::from(vec![
            Span::styled(
                format!(" {label}{:>pad$}", "", pad = padding),
                Style::default().fg(fg).bg(bg),
            ),
            Span::styled(format!("{tag} "), Style::default().fg(tag_fg).bg(bg)),
        ]);

        let row_rect = Rect::new(inner.x, row_y, inner.width, 1);
        frame.render_widget(Paragraph::new(line), row_rect);
    }

    // Show "..." indicator if there are more items below
    if has_more {
        let more_y = inner.y + visible_rows;
        if more_y < inner.y + inner.height {
            let remaining = cmp.items.len().saturating_sub(scroll + visible_count);
            let more_text = if remaining > 0 {
                format!(" ... +{remaining} more")
            } else {
                " ...".to_string()
            };
            let more_line = ratatui::text::Line::from(Span::styled(
                more_text,
                Style::default().fg(theme.dim).bg(theme.dialog_bg),
            ));
            let more_rect = Rect::new(inner.x, more_y, inner.width, 1);
            frame.render_widget(Paragraph::new(more_line), more_rect);
        }
    }
}

/// Render red underlines on diagnostic ranges within the editor area.
fn render_bind_variables(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    let bv = match &state.bind_variables {
        Some(b) => b,
        None => return,
    };

    let var_count = bv.variables.len();
    let width = 50_u16.min(area.width.saturating_sub(4));
    // 3 = border top + title line + border bottom, +1 per var, +1 for hint line
    let height = (3 + var_count as u16 + 2).min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    let block = Block::default()
        .title(" Bind Variables ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.conn_connecting))
        .style(Style::default().bg(theme.dialog_bg));

    frame.render_widget(ratatui::widgets::Clear, popup);

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // Render each variable
    for (i, (name, value)) in bv.variables.iter().enumerate() {
        if i as u16 >= inner.height.saturating_sub(1) {
            break;
        }
        let row_y = inner.y + i as u16;
        let is_selected = i == bv.selected_idx;

        let label = format!(":{name}");
        let display_val = if is_selected {
            format!("{value}\u{2588}") // █ cursor
        } else {
            value.clone()
        };

        let label_style = if is_selected {
            Style::default()
                .fg(theme.conn_connecting)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.dim)
        };

        let val_style = if is_selected {
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.status_fg)
        };

        let line = Line::from(vec![
            Span::styled(format!("  {label:<16}"), label_style),
            Span::styled(display_val, val_style),
        ]);

        let row_rect = Rect::new(inner.x, row_y, inner.width, 1);
        frame.render_widget(Paragraph::new(line), row_rect);
    }

    // Hint line at bottom
    let hint_y = inner.y + inner.height.saturating_sub(1);
    let hint = Line::from(vec![
        Span::styled("  Tab", Style::default().fg(theme.accent)),
        Span::styled(" next  ", Style::default().fg(theme.dim)),
        Span::styled("Enter", Style::default().fg(theme.conn_connected)),
        Span::styled(" execute  ", Style::default().fg(theme.dim)),
        Span::styled("Esc", Style::default().fg(theme.error_fg)),
        Span::styled(" cancel", Style::default().fg(theme.dim)),
    ]);
    let hint_rect = Rect::new(inner.x, hint_y, inner.width, 1);
    frame.render_widget(Paragraph::new(hint), hint_rect);
}

fn render_diagnostic_underlines(
    frame: &mut Frame,
    state: &AppState,
    theme: &Theme,
    editor_area: Rect,
) {
    let tab = match state.tabs.get(state.active_tab_idx) {
        Some(t) => t,
        None => return,
    };
    let editor = match tab.active_editor() {
        Some(e) => e,
        None => return,
    };

    // If there are results (split view), editor only occupies top 60%
    let has_results = !tab.result_tabs.is_empty() || tab.query_result.is_some();
    let actual_editor_area = if has_results {
        Rect::new(
            editor_area.x,
            editor_area.y,
            editor_area.width,
            (editor_area.height * 60) / 100,
        )
    } else {
        editor_area
    };

    // Calculate gutter width (same as vimltui render)
    let line_count_width = format!("{}", editor.lines.len()).len().max(3);
    let num_col_width = (line_count_width + 2) as u16;

    // Inner area (inside borders)
    let inner_x = actual_editor_area.x + 1 + num_col_width;
    let inner_y = actual_editor_area.y + 1; // +1 for top border
    let inner_height = actual_editor_area.height.saturating_sub(3) as usize; // borders + command line

    for diag in &state.diagnostics {
        // Check if diagnostic line is visible
        if diag.row < editor.scroll_offset || diag.row >= editor.scroll_offset + inner_height {
            continue;
        }

        let screen_row = inner_y + (diag.row - editor.scroll_offset) as u16;
        let col_start = diag.col_start as u16;
        let col_len = (diag.col_end - diag.col_start).max(1) as u16;
        let screen_x = inner_x + col_start;

        // Don't render outside editor area
        if screen_x >= actual_editor_area.right()
            || screen_row >= actual_editor_area.bottom().saturating_sub(2)
        {
            continue;
        }

        let available = actual_editor_area.right().saturating_sub(screen_x);
        let width = col_len.min(available);

        let underline_rect = Rect::new(screen_x, screen_row, width, 1);

        // Get the original text to preserve it, just add underline style
        let Some(line) = editor.lines.get(diag.row) else {
            continue;
        };
        let start = diag.col_start.min(line.len());
        let end = diag.col_end.min(line.len());
        let text = if start < end { &line[start..end] } else { " " };

        let styled = Paragraph::new(Span::styled(
            text,
            Style::default()
                .fg(theme.error_fg)
                .add_modifier(Modifier::UNDERLINED),
        ));
        frame.render_widget(styled, underline_rect);
    }
}

/// Render the split view: editor (top 60%) + results/errors (bottom 40%).
/// Handles result tab bars, error panes with query views, and data grids.
fn render_script_with_results(
    frame: &mut Frame,
    tab: &mut WorkspaceTab,
    focused: bool,
    theme: &Theme,
    area: Rect,
    mode: &Mode,
    title: &str,
) {
    let has_result_tabs = !tab.result_tabs.is_empty();

    // Split: editor top (60%) + results bottom (40%)
    let splits = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let sf = tab.sub_focus;
    if let Some(editor) = tab.editor.as_mut() {
        let editor_focused = focused && sf == crate::ui::tabs::SubFocus::Editor;
        vimltui::render::render(
            frame,
            editor,
            editor_focused,
            &theme.vim_theme(),
            &crate::ui::sql_highlighter::SqlHighlighter::from_theme(theme),
            splits[0],
            title,
        );
    }

    if has_result_tabs {
        // Script: render result tab bar + active result
        let result_area = splits[1];
        let result_splits = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(3)])
            .split(result_area);

        // Result tab bar
        render_result_tab_bar(frame, tab, theme, result_splits[0]);

        // Active result tab content
        let idx = tab.active_result_idx;
        let is_error = idx < tab.result_tabs.len() && tab.result_tabs[idx].error_editor.is_some();

        if is_error {
            use ratatui::style::Color;
            let err_area = result_splits[1];
            let err_focused = focused && sf == crate::ui::tabs::SubFocus::Results;
            let q_focused = focused && sf == crate::ui::tabs::SubFocus::QueryView;

            // Red border: bright when focused, dim when not
            let red_bright = Color::Rgb(220, 80, 80);
            let red_dim = Color::Rgb(120, 50, 50);
            let err_border = if err_focused { red_bright } else { red_dim };
            let q_border = if q_focused { red_bright } else { red_dim };

            // Split error pane: error message (left) + query (right)
            let has_query = tab.result_tabs[idx].query_editor.is_some();
            if has_query {
                let err_splits = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(err_area);

                let vt = theme.vim_theme();
                let hl = crate::ui::sql_highlighter::SqlHighlighter::from_theme(theme);
                if let Some(err_editor) = tab.result_tabs[idx].error_editor.as_mut() {
                    vimltui::render::render_with_options(
                        frame,
                        err_editor,
                        err_focused,
                        &vt,
                        &hl,
                        err_splits[0],
                        "Error",
                        Some(err_border),
                    );
                }
                if let Some(q_editor) = tab.result_tabs[idx].query_editor.as_mut() {
                    vimltui::render::render_with_options(
                        frame,
                        q_editor,
                        q_focused,
                        &vt,
                        &hl,
                        err_splits[1],
                        "Query",
                        Some(q_border),
                    );
                }
            } else if let Some(err_editor) = tab.result_tabs[idx].error_editor.as_mut() {
                vimltui::render::render_with_options(
                    frame,
                    err_editor,
                    err_focused,
                    &theme.vim_theme(),
                    &crate::ui::sql_highlighter::SqlHighlighter::from_theme(theme),
                    err_area,
                    "Error",
                    Some(err_border),
                );
            }
        } else {
            if idx < tab.result_tabs.len() {
                let rt = &tab.result_tabs[idx];
                tab.query_result = Some(rt.result.clone());
                tab.grid_scroll_row = rt.scroll_row;
                tab.grid_selected_row = rt.selected_row;
                tab.grid_selected_col = rt.selected_col;
                tab.grid_visible_height = rt.visible_height;
                tab.grid_selection_anchor = rt.selection_anchor;
            }
            widgets::data_grid::render_for_tab(frame, tab, focused, theme, result_splits[1], mode);

            if idx < tab.result_tabs.len() {
                tab.result_tabs[idx].visible_height = tab.grid_visible_height;
            }
        }
    } else {
        widgets::data_grid::render_for_tab(frame, tab, focused, theme, splits[1], mode);
    }
}

fn render_theme_picker(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    use crate::ui::theme::THEME_NAMES;

    let count = THEME_NAMES.len();
    let height = (count as u16 + 2).min(area.height);
    let width = 30_u16.min(area.width);
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(ratatui::widgets::Clear, popup);

    let block = Block::default()
        .title(" Theme ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.dialog_bg));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let items: Vec<ratatui::widgets::ListItem> = THEME_NAMES
        .iter()
        .map(|name| {
            let is_current = theme.name == *name;
            let icon = if is_current { "● " } else { "  " };
            ratatui::widgets::ListItem::new(Line::from(vec![
                Span::styled(
                    icon,
                    Style::default().fg(if is_current {
                        theme.conn_connected
                    } else {
                        theme.dim
                    }),
                ),
                Span::styled(*name, Style::default().fg(theme.topbar_fg)),
            ]))
        })
        .collect();

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(state.theme_picker.cursor));

    let list = ratatui::widgets::List::new(items)
        .highlight_style(
            Style::default()
                .bg(theme.tree_selected_bg)
                .fg(theme.tree_selected_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, inner, &mut list_state);
}

fn render_result_tab_bar(
    frame: &mut Frame,
    tab: &crate::ui::tabs::WorkspaceTab,
    theme: &Theme,
    area: Rect,
) {
    let mut spans: Vec<Span> = Vec::new();
    for (idx, rt) in tab.result_tabs.iter().enumerate() {
        let is_active = idx == tab.active_result_idx;
        let time_str = rt
            .result
            .elapsed
            .map(|d| {
                let ms = d.as_millis();
                if ms < 1000 {
                    format!(" {ms}ms")
                } else {
                    format!(" {:.2}s", d.as_secs_f64())
                }
            })
            .unwrap_or_default();
        let label = format!(" {} ({}){time_str} ", rt.label, rt.result.rows.len());
        let style = if is_active {
            Style::default()
                .fg(theme.tab_active_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.tab_inactive_fg)
        };
        spans.push(Span::raw(" "));
        spans.push(Span::styled(label, style));
        spans.push(Span::styled(
            "\u{2502}",
            Style::default().fg(theme.separator),
        ));
    }
    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(Style::default().bg(theme.status_bg));
    frame.render_widget(bar, area);
}

fn render_package_list(
    frame: &mut Frame,
    state: &mut AppState,
    theme: &Theme,
    area: Rect,
    focused: bool,
    is_functions: bool,
) {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return;
    }
    let tab = &state.tabs[tab_idx];

    let title = if is_functions {
        " Functions "
    } else {
        " Procedures "
    };
    let items = if is_functions {
        &tab.package_functions
    } else {
        &tab.package_procedures
    };

    let border_style = theme.border_style(focused, &state.mode);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(theme.editor_bg));

    if items.is_empty() {
        let empty_msg = if is_functions {
            "(no functions)"
        } else {
            "(no procedures)"
        };
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  {empty_msg}"),
                Style::default().fg(theme.dim),
            )),
        ];
        let content = Paragraph::new(lines).block(block);
        frame.render_widget(content, area);
        return;
    }

    let visible_height = area.height.saturating_sub(2) as usize;
    let offset = if tab.package_list_cursor >= visible_height {
        tab.package_list_cursor - visible_height + 1
    } else {
        0
    };

    let inner_width = area.width.saturating_sub(2) as usize;

    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_height)
        .map(|(i, name)| {
            let icon = if is_functions { "λ" } else { "ƒ" };
            let style = if i == tab.package_list_cursor {
                Style::default()
                    .bg(theme.tree_selected_bg)
                    .fg(theme.tree_selected_fg)
            } else {
                Style::default()
            };
            let text = format!("  {icon}  {name}");
            let display_w = UnicodeWidthStr::width(text.as_str());
            let padded = if display_w < inner_width {
                format!("{}{}", text, " ".repeat(inner_width - display_w))
            } else {
                text
            };
            Line::from(Span::styled(padded, style))
        })
        .collect();

    let content = Paragraph::new(lines).block(block);
    frame.render_widget(content, area);
}
