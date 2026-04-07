use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::ui::state::{AppState, ExportField, ImportField};
use crate::ui::theme::Theme;

pub(super) fn render_confirm_delete_connection(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    conn_name: &str,
) {
    let width = 50_u16;
    let height = 5_u16;
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width.min(area.width), height.min(area.height));

    let block = Block::default()
        .title(" Delete Connection ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.error_fg))
        .style(Style::default().bg(theme.dialog_bg));

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Delete "),
            Span::styled(
                conn_name,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("? "),
            Span::styled(
                "y",
                Style::default()
                    .fg(theme.error_fg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("/"),
            Span::styled(
                "n",
                Style::default()
                    .fg(theme.conn_connected)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    frame.render_widget(ratatui::widgets::Clear, popup);
    let content = Paragraph::new(text).block(block);
    frame.render_widget(content, popup);
}

pub(super) fn render_confirm_close(frame: &mut Frame, theme: &Theme, area: Rect) {
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
    frame.render_widget(ratatui::widgets::Clear, popup);

    let content = Paragraph::new(text).block(block);
    frame.render_widget(content, popup);
}

pub(super) fn render_confirm_quit(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
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

    frame.render_widget(ratatui::widgets::Clear, popup);

    let content = Paragraph::new(lines).block(block);
    frame.render_widget(content, popup);
}

pub(super) fn render_save_grid_confirm(
    frame: &mut Frame,
    state: &AppState,
    theme: &Theme,
    area: Rect,
) {
    use crate::ui::tabs::RowChange;

    let tab = match state.active_tab() {
        Some(t) => t,
        None => return,
    };

    let modified = tab
        .grid_changes
        .values()
        .filter(|c| matches!(c, RowChange::Modified { .. }))
        .count();
    let new = tab
        .grid_changes
        .values()
        .filter(|c| matches!(c, RowChange::New { .. }))
        .count();
    let deleted = tab
        .grid_changes
        .values()
        .filter(|c| matches!(c, RowChange::Deleted))
        .count();

    let width = 44_u16;
    let height = 9_u16;
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width.min(area.width), height.min(area.height));

    let block = Block::default()
        .title(" Save Changes ")
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(theme.dialog_bg));

    let mut lines = vec![Line::from("")];
    if modified > 0 {
        lines.push(Line::from(Span::styled(
            format!("  {modified} modified row(s)"),
            Style::default().fg(Color::Yellow),
        )));
    }
    if new > 0 {
        lines.push(Line::from(Span::styled(
            format!("  {new} new row(s)"),
            Style::default().fg(Color::Green),
        )));
    }
    if deleted > 0 {
        lines.push(Line::from(Span::styled(
            format!("  {deleted} deleted row(s)"),
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  Save to database? "),
        Span::styled(
            "y",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("/"),
        Span::styled(
            "n",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    ]));

    frame.render_widget(ratatui::widgets::Clear, popup);

    let content = Paragraph::new(lines).block(block);
    frame.render_widget(content, popup);
}

pub(super) fn render_confirm_drop(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    let action = match &state.sidebar.pending_action {
        Some(a) => a,
        None => return,
    };

    let width = 48_u16;
    let height = 7_u16;
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width.min(area.width), height.min(area.height));

    let block = Block::default()
        .title(" Drop Object ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        .style(Style::default().bg(theme.dialog_bg));

    let obj_label = format!("  {} {}.{}", action.obj_type, action.schema, action.name);
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            obj_label,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("  Drop? "),
            Span::styled(
                "y",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw("/"),
            Span::styled(
                "n",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    frame.render_widget(ratatui::widgets::Clear, popup);

    let content = Paragraph::new(lines).block(block);
    frame.render_widget(content, popup);
}

pub(super) fn render_rename_object(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    let action = match &state.sidebar.pending_action {
        Some(a) => a,
        None => return,
    };

    let width = 50_u16;
    let height = 7_u16;
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width.min(area.width), height.min(area.height));

    let block = Block::default()
        .title(format!(" Rename {} ", action.obj_type))
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(theme.dialog_bg));

    let old_label = format!("  {}.{} \u{2192}", action.schema, action.name);
    let input_text = format!("  {}\u{2588}", state.sidebar.rename_buf);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(old_label, Style::default().fg(theme.dim))),
        Line::from(Span::styled(
            input_text,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    frame.render_widget(ratatui::widgets::Clear, popup);

    let content = Paragraph::new(lines).block(block);
    frame.render_widget(content, popup);
}

pub(super) fn render_confirm_compile(
    frame: &mut Frame,
    state: &AppState,
    theme: &Theme,
    area: Rect,
) {
    let tab = match state.active_tab() {
        Some(t) => t,
        None => return,
    };

    let obj_label = match &tab.kind {
        crate::ui::tabs::TabKind::Package { schema, name, .. } => {
            format!("  PACKAGE {schema}.{name}")
        }
        crate::ui::tabs::TabKind::Function { schema, name, .. } => {
            format!("  FUNCTION {schema}.{name}")
        }
        crate::ui::tabs::TabKind::Procedure { schema, name, .. } => {
            format!("  PROCEDURE {schema}.{name}")
        }
        _ => return,
    };

    let has_decl = tab.decl_editor.as_ref().is_some_and(|e| e.modified);
    let has_body = tab.body_editor.as_ref().is_some_and(|e| e.modified);
    let has_source = tab.editor.as_ref().is_some_and(|e| e.modified);

    let width = 48_u16;
    let height = 9_u16;
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width.min(area.width), height.min(area.height));

    let block = Block::default()
        .title(" Compile to Database ")
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(theme.dialog_bg));

    let mut lines = vec![Line::from("")];
    lines.push(Line::from(Span::styled(
        obj_label,
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    if has_decl {
        lines.push(Line::from(Span::styled(
            "  \u{270e} Declaration (modified)",
            Style::default().fg(Color::Yellow),
        )));
    }
    if has_body {
        lines.push(Line::from(Span::styled(
            "  \u{270e} Body (modified)",
            Style::default().fg(Color::Yellow),
        )));
    }
    if has_source {
        lines.push(Line::from(Span::styled(
            "  \u{270e} Source (modified)",
            Style::default().fg(Color::Yellow),
        )));
    }
    if !has_decl && !has_body && !has_source {
        lines.push(Line::from(Span::styled(
            "  (no changes detected)",
            Style::default().fg(theme.dim),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  Compile? "),
        Span::styled(
            "y",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("/"),
        Span::styled(
            "n",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    ]));

    frame.render_widget(ratatui::widgets::Clear, popup);
    let content = Paragraph::new(lines).block(block);
    frame.render_widget(content, popup);
}

pub(super) fn render_save_script_name(
    frame: &mut Frame,
    state: &AppState,
    theme: &Theme,
    area: Rect,
) {
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

    let name_buf = state.scripts.save_name.as_deref().unwrap_or("");

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Name: "),
            Span::styled(
                format!("{name_buf}\u{2588}"),
                Style::default()
                    .fg(theme.conn_connecting)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    frame.render_widget(ratatui::widgets::Clear, popup);

    let content = Paragraph::new(text).block(block);
    frame.render_widget(content, popup);
}

pub(super) fn render_bind_variables(
    frame: &mut Frame,
    state: &AppState,
    theme: &Theme,
    area: Rect,
) {
    let bv = match &state.dialogs.bind_variables {
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
            format!("{value}\u{2588}") // block cursor
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

pub(super) fn render_theme_picker(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
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
            let icon = if is_current { "\u{25cf} " } else { "  " };
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
    list_state.select(Some(state.dialogs.theme_picker.cursor));

    let list = ratatui::widgets::List::new(items)
        .highlight_style(
            Style::default()
                .bg(theme.tree_selected_bg)
                .fg(theme.tree_selected_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{25b8} ");

    frame.render_stateful_widget(list, inner, &mut list_state);
}

pub(super) fn render_export_dialog(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    let dialog = match &state.dialogs.export_dialog {
        Some(d) => d,
        None => return,
    };

    let width = 56u16.min(area.width.saturating_sub(4));
    let height = 13u16.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    let border_color = if dialog.error.is_some() {
        theme.error_fg
    } else {
        theme.border_focused
    };
    // Clear the area behind the dialog and paint an opaque background so
    // the editor/grid underneath doesn't bleed through.
    frame.render_widget(ratatui::widgets::Clear, dialog_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(" Export ")
        .style(Style::default().bg(theme.dialog_bg));
    let inner_block = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    let inner = inner_block;
    let mut lines = Vec::new();
    let focused_fg = theme.border_focused;
    let normal_fg = theme.status_fg;

    // Error line
    if let Some(ref err) = dialog.error {
        lines.push(Line::from(Span::styled(
            err.as_str(),
            Style::default().fg(theme.error_fg),
        )));
    }

    // Path
    let path_fg = if dialog.focused == ExportField::Path {
        focused_fg
    } else {
        normal_fg
    };
    lines.push(Line::from(vec![
        Span::styled("Path: ", Style::default().fg(path_fg)),
        Span::styled(dialog.path.as_str(), Style::default().fg(path_fg)),
        if dialog.focused == ExportField::Path {
            Span::styled("\u{2588}", Style::default().fg(path_fg))
        } else {
            Span::raw("")
        },
    ]));

    // Include credentials
    let cred_fg = if dialog.focused == ExportField::IncludeCredentials {
        focused_fg
    } else {
        normal_fg
    };
    let cred_val = if dialog.include_credentials {
        "[Y] Yes"
    } else {
        "[N] No "
    };
    lines.push(Line::from(vec![
        Span::styled("Include credentials? ", Style::default().fg(cred_fg)),
        Span::styled(
            cred_val,
            Style::default().fg(cred_fg).add_modifier(Modifier::BOLD),
        ),
    ]));

    // Show password toggle
    let sp_fg = if dialog.focused == ExportField::ShowPassword {
        focused_fg
    } else {
        normal_fg
    };
    let sp_val = if dialog.show_password {
        "[Y] Yes"
    } else {
        "[N] No "
    };
    lines.push(Line::from(vec![
        Span::styled("Show password?      ", Style::default().fg(sp_fg)),
        Span::styled(
            sp_val,
            Style::default().fg(sp_fg).add_modifier(Modifier::BOLD),
        ),
    ]));

    // Password
    let pw_fg = if dialog.focused == ExportField::Password {
        focused_fg
    } else {
        normal_fg
    };
    let pw_display = if dialog.show_password {
        dialog.password.clone()
    } else {
        "*".repeat(dialog.password.len())
    };
    lines.push(Line::from(vec![
        Span::styled("Password: ", Style::default().fg(pw_fg)),
        Span::styled(pw_display, Style::default().fg(pw_fg)),
        if dialog.focused == ExportField::Password {
            Span::styled("\u{2588}", Style::default().fg(pw_fg))
        } else {
            Span::raw("")
        },
    ]));

    // Confirm
    let cf_fg = if dialog.focused == ExportField::Confirm {
        focused_fg
    } else {
        normal_fg
    };
    let cf_display = if dialog.show_password {
        dialog.confirm.clone()
    } else {
        "*".repeat(dialog.confirm.len())
    };
    lines.push(Line::from(vec![
        Span::styled("Confirm:  ", Style::default().fg(cf_fg)),
        Span::styled(cf_display, Style::default().fg(cf_fg)),
        if dialog.focused == ExportField::Confirm {
            Span::styled("\u{2588}", Style::default().fg(cf_fg))
        } else {
            Span::raw("")
        },
    ]));

    // Footer
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("[Enter] Export  ", Style::default().fg(normal_fg)),
        Span::styled("[Esc] Cancel  ", Style::default().fg(normal_fg)),
        Span::styled("[Tab] Next field", Style::default().fg(normal_fg)),
    ]));

    let content = Paragraph::new(lines);
    frame.render_widget(content, inner);
}

pub(super) fn render_import_dialog(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    let dialog = match &state.dialogs.import_dialog {
        Some(d) => d,
        None => return,
    };

    let width = 56u16.min(area.width.saturating_sub(4));
    let height = 10u16.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    let border_color = if dialog.error.is_some() {
        theme.error_fg
    } else {
        theme.border_focused
    };
    // Clear + opaque background so the editor underneath doesn't bleed
    // through the dialog.
    frame.render_widget(ratatui::widgets::Clear, dialog_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(" Import ")
        .style(Style::default().bg(theme.dialog_bg));
    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);
    let mut lines = Vec::new();
    let focused_fg = theme.border_focused;
    let normal_fg = theme.status_fg;

    // Error line
    if let Some(ref err) = dialog.error {
        lines.push(Line::from(Span::styled(
            err.as_str(),
            Style::default().fg(theme.error_fg),
        )));
    }

    // File path
    let path_fg = if dialog.focused == ImportField::Path {
        focused_fg
    } else {
        normal_fg
    };
    lines.push(Line::from(vec![
        Span::styled("File: ", Style::default().fg(path_fg)),
        Span::styled(dialog.path.as_str(), Style::default().fg(path_fg)),
        if dialog.focused == ImportField::Path {
            Span::styled("\u{2588}", Style::default().fg(path_fg))
        } else {
            Span::raw("")
        },
    ]));

    // Show password toggle
    let sp_fg = if dialog.focused == ImportField::ShowPassword {
        focused_fg
    } else {
        normal_fg
    };
    let sp_val = if dialog.show_password {
        "[Y] Yes"
    } else {
        "[N] No "
    };
    lines.push(Line::from(vec![
        Span::styled("Show password?  ", Style::default().fg(sp_fg)),
        Span::styled(
            sp_val,
            Style::default().fg(sp_fg).add_modifier(Modifier::BOLD),
        ),
    ]));

    // Password
    let pw_fg = if dialog.focused == ImportField::Password {
        focused_fg
    } else {
        normal_fg
    };
    let pw_display = if dialog.show_password {
        dialog.password.clone()
    } else {
        "*".repeat(dialog.password.len())
    };
    lines.push(Line::from(vec![
        Span::styled("Password: ", Style::default().fg(pw_fg)),
        Span::styled(pw_display, Style::default().fg(pw_fg)),
        if dialog.focused == ImportField::Password {
            Span::styled("\u{2588}", Style::default().fg(pw_fg))
        } else {
            Span::raw("")
        },
    ]));

    // Footer
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("[Enter] Import  ", Style::default().fg(normal_fg)),
        Span::styled("[Esc] Cancel  ", Style::default().fg(normal_fg)),
        Span::styled("[Tab] Next field", Style::default().fg(normal_fg)),
    ]));

    let content = Paragraph::new(lines);
    frame.render_widget(content, inner);
}

pub(super) fn render_leader_help(
    frame: &mut Frame,
    state: &AppState,
    theme: &Theme,
    area: Rect,
    level: usize,
) {
    use crate::keybindings::Context;
    use ratatui::style::Color;

    let key_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(theme.status_fg);
    let header_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);

    // Read the configured key for (context, action) at render time so the
    // popup always reflects the user's current keybindings.toml.
    let pk = |ctx: Context, action: &str| state.bindings.primary_key(ctx, action);
    let owned: Vec<(String, &str)>;
    let title: &str;
    match level {
        2 => {
            title = "Leader > b";
            owned = vec![(pk(Context::LeaderBuffer, "close_tab"), "close buffer")];
        }
        3 => {
            title = "Leader > Leader";
            // <leader><leader>s is hardcoded (compile_to_db isn't in any context map).
            owned = vec![("s".to_string(), "compile to DB")];
        }
        4 => {
            title = "Leader > w";
            owned = vec![(pk(Context::LeaderWindow, "close_group"), "close group")];
        }
        5 => {
            title = "Leader > s";
            owned = vec![
                (pk(Context::LeaderSnippet, "snippet_select"), "SELECT"),
                (pk(Context::LeaderSnippet, "snippet_update"), "UPDATE"),
                (pk(Context::LeaderSnippet, "snippet_delete"), "DELETE"),
                (
                    pk(Context::LeaderSnippet, "snippet_call_proc"),
                    "CALL/EXEC proc",
                ),
                (
                    pk(Context::LeaderSnippet, "snippet_select_func"),
                    "SELECT func",
                ),
                (
                    pk(Context::LeaderSnippet, "snippet_create_table"),
                    "CREATE TABLE",
                ),
            ];
        }
        6 => {
            title = "Leader > f";
            owned = vec![
                (
                    pk(Context::LeaderFile, "export_connections"),
                    "export connections",
                ),
                (
                    pk(Context::LeaderFile, "import_connections"),
                    "import connections",
                ),
            ];
        }
        7 => {
            title = "Leader > q";
            owned = vec![(pk(Context::LeaderQuit, "quit_app"), "quit app")];
        }
        _ => {
            title = "Leader (Space)";
            owned = vec![
                (pk(Context::Leader, "execute_query"), "execute query"),
                (
                    pk(Context::Leader, "execute_query_new_tab"),
                    "execute \u{2192} new tab",
                ),
                (pk(Context::Leader, "toggle_sidebar"), "toggle sidebar"),
                (
                    pk(Context::Leader, "toggle_oil_navigator"),
                    "floating navigator",
                ),
                (pk(Context::Leader, "vertical_split"), "vertical split"),
                (
                    pk(Context::Leader, "move_tab_to_other_group"),
                    "move tab to other group",
                ),
                (
                    pk(Context::Leader, "open_script_connection_picker"),
                    "connection",
                ),
                (pk(Context::Leader, "open_theme_picker"), "theme"),
                (
                    pk(Context::Leader, "toggle_diagnostic_list"),
                    "diagnostics",
                ),
                (
                    pk(Context::Leader, "open_file_submenu"),
                    "+file (export/import)",
                ),
                (pk(Context::Leader, "open_quit_submenu"), "+quit..."),
                (
                    pk(Context::Leader, "open_snippet_submenu"),
                    "+snippets...",
                ),
                (pk(Context::Leader, "open_buffer_submenu"), "+buffer..."),
                (
                    pk(Context::Leader, "open_window_submenu"),
                    "+close group...",
                ),
                ("Spc".to_string(), "+compile..."),
            ];
        }
    };
    let entries: Vec<(&str, &str)> = owned
        .iter()
        .map(|(k, d)| (k.as_str(), *d))
        .collect();

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
        .style(Style::default().bg(Color::Reset));

    let content = Paragraph::new(lines).block(block);
    frame.render_widget(content, popup);
}

pub(super) fn render_script_conn_picker(
    frame: &mut Frame,
    state: &AppState,
    theme: &Theme,
    area: Rect,
) {
    use crate::ui::state::PickerItem;

    let picker = match &state.dialogs.script_conn_picker {
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
                Span::styled("\u{25cf} ", Style::default().fg(theme.conn_connected)),
                Span::styled(name.as_str(), Style::default().fg(theme.topbar_fg)),
            ])),
            PickerItem::OthersHeader => {
                let arrow = if picker.others_expanded {
                    "\u{25bc}"
                } else {
                    "\u{25b6}"
                };
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
                Span::styled("\u{25cb} ", Style::default().fg(theme.dim)),
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
        .highlight_symbol("\u{25b8} ");

    frame.render_stateful_widget(list, inner, &mut list_state);
}
