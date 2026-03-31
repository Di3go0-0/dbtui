use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::Text;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::ui::state::{AppState, Panel};
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, state: &AppState, theme: &Theme, area: Rect) {
    let is_focused = state.active_panel == Panel::Properties;
    let border_style = theme.border_style(is_focused, &state.mode);

    let block = Block::default()
        .title(" Properties ")
        .borders(Borders::ALL)
        .border_style(border_style);

    if state.columns.is_empty() {
        let empty_rows: Vec<Row> = vec![];
        let empty = Table::new(empty_rows, &[Constraint::Min(1)]).block(block);
        frame.render_widget(empty, area);
        return;
    }

    let header_cells = vec![
        Cell::from(Text::from("Column")).style(theme.grid_header_style()),
        Cell::from(Text::from("Type")).style(theme.grid_header_style()),
        Cell::from(Text::from("Nullable")).style(theme.grid_header_style()),
        Cell::from(Text::from("PK")).style(theme.grid_header_style()),
    ];
    let header = Row::new(header_cells).height(1);

    let rows: Vec<Row> = state
        .columns
        .iter()
        .map(|col| {
            Row::new(vec![
                Cell::from(Text::from(col.name.as_str())),
                Cell::from(Text::from(col.data_type.as_str())),
                Cell::from(Text::from(if col.nullable { "YES" } else { "NO" })),
                Cell::from(Text::from(if col.is_primary_key {
                    "✓"
                } else {
                    ""
                }))
                .style(if col.is_primary_key {
                    Style::default().fg(theme.conn_connected)
                } else {
                    Style::default()
                }),
            ])
        })
        .collect();

    let widths = vec![
        Constraint::Percentage(35),
        Constraint::Percentage(30),
        Constraint::Percentage(15),
        Constraint::Percentage(10),
    ];

    let table = Table::new(rows, &widths)
        .header(header)
        .block(block)
        .column_spacing(1);

    frame.render_widget(table, area);
}
