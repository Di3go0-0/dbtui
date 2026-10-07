//! Drawing diagnostics over the editor: underlines on the offending text,
//! the list panel, and the hover tooltip.

use super::*;
use crate::ui::diagnostics::Severity;
use vimltui::VimEditor;

fn severity_color(severity: Severity, theme: &Theme) -> Color {
    match severity {
        Severity::Error => theme.error_fg,
        Severity::Warning => Color::Yellow,
        Severity::Info => Color::Blue,
        Severity::Hint => theme.dim,
    }
}

/// Area the active editor occupies inside the tab content area. Mirrors the
/// splits `render_tab_content` makes: scripts share the area with their
/// results 60/40, package sources with a compile error panel 50/50.
fn editor_rect(tab: &WorkspaceTab, area: Rect) -> Rect {
    let top = |percent: u16| {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(percent),
                Constraint::Percentage(100 - percent),
            ])
            .split(area)[0]
    };
    match tab.active_sub_view {
        None if tab.query_result.is_some() || !tab.result_tabs.is_empty() || tab.streaming => {
            top(60)
        }
        Some(
            SubView::PackageDeclaration
            | SubView::TypeDeclaration
            | SubView::TriggerDeclaration
            | SubView::PackageBody
            | SubView::TypeBody,
        ) if tab.grid_error_editor.is_some() => top(50),
        _ => area,
    }
}

/// Width of everything vimltui draws left of the text: the marks column, the
/// diagnostic sign column and the line numbers. Must match its render pass.
fn gutter_width(editor: &VimEditor) -> u16 {
    let marks = usize::from(!editor.marks.is_empty());
    let numbers = editor.lines.len().to_string().len().max(3);
    let signs = if editor
        .gutter
        .as_ref()
        .is_some_and(|g| !g.diagnostics.is_empty())
    {
        2
    } else {
        0
    };
    (marks + numbers + 2 + signs) as u16
}

/// Largest char boundary of `s` that is `<= idx`.
fn floor_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Underline each diagnostic's range in the editor.
///
/// Diagnostic columns are byte offsets into the line, while the screen is
/// measured in display cells and may be scrolled sideways, so the range is
/// converted before drawing. The existing cells are restyled in place — the
/// text, its background and the cursor line stay as the editor drew them.
pub(super) fn render_diagnostic_underlines(
    frame: &mut Frame,
    state: &AppState,
    theme: &Theme,
    content_area: Rect,
) {
    let Some(tab) = state.tabs.get(state.active_tab_idx) else {
        return;
    };
    let Some(editor) = tab.active_editor() else {
        return;
    };
    let area = editor_rect(tab, content_area);

    // Inside the border; the last inner row is the command line.
    let text_x = area.x + 1 + gutter_width(editor);
    let text_right = area.right().saturating_sub(1);
    let text_y = area.y + 1;
    let visible_rows = area.height.saturating_sub(3) as usize;

    for diag in &state.engine.diagnostics {
        if diag.row < editor.scroll_offset || diag.row >= editor.scroll_offset + visible_rows {
            continue;
        }
        let Some(line) = editor.lines.get(diag.row) else {
            continue;
        };

        let scrolled = floor_boundary(line, editor.horizontal_scroll);
        let end = floor_boundary(line, diag.col_end);
        let start = floor_boundary(line, diag.col_start).max(scrolled);
        if end < start {
            continue; // scrolled out of view to the left
        }

        let x = text_x.saturating_add(line[scrolled..start].width() as u16);
        let width = (line[start..end].width() as u16).max(1);
        let y = text_y + (diag.row - editor.scroll_offset) as u16;
        let style = Style::default()
            .fg(severity_color(diag.severity, theme))
            .add_modifier(Modifier::UNDERLINED);

        let buffer = frame.buffer_mut();
        for cell_x in x..x.saturating_add(width).min(text_right) {
            if let Some(cell) = buffer.cell_mut((cell_x, y)) {
                cell.set_style(style);
            }
        }
    }
}

