use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Text;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::core::models::QueryResult;
use crate::ui::state::Mode;
use crate::ui::tabs::{RowChange, WorkspaceTab};
use crate::ui::theme::Theme;

pub fn render_for_tab(
    frame: &mut Frame,
    tab: &mut WorkspaceTab,
    focused: bool,
    theme: &Theme,
    area: Rect,
    mode: &Mode,
) {
    let is_table_view = matches!(tab.kind, crate::ui::tabs::TabKind::Table { .. });
    let grid_active = focused && (is_table_view || tab.grid_focused);
    let border_style = theme.border_style(grid_active, mode);

    if tab.query_result.is_none() {
        let block = Block::default()
            .title(" Data ")
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(Style::default().bg(theme.editor_bg));
        if tab.streaming {
            let msg = crate::ui::loading::fetching_text(tab.streaming_since);
            let loading = Paragraph::new(msg)
                .style(Style::default().fg(Color::Yellow).bg(theme.editor_bg))
                .alignment(Alignment::Left)
                .block(block);
            frame.render_widget(loading, area);
        } else {
            let empty_rows: Vec<Row> = vec![];
            let empty = Table::new(empty_rows, &[Constraint::Min(1)]).block(block);
            frame.render_widget(empty, area);
        }
        return;
    }

    let visible_height = area.height.saturating_sub(4) as usize;
    tab.grid_visible_height = visible_height.max(1);
    let available_width = area.width.saturating_sub(2) as usize;

    // Compute widths and scroll (mutable access to tab)
    let col_widths = compute_smart_widths(tab.query_result.as_ref().expect("checked"));
    ensure_col_visible(tab, &col_widths, available_width);

    // Now immutably borrow result for rendering
    let result = tab.query_result.as_ref().expect("checked");
    let total_rows = result.rows.len();
    let total_cols = result.columns.len();

    let scroll_col = tab.grid_scroll_col;

    // Determine which columns fit on screen starting from scroll_col
    let (vis_col_start, vis_col_end) = visible_col_range(&col_widths, scroll_col, available_width);

    // Selection range
    let sel_range: Option<((usize, usize), (usize, usize))> =
        tab.grid_selection_anchor.map(|(ar, ac)| {
            let cur = (tab.grid_selected_row, tab.grid_selected_col);
            let anchor = (ar, ac);
            if anchor <= cur {
                (anchor, cur)
            } else {
                (cur, anchor)
            }
        });

    let visual_tag = if tab.grid_visual_mode { " VISUAL " } else { "" };
    let col_info = if total_cols > vis_col_end - vis_col_start {
        format!(" cols {}-{}/{}", vis_col_start + 1, vis_col_end, total_cols)
    } else {
        String::new()
    };
    let status = format!(
        " Data [{}-{} of {}]{col_info} {visual_tag}",
        if total_rows > 0 {
            tab.grid_scroll_row + 1
        } else {
            0
        },
        (tab.grid_scroll_row + visible_height).min(total_rows),
        total_rows
    );

    let block = Block::default()
        .title(status)
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(theme.editor_bg));

    // Row number column width (based on total rows digit count)
    let row_num_width = if total_rows == 0 {
        2
    } else {
        total_rows.to_string().len().max(2)
    } as u16;

    // Build constraints: row number (fixed) + visible data columns (fixed, last fills remaining)
    let mut vis_constraints: Vec<Constraint> = Vec::with_capacity(1 + vis_col_end - vis_col_start);
    vis_constraints.push(Constraint::Length(row_num_width));
    let vis_count = vis_col_end - vis_col_start;
    for (i, &w) in col_widths[vis_col_start..vis_col_end].iter().enumerate() {
        if i == vis_count - 1 {
            // Last visible column fills remaining space
            vis_constraints.push(Constraint::Min(w as u16));
        } else {
            vis_constraints.push(Constraint::Length(w as u16));
        }
    }

    // Header: row number + visible columns
    let mut header_cells: Vec<Cell> = Vec::with_capacity(1 + vis_col_end - vis_col_start);
    header_cells.push(
        Cell::from(Text::from("#")).style(
            theme
                .grid_header_style()
                .fg(theme.dim)
                .add_modifier(Modifier::DIM),
        ),
    );
    header_cells.extend(
        result.columns[vis_col_start..vis_col_end]
            .iter()
            .map(|c| Cell::from(Text::from(c.as_str())).style(theme.grid_header_style())),
    );
    let header = Row::new(header_cells)
        .height(1)
        .style(Style::default().bg(theme.grid_header_bg));

    let selected_col = tab.grid_selected_col;
    let is_grid_focused = grid_active;

    // Rows (visible columns only)
    let rows: Vec<Row> = result
        .rows
        .iter()
        .skip(tab.grid_scroll_row)
        .take(visible_height)
        .enumerate()
        .map(|(vis_idx, row_data)| {
            let absolute_idx = tab.grid_scroll_row + vis_idx;
            let row_style = theme.grid_row_style(absolute_idx);

            // Check for pending changes on this row
            let row_change = tab.grid_changes.get(&absolute_idx);
            let is_deleted = matches!(row_change, Some(RowChange::Deleted));
            let is_new = matches!(row_change, Some(RowChange::New { .. }));
            let modified_cols: Vec<usize> = match row_change {
                Some(RowChange::Modified { edits }) => edits.iter().map(|e| e.col).collect(),
                _ => vec![],
            };

            let row_num_style = if is_deleted {
                Style::default().fg(Color::Red).add_modifier(Modifier::DIM)
            } else if is_new {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(theme.dim).add_modifier(Modifier::DIM)
            };
            let row_num =
                Cell::from(Text::from(format!("{}", absolute_idx + 1))).style(row_num_style);
            let mut cells: Vec<Cell> = Vec::with_capacity(1 + vis_col_end - vis_col_start);
            cells.push(row_num);
            cells.extend((vis_col_start..vis_col_end).map(|col_idx| {
                // Check if this cell is being edited inline
                let is_editing = tab.grid_editing == Some((absolute_idx, col_idx));
                let val: String = if is_editing {
                    // Show edit buffer with cursor
                    let buf = &tab.grid_edit_buffer;
                    let cur = tab.grid_edit_cursor.min(buf.len());
                    if cur < buf.len() {
                        format!("{}|{}", &buf[..cur], &buf[cur..])
                    } else {
                        format!("{buf}|")
                    }
                } else {
                    row_data.get(col_idx).cloned().unwrap_or_default()
                };

                let base_style = if is_editing {
                    Style::default().fg(Color::White).bg(Color::Rgb(30, 40, 80))
                } else if is_deleted {
                    Style::default().fg(Color::Red).add_modifier(Modifier::DIM)
                } else if is_new {
                    Style::default().fg(Color::Green).bg(Color::Rgb(20, 40, 20))
                } else if modified_cols.contains(&col_idx) {
                    Style::default()
                        .fg(Color::Yellow)
                        .bg(Color::Rgb(50, 45, 15))
                } else if val.as_str() == "NULL" {
                    theme.null_style()
                } else if val.parse::<f64>().is_ok() {
                    Style::default().fg(theme.grid_number)
                } else {
                    Style::default()
                };

                let is_cursor = is_grid_focused
                    && absolute_idx == tab.grid_selected_row
                    && col_idx == selected_col
                    && !is_editing;

                let in_selection = sel_range.is_some_and(|((sr, sc), (er, ec))| {
                    absolute_idx >= sr && absolute_idx <= er && col_idx >= sc && col_idx <= ec
                });

                if is_cursor {
                    Cell::from(Text::from(val)).style(
                        base_style
                            .bg(theme.grid_selected_bg)
                            .fg(theme.grid_selected_fg)
                            .add_modifier(Modifier::BOLD),
                    )
                } else if in_selection {
                    Cell::from(Text::from(val)).style(
                        base_style
                            .bg(Color::Rgb(40, 50, 70))
                            .fg(Color::Rgb(200, 210, 230)),
                    )
                } else {
                    Cell::from(Text::from(val)).style(base_style)
                }
            }));
            Row::new(cells).style(row_style)
        })
        .collect();

    let table = Table::new(rows, &vis_constraints)
        .header(header)
        .block(block)
        .column_spacing(1);

    frame.render_widget(table, area);
}

