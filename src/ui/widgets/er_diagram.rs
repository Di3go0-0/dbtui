//! Renders a diagram tab: the ASCII canvas through a scrolling viewport,
//! with the selected table and its edges picked out, a status line and
//! the search prompt.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::keybindings::{Context, KeyBindings};
use crate::ui::diagram::canvas::{Owner, Part};
use crate::ui::diagram::{DiagramState, KeyKind};
use crate::ui::state::Mode;
use crate::ui::tabs::WorkspaceTab;
use crate::ui::theme::Theme;

pub fn render_for_tab(
    frame: &mut Frame,
    tab: &mut WorkspaceTab,
    focused: bool,
    theme: &Theme,
    bindings: &KeyBindings,
    area: Rect,
    mode: &Mode,
) {
    let Some(diagram) = tab.diagram.as_mut() else {
        return;
    };
    let loading_since = diagram.loading_since;
    let border_style = theme.border_style(focused, mode);
    let title = format!(" ER \u{00b7} {} ", diagram.schema);
    let footer = footer_line(diagram, theme, bindings);
    let block = Block::default()
        .title(title)
        .title_bottom(footer)
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(theme.editor_bg));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(error) = &diagram.error {
        let text = format!("{error}\n\nPress r to retry.");
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(theme.error_fg)),
            inner,
        );
        return;
    }
    if !diagram.loaded {
        let text = if loading_since.is_some() {
            crate::ui::loading::fetching_text(loading_since)
        } else {
            "Press r to load the schema.".to_string()
        };
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(theme.dim)),
            inner,
        );
        return;
    }
    if diagram.graph().is_empty() {
        frame.render_widget(
            Paragraph::new("No tables in this schema.").style(Style::default().fg(theme.dim)),
            inner,
        );
        return;
    }

    diagram.viewport = (inner.width, inner.height);
    diagram.apply_scroll_request();
    diagram.clamp_scroll();
    blit(frame, diagram, theme, inner);

    if let Some(query) = &diagram.search {
        let landed = match &diagram.last_hit {
            Some(hit) => match &hit.column {
                Some(column) => format!("  \u{2192} {}.{column}", hit.table),
                None => format!("  \u{2192} {}", hit.table),
            },
            None if query.is_empty() => String::new(),
            None => "  (no match)".to_string(),
        };
        let prompt = format!(
            "{}{query}\u{2588}{landed}",
            diagram.search_kind.prompt_char()
        );
        let width = (prompt.chars().count() as u16).min(inner.width);
        let rect = Rect {
            x: inner.x,
            y: inner.y + inner.height.saturating_sub(1),
            width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(prompt).style(
                Style::default()
                    .fg(theme.accent)
                    .bg(theme.editor_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            rect,
        );
    }
}

/// The key hints, read from the bindings so a remapped key shows as the
/// user set it.
fn footer_line(diagram: &DiagramState, theme: &Theme, bindings: &KeyBindings) -> Line<'static> {
    let k = |action: &str| bindings.primary_key(Context::Diagram, action);
    let hint = if diagram.z_pending {
        format!(
            "  {}: {} zoom in  {} zoom out  {} centre ",
            k("zoom_prefix"),
            k("zoom_in"),
            k("zoom_out"),
            k("center")
        )
    } else if diagram.column_cursor.is_some() {
        format!(
            "  {}/{} column  {} follow key  {} back to tables ",
            k("select_down"),
            k("select_up"),
            k("enter_columns"),
            k("leave_columns")
        )
    } else {
        let moves = format!(
            "{}{}{}{}",
            k("select_left"),
            k("select_down"),
            k("select_up"),
            k("select_right")
        );
        let focus = if diagram.focus.is_some() {
            format!(
                "{} whole  {}/{} hops",
                k("toggle_focus"),
                k("depth_more"),
                k("depth_less")
            )
        } else {
            format!("{} focus", k("toggle_focus"))
        };
        format!(
            "  {moves} select  {} columns  {}-{} focus  {focus}  {z}{}/{z}{} zoom  {} open  {} table  {} column  {} mermaid ",
            k("enter_columns"),
            bindings.primary_key(Context::Global, "leader"),
            bindings.primary_key(Context::Leader, "execute_query"),
            k("zoom_in"),
            k("zoom_out"),
            k("open_table"),
            k("search"),
            k("search_columns"),
            k("export_diagram"),
            z = k("zoom_prefix"),
        )
    };
    Line::from(vec![
        ratatui::text::Span::styled(
            format!(" {} ", diagram.summary()),
            Style::default().fg(theme.tab_active_fg),
        ),
        ratatui::text::Span::styled(hint, Style::default().fg(theme.dim)),
    ])
}

