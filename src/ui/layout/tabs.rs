use super::*;

pub(super) fn render_tab_bar(frame: &mut Frame, state: &mut AppState, theme: &Theme, area: Rect) {
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

    // Calculate per-tab widths to scroll so the active tab is visible
    let available_width = area.width as usize;
    let mut tab_positions: Vec<(usize, usize)> = Vec::new(); // (start, end)
    let mut pos = 0;
    let mut span_idx = 0;
    for _ in 0..state.tabs.len() {
        let start = pos;
        while span_idx < spans.len() {
            pos += spans[span_idx].width();
            span_idx += 1;
            if spans[span_idx - 1].content.contains('\u{2502}') {
                break;
            }
        }
        tab_positions.push((start, pos));
    }

    let mut scroll_offset: usize = 0;
    if let Some(&(active_start, active_end)) = tab_positions.get(state.active_tab_idx) {
        if active_end > scroll_offset + available_width {
            scroll_offset = active_end.saturating_sub(available_width);
        }
        if active_start < scroll_offset {
            scroll_offset = active_start;
        }
    }

    // Count hidden tabs to the left and right
    let hidden_left = tab_positions
        .iter()
        .filter(|&&(_, end)| end <= scroll_offset)
        .count();
    let hidden_right = tab_positions
        .iter()
        .filter(|&&(start, _)| start >= scroll_offset + available_width)
        .count();

    let left_indicator = if hidden_left > 0 {
        format!("\u{25C0} {hidden_left} ")
    } else {
        String::new()
    };
    let right_indicator = if hidden_right > 0 {
        format!(" {hidden_right} \u{25B6}")
    } else {
        String::new()
    };
    let left_w = left_indicator.len() as u16;
    let right_w = right_indicator.len() as u16;

    // Render left indicator
    if !left_indicator.is_empty() {
        let left_area = Rect {
            x: area.x,
            y: area.y,
            width: left_w.min(area.width),
            height: 1,
        };
        let left = Paragraph::new(left_indicator).style(
            Style::default()
                .fg(Color::Yellow)
                .bg(theme.status_bg)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(left, left_area);
    }

    // Render right indicator
    if !right_indicator.is_empty() {
        let right_area = Rect {
            x: area.x + area.width.saturating_sub(right_w),
            y: area.y,
            width: right_w.min(area.width),
            height: 1,
        };
        let right = Paragraph::new(right_indicator).style(
            Style::default()
                .fg(Color::Yellow)
                .bg(theme.status_bg)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(right, right_area);
    }

    // Render tab bar in the middle area
    let mid_x = area.x + left_w;
    let mid_w = area.width.saturating_sub(left_w + right_w);
    let mid_area = Rect {
        x: mid_x,
        y: area.y,
        width: mid_w,
        height: 1,
    };
    let line = Line::from(spans);
    let bar = Paragraph::new(line)
        .style(Style::default().bg(theme.status_bg))
        .scroll((0, scroll_offset as u16));
    frame.render_widget(bar, mid_area);
}

pub(super) fn render_sub_view_bar(
    frame: &mut Frame,
    state: &mut AppState,
    theme: &Theme,
    area: Rect,
) {
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

pub(super) fn render_tab_content(
    frame: &mut Frame,
    state: &mut AppState,
    theme: &Theme,
    area: Rect,
) {
    let tab_idx = state.active_tab_idx;
    if tab_idx >= state.tabs.len() {
        return;
    }

    let focused = state.focus == Focus::TabContent;
    let mode = state.mode.clone();

    let sub_view = state.tabs[tab_idx].active_sub_view.clone();
    let loading_since = state.tabs[tab_idx].streaming_since;

    match sub_view {
        Some(SubView::TableData) => {
            use crate::ui::tabs::SubFocus;
            let tab = &mut state.tabs[tab_idx];
            let has_error = tab.grid_error_editor.is_some();
            if has_error {
                let splits = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(area);
                let grid_focused = focused && tab.sub_focus == SubFocus::Editor;
                widgets::data_grid::render_for_tab(
                    frame,
                    tab,
                    grid_focused,
                    theme,
                    splits[0],
                    &mode,
                );
                // Error panes below: error (left) + SQL (right)
                let error_splits = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(splits[1]);
                let vt = theme.vim_theme();
                let hl = crate::ui::sql_highlighter::SqlHighlighter::from_theme(theme);
                let err_focused = focused && tab.sub_focus == SubFocus::Results;
                let sql_focused = focused && tab.sub_focus == SubFocus::QueryView;
                let err_bright = Color::Rgb(220, 80, 80);
                let err_dim = Color::Rgb(120, 50, 50);
                let sql_bright = Color::Rgb(200, 180, 60);
                let sql_dim = Color::Rgb(100, 90, 30);
                if let Some(ref mut err_ed) = tab.grid_error_editor {
                    vimltui::render::render_with_options(
                        frame,
                        err_ed,
                        err_focused,
                        &vt,
                        &hl,
                        error_splits[0],
                        "Error",
                        Some(if err_focused { err_bright } else { err_dim }),
                    );
                }
                if let Some(ref mut q_ed) = tab.grid_query_editor {
                    vimltui::render::render_with_options(
                        frame,
                        q_ed,
                        sql_focused,
                        &vt,
                        &hl,
                        error_splits[1],
                        "SQL",
                        Some(if sql_focused { sql_bright } else { sql_dim }),
                    );
                }
            } else {
                widgets::data_grid::render_for_tab(frame, tab, focused, theme, area, &mode);
            }
        }
        Some(SubView::TableProperties) => {
            let tab = &state.tabs[tab_idx];
            widgets::properties::render_for_tab(frame, tab, focused, theme, area, &mode);
        }
        Some(SubView::TableDDL) => {
            let tab = &mut state.tabs[tab_idx];
            if let Some(editor) = tab.ddl_editor.as_mut() {
                crate::ui::loading::render_editor_or_loading(
                    frame,
                    editor,
                    focused,
                    theme,
                    area,
                    "DDL",
                    loading_since,
                );
            } else {
                crate::ui::loading::render_loading(frame, theme, area, "DDL", loading_since);
            }
        }
        Some(SubView::PackageDeclaration)
        | Some(SubView::TypeDeclaration)
        | Some(SubView::TriggerDeclaration) => {
            let tab = &mut state.tabs[tab_idx];
            let has_error = tab.grid_error_editor.is_some();
            if has_error {
                render_source_with_error(
                    frame,
                    tab,
                    focused,
                    theme,
                    area,
                    &mode,
                    "Declaration",
                    true,
                );
            } else if let Some(editor) = tab.decl_editor.as_mut() {
                crate::ui::loading::render_editor_or_loading(
                    frame,
                    editor,
                    focused,
                    theme,
                    area,
                    "Declaration",
                    loading_since,
                );
            } else {
                crate::ui::loading::render_loading(
                    frame,
                    theme,
                    area,
                    "Declaration",
                    loading_since,
                );
            }
        }
        Some(SubView::PackageBody) | Some(SubView::TypeBody) => {
            let tab = &mut state.tabs[tab_idx];
            let has_error = tab.grid_error_editor.is_some();
            if has_error {
                render_source_with_error(frame, tab, focused, theme, area, &mode, "Body", false);
            } else if let Some(editor) = tab.body_editor.as_mut() {
                crate::ui::loading::render_editor_or_loading(
                    frame,
                    editor,
                    focused,
                    theme,
                    area,
                    "Body",
                    loading_since,
                );
            } else {
                crate::ui::loading::render_loading(frame, theme, area, "Body", loading_since);
            }
        }
        Some(SubView::PackageFunctions) => {
            render_package_list(frame, state, theme, area, focused, true);
        }
        Some(SubView::PackageProcedures) => {
            render_package_list(frame, state, theme, area, focused, false);
        }
        Some(SubView::TypeAttributes)
        | Some(SubView::TypeMethods)
        | Some(SubView::TriggerColumns) => {
            let tab = &mut state.tabs[tab_idx];
            widgets::data_grid::render_for_tab(frame, tab, focused, theme, area, &mode);
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
                if is_source {
                    crate::ui::loading::render_editor_or_loading(
                        frame,
                        editor,
                        focused,
                        theme,
                        area,
                        &title,
                        loading_since,
                    );
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
                crate::ui::loading::render_loading(frame, theme, area, &title, loading_since);
            }
        }
    }
}

/// Render the completion popup below the cursor.
pub(super) fn render_completion_popup(
    frame: &mut Frame,
    state: &AppState,
    theme: &Theme,
    editor_area: Rect,
) {
    let cmp = match &state.engine.completion {
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
pub(super) fn render_diagnostic_underlines(
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

    for diag in &state.engine.diagnostics {
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
pub(super) fn render_script_with_results(
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

pub(super) fn render_package_list(
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
            let icon = if is_functions { "\u{03BB}" } else { "\u{0192}" };
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

#[allow(clippy::too_many_arguments)]
fn render_source_with_error(
    frame: &mut Frame,
    tab: &mut WorkspaceTab,
    focused: bool,
    theme: &Theme,
    area: Rect,
    _mode: &Mode,
    title: &str,
    is_decl: bool,
) {
    use crate::ui::tabs::SubFocus;

    let splits = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Top: source editor
    let editor_focused = focused && tab.sub_focus == SubFocus::Editor;
    let editor = if is_decl {
        tab.decl_editor.as_mut()
    } else {
        tab.body_editor.as_mut()
    };
    if let Some(editor) = editor {
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

    // Bottom: error (left) + SQL (right)
    let error_splits = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(splits[1]);

    let vt = theme.vim_theme();
    let hl = crate::ui::sql_highlighter::SqlHighlighter::from_theme(theme);
    let err_focused = focused && tab.sub_focus == SubFocus::Results;
    let sql_focused = focused && tab.sub_focus == SubFocus::QueryView;
    let err_bright = Color::Rgb(220, 80, 80);
    let err_dim = Color::Rgb(120, 50, 50);
    let sql_bright = Color::Rgb(200, 180, 60);
    let sql_dim = Color::Rgb(100, 90, 30);

    if let Some(ref mut err_ed) = tab.grid_error_editor {
        vimltui::render::render_with_options(
            frame,
            err_ed,
            err_focused,
            &vt,
            &hl,
            error_splits[0],
            "Error",
            Some(if err_focused { err_bright } else { err_dim }),
        );
    }
    if let Some(ref mut q_ed) = tab.grid_query_editor {
        vimltui::render::render_with_options(
            frame,
            q_ed,
            sql_focused,
            &vt,
            &hl,
            error_splits[1],
            "SQL",
            Some(if sql_focused { sql_bright } else { sql_dim }),
        );
    }
}
