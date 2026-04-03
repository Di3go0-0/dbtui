use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Text;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::core::models::QueryResult;
use crate::ui::state::Mode;
use crate::ui::tabs::WorkspaceTab;
use crate::ui::theme::Theme;

pub fn render_for_tab(
    frame: &mut Frame,
    tab: &mut WorkspaceTab,
    focused: bool,
    theme: &Theme,
    area: Rect,
    mode: &Mode,
) {
    // Grid is focused if: it's a table/view (always), or it's a script with grid_focused=true
    let is_table_view = matches!(tab.kind, crate::ui::tabs::TabKind::Table { .. });
    let grid_active = focused && (is_table_view || tab.grid_focused);
    let border_style = theme.border_style(grid_active, mode);

    let result = match &tab.query_result {
        Some(r) => r,
        None => {
            let block = Block::default()
                .title(" Data ")
                .borders(Borders::ALL)
                .border_style(border_style)
                .style(Style::default().bg(theme.editor_bg));
            let empty_rows: Vec<Row> = vec![];
            let empty = Table::new(empty_rows, &[Constraint::Min(1)]).block(block);
            frame.render_widget(empty, area);
            return;
        }
    };

    let visible_height = area.height.saturating_sub(4) as usize;
    tab.grid_visible_height = visible_height.max(1);

    let total_rows = result.rows.len();

    // Determine selection range for highlighting (row, col)
    let sel_range: Option<((usize, usize), (usize, usize))> = tab.grid_selection_anchor.map(|(ar, ac)| {
        let cur = (tab.grid_selected_row, tab.grid_selected_col);
        let anchor = (ar, ac);
        if anchor <= cur { (anchor, cur) } else { (cur, anchor) }
    });

    let visual_tag = if tab.grid_visual_mode { " VISUAL " } else { "" };
    let status = format!(
        " Data [{}-{} of {}] {visual_tag}",
        if total_rows > 0 { tab.grid_scroll_row + 1 } else { 0 },
        (tab.grid_scroll_row + visible_height).min(total_rows),
        total_rows
    );

    let block = Block::default()
        .title(status)
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(theme.editor_bg));

    let col_widths = compute_column_widths(result, area.width.saturating_sub(2));

    // Highlight selected column in header
    let header_cells: Vec<Cell> = result
        .columns
        .iter()
        .map(|c| Cell::from(Text::from(c.as_str())).style(theme.grid_header_style()))
        .collect();
    let header = Row::new(header_cells)
        .height(1)
        .style(Style::default().bg(theme.grid_header_bg));

    let selected_col = tab.grid_selected_col;
    let is_grid_focused = grid_active;

    let rows: Vec<Row> = result
        .rows
        .iter()
        .skip(tab.grid_scroll_row)
        .take(visible_height)
        .enumerate()
        .map(|(vis_idx, row_data)| {
            let absolute_idx = tab.grid_scroll_row + vis_idx;
            let row_style = theme.grid_row_style(absolute_idx);

            let cells: Vec<Cell> = row_data
                .iter()
                .enumerate()
                .map(|(col_idx, val)| {
                    let base_style = if val == "NULL" {
                        theme.null_style()
                    } else if val.parse::<f64>().is_ok() {
                        Style::default().fg(theme.grid_number)
                    } else {
                        Style::default()
                    };

                    let is_cursor = is_grid_focused
                        && absolute_idx == tab.grid_selected_row
                        && col_idx == selected_col;

                    let in_selection = sel_range.is_some_and(|((sr, sc), (er, ec))| {
                        let in_row = absolute_idx >= sr && absolute_idx <= er;
                        let in_col = col_idx >= sc && col_idx <= ec;
                        in_row && in_col
                    });

                    if is_cursor {
                        Cell::from(Text::from(val.as_str())).style(
                            base_style
                                .bg(theme.grid_selected_bg)
                                .fg(theme.grid_selected_fg)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else if in_selection {
                        Cell::from(Text::from(val.as_str())).style(
                            base_style
                                .bg(ratatui::style::Color::Rgb(40, 50, 70))
                                .fg(ratatui::style::Color::Rgb(200, 210, 230)),
                        )
                    } else {
                        Cell::from(Text::from(val.as_str())).style(base_style)
                    }
                })
                .collect();
            Row::new(cells).style(row_style)
        })
        .collect();

    let table = Table::new(rows, &col_widths)
        .header(header)
        .block(block)
        .column_spacing(1);

    frame.render_widget(table, area);
}

fn compute_column_widths(result: &QueryResult, available: u16) -> Vec<Constraint> {
    if result.columns.is_empty() {
        return vec![];
    }

    let num_cols = result.columns.len();
    let per_col = available / num_cols as u16;
    let min_width = per_col.max(8);

    result
        .columns
        .iter()
        .map(|_| Constraint::Min(min_width))
        .collect()
}