/// Copy the visible window of the canvas into the frame, colouring each
/// cell by what it belongs to and how it relates to the selection.
fn blit(frame: &mut Frame, diagram: &mut DiagramState, theme: &Theme, inner: Rect) {
    let selected = diagram.selected_box();
    let column_cursor = diagram.column_cursor;
    let layout = diagram.layout();
    let incident: Vec<bool> = layout
        .edges
        .iter()
        .map(|e| selected.is_some_and(|s| e.from == s || e.to == s))
        .collect();
    let external: Vec<bool> = layout.boxes.iter().map(|b| b.external).collect();
    // Canvas span (x0, x1, y) of the column row under the cursor.
    let cursor_row: Option<(i32, i32, i32)> = selected.zip(column_cursor).map(|(s, c)| {
        let b = &layout.boxes[s];
        let m = crate::ui::diagram::canvas::MARGIN;
        (b.x + m + 1, b.x + m + b.width as i32 - 2, b.row_y(c) + m)
    });
    let (scroll_x, scroll_y) = (diagram.scroll_x, diagram.scroll_y);
    let canvas = diagram.canvas();
    let bg = theme.editor_bg;
    let buffer = frame.buffer_mut();

    for row in 0..inner.height {
        for col in 0..inner.width {
            let (cx, cy) = (scroll_x + col as i32, scroll_y + row as i32);
            let Some(cell) = canvas.get(cx, cy) else {
                continue;
            };
            let on_cursor_row =
                cursor_row.is_some_and(|(x0, x1, y)| cy == y && (x0..=x1).contains(&cx));
            let style = match (cell.owner, cell.part) {
                (Owner::Node(n), Part::Border) => {
                    let color = if Some(n) == selected {
                        theme.accent
                    } else {
                        theme.border_unfocused
                    };
                    Style::default().fg(color)
                }
                (Owner::Node(n), Part::Title) => {
                    let color = if Some(n) == selected {
                        theme.accent
                    } else if external.get(n).copied().unwrap_or(false) {
                        theme.dim
                    } else {
                        theme.tree_table
                    };
                    Style::default().fg(color).add_modifier(Modifier::BOLD)
                }
                (_, Part::Key(kind)) => Style::default().fg(match kind {
                    KeyKind::Primary => theme.conn_connected,
                    KeyKind::Foreign => theme.sql_number,
                    KeyKind::Both => theme.accent,
                    KeyKind::None => theme.dim,
                }),
                (_, Part::ColumnName) => Style::default().fg(theme.status_fg),
                (_, Part::ColumnType) | (_, Part::Note) => Style::default().fg(theme.dim),
                (Owner::Edge(e), Part::Line) | (Owner::Edge(e), Part::Arrow) => {
                    let on = incident.get(e).copied().unwrap_or(false);
                    let mut style =
                        Style::default().fg(if on { theme.accent } else { theme.separator });
                    if on {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    style
                }
                _ => Style::default(),
            };
            let style = if on_cursor_row {
                style
                    .bg(theme.tree_selected_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                style.bg(bg)
            };
            if let Some(target) = buffer.cell_mut((inner.x + col, inner.y + row)) {
                target.set_char(cell.ch);
                target.set_style(style);
            }
        }
    }
}