/// Compute column widths based on header + content (capped at 40)
fn compute_smart_widths(result: &QueryResult) -> Vec<usize> {
    let mut widths: Vec<usize> = result
        .columns
        .iter()
        .map(|c| c.len().max(4)) // min 4, start with header width
        .collect();

    // Sample first 100 rows to determine max content width
    for row in result.rows.iter().take(100) {
        for (i, val) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(val.len());
            }
        }
    }

    // Cap at 40 characters per column
    for w in &mut widths {
        *w = (*w).min(40);
    }

    widths
}

/// Ensure the selected column is visible by adjusting grid_scroll_col
fn ensure_col_visible(tab: &mut WorkspaceTab, col_widths: &[usize], available: usize) {
    let sel = tab.grid_selected_col;

    // If selected column is before scroll, scroll left
    if sel < tab.grid_scroll_col {
        tab.grid_scroll_col = sel;
        return;
    }

    // If selected column is past visible range, scroll right
    let mut used = 0;
    for (i, &w) in col_widths.iter().enumerate().skip(tab.grid_scroll_col) {
        used += w + 1; // +1 for column spacing
        if i == sel {
            if used > available {
                // Need to scroll right
                tab.grid_scroll_col = sel;
                // Try to show a few columns before too
                let mut back_used = 0;
                let mut new_start = sel;
                for j in (0..sel).rev() {
                    back_used += col_widths[j] + 1;
                    if back_used + col_widths[sel] + 1 > available {
                        break;
                    }
                    new_start = j;
                }
                tab.grid_scroll_col = new_start;
            }
            return;
        }
    }
}

/// Determine which columns are visible given scroll offset and available width
fn visible_col_range(col_widths: &[usize], scroll_col: usize, available: usize) -> (usize, usize) {
    let start = scroll_col.min(col_widths.len());
    let mut used = 0;
    let mut end = start;
    for &w in &col_widths[start..] {
        if used + w + 1 > available && end > start {
            break;
        }
        used += w + 1;
        end += 1;
    }
    (start, end)
}
