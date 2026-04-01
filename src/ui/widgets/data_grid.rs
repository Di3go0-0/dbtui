use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::Text;
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};
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
    let border_style = theme.border_style(focused, mode);

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
    let status = format!(
        " Data [{}-{} of {}] ",
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

    let col_widths = compute_column_widths(result, area.width.saturating_sub(2));

    let header_cells: Vec<Cell> = result
        .columns
        .iter()
        .map(|c| Cell::from(Text::from(c.as_str())).style(theme.grid_header_style()))
        .collect();
    let header = Row::new(header_cells)
        .height(1)
        .style(Style::default().bg(theme.grid_header_bg));

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
                .map(|val| {
                    if val == "NULL" {
                        Cell::from(Text::from("NULL")).style(theme.null_style())
                    } else if val.parse::<f64>().is_ok() {
                        Cell::from(Text::from(val.as_str()))
                            .style(Style::default().fg(theme.grid_number))
                    } else {
                        Cell::from(Text::from(val.as_str()))
                    }
                })
                .collect();
            Row::new(cells).style(row_style)
        })
        .collect();

    let mut table_state = TableState::default();
    if total_rows > 0 && tab.grid_selected_row >= tab.grid_scroll_row {
        let vis_sel = tab.grid_selected_row - tab.grid_scroll_row;
        if vis_sel < visible_height {
            table_state.select(Some(vis_sel));
        }
    }

    let table = Table::new(rows, &col_widths)
        .header(header)
        .block(block)
        .highlight_style(theme.grid_selected_style())
        .column_spacing(1);

    frame.render_stateful_widget(table, area, &mut table_state);
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