/// Render the diagnostic list panel at the bottom of the editor area.
pub(super) fn render_diagnostic_list(
    frame: &mut Frame,
    state: &AppState,
    theme: &Theme,
    area: Rect,
) {
    let diagnostics = &state.engine.diagnostics;
    let block = Block::default()
        .title(format!(" Diagnostics ({}) ", diagnostics.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused))
        .style(Style::default().bg(theme.editor_bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if diagnostics.is_empty() {
        let msg = Paragraph::new(Span::styled(
            "  No diagnostics",
            Style::default().fg(theme.dim),
        ));
        frame.render_widget(msg, inner);
        return;
    }

    // Keep the selected entry in view when the list is longer than the panel.
    let visible = inner.height.max(1) as usize;
    let cursor = state.engine.diagnostic_list_cursor;
    let first = cursor.saturating_sub(visible - 1);

    let lines: Vec<Line<'_>> = diagnostics
        .iter()
        .enumerate()
        .skip(first)
        .take(visible)
        .map(|(i, d)| {
            let icon = match d.severity {
                Severity::Error => "✘",
                Severity::Warning => "⚠",
                Severity::Info => "ℹ",
                Severity::Hint => "·",
            };
            let bg = if i == cursor {
                theme.tree_selected_bg
            } else {
                Color::Reset
            };
            Line::from(vec![
                Span::styled(
                    format!(" {icon} "),
                    Style::default()
                        .fg(severity_color(d.severity, theme))
                        .bg(bg),
                ),
                Span::styled(
                    format!("{}:{} ", d.row + 1, d.col_start + 1),
                    Style::default().fg(theme.dim).bg(bg),
                ),
                Span::styled(
                    d.message.as_str(),
                    Style::default().fg(theme.status_fg).bg(bg),
                ),
                Span::styled(
                    format!("  [{}]", d.source.label()),
                    Style::default().fg(theme.dim).bg(bg),
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Break `text` into rows at most `max_width` cells wide, on word boundaries.
/// A single word wider than the row is split by characters.
fn wrap_to_width(text: &str, max_width: usize) -> Vec<String> {
    let max_width = max_width.max(1);
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let needed = current.width() + usize::from(!current.is_empty()) + word.width();
        if needed <= max_width {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
            continue;
        }
        if !current.is_empty() {
            rows.push(std::mem::take(&mut current));
        }
        for ch in word.chars() {
            if current.width() + ch.to_string().width() > max_width {
                rows.push(std::mem::take(&mut current));
            }
            current.push(ch);
        }
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

/// Render a floating tooltip with the diagnostic message near the cursor.
pub(super) fn render_diagnostic_hover(
    frame: &mut Frame,
    state: &AppState,
    theme: &Theme,
    content_area: Rect,
    diag_row: usize,
    message: &str,
) {
    let Some(tab) = state.tabs.get(state.active_tab_idx) else {
        return;
    };
    let Some(editor) = tab.active_editor() else {
        return;
    };
    let area = editor_rect(tab, content_area);
    let gutter = gutter_width(editor);

    let screen_row = diag_row.saturating_sub(editor.scroll_offset) as u16;
    let popup_x = area.x + 1 + gutter;
    let popup_y = area.y + 1 + screen_row; // line of the diagnostic

    let max_width = area.width.saturating_sub(gutter + 4).max(20) as usize;
    let rows = wrap_to_width(message, max_width);
    let height = rows.len() as u16 + 2; // +2 for borders
    let width = (rows.iter().map(|r| r.width()).max().unwrap_or(10) + 4) as u16;

    // Position above the line if possible, else below
    let y = if popup_y > height {
        popup_y - height
    } else {
        popup_y + 1
    };
    let x = popup_x.min(area.right().saturating_sub(width));
    let popup = Rect::new(x, y, width.min(area.width), height).intersection(frame.area());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused))
        .style(Style::default().bg(theme.dialog_bg));

    let text: Vec<Line<'_>> = rows
        .iter()
        .map(|r| {
            Line::from(Span::styled(
                r.as_str(),
                Style::default().fg(theme.status_fg),
            ))
        })
        .collect();

    frame.render_widget(ratatui::widgets::Clear, popup);
    frame.render_widget(Paragraph::new(text).block(block), popup);
}

#[cfg(test)]
mod tests {
    use super::{floor_boundary, wrap_to_width};

    #[test]
    fn wraps_on_words_and_measures_display_width() {
        assert_eq!(
            wrap_to_width("relación «año» no existe", 12),
            vec!["relación", "«año» no", "existe"]
        );
    }

    #[test]
    fn splits_a_word_longer_than_the_row() {
        assert_eq!(wrap_to_width("abcdefgh", 3), vec!["abc", "def", "gh"]);
    }

    #[test]
    fn floor_boundary_never_lands_inside_a_character() {
        let s = "año";
        assert_eq!(floor_boundary(s, 2), 1); // inside "ñ"
        assert_eq!(floor_boundary(s, 3), 3);
        assert_eq!(floor_boundary(s, 99), s.len());
    }
}
