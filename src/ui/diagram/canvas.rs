//! Draws a diagram layout into a grid of characters: edges first, as
//! orthogonal lines that merge where they meet, then the boxes on top so
//! a line passing behind a table disappears under it.

use std::collections::HashMap;

use unicode_width::UnicodeWidthChar;

use super::{DiagramBox, DiagramLayout, KeyKind};

/// What a cell belongs to, so the renderer can colour it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// Box index in the layout.
    Node(usize),
    /// Edge index in the layout.
    Edge(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Border,
    Title,
    Key(KeyKind),
    ColumnName,
    ColumnType,
    Note,
    Line,
    Arrow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub owner: Owner,
    pub part: Part,
}

#[derive(Debug, Clone, Default)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    cells: Vec<Option<Cell>>,
}

/// Room kept around the layout for the lines that leave it.
const MARGIN_RIGHT: i32 = 10;
const MARGIN: i32 = 1;
/// Distinct vertical channels inside one column gap before they repeat.
const CHANNELS: i32 = 7;

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

impl Canvas {
    pub fn get(&self, x: i32, y: i32) -> Option<&Cell> {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return None;
        }
        self.cells[y as usize * self.width + x as usize].as_ref()
    }

    fn put(&mut self, x: i32, y: i32, cell: Cell) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        self.cells[y as usize * self.width + x as usize] = Some(cell);
    }

    pub fn draw(layout: &DiagramLayout) -> Self {
        if layout.boxes.is_empty() {
            return Self::default();
        }
        let width = (layout.width + MARGIN_RIGHT + MARGIN) as usize;
        let height = (layout.height + MARGIN * 2) as usize;
        let mut canvas = Self {
            width,
            height,
            cells: vec![None; width * height],
        };
        let mut lines = Lines::new(width, height);
        let mut channels: HashMap<i32, i32> = HashMap::new();
        for index in 0..layout.edges.len() {
            route(&mut lines, &mut channels, layout, index);
        }
        lines.resolve(&mut canvas);
        for (index, edge) in layout.edges.iter().enumerate() {
            let (x, y, ch) = arrow(layout, edge);
            canvas.put(
                x + MARGIN,
                y + MARGIN,
                Cell {
                    ch,
                    owner: Owner::Edge(index),
                    part: Part::Arrow,
                },
            );
        }
        for (index, b) in layout.boxes.iter().enumerate() {
            canvas.draw_box(index, b);
        }
        canvas
    }

    fn draw_box(&mut self, index: usize, b: &DiagramBox) {
        let owner = Owner::Node(index);
        let x0 = b.x + MARGIN;
        let y0 = b.y + MARGIN;
        let w = b.width as i32;
        let h = b.height as i32;
        let x1 = x0 + w - 1;
        let y1 = y0 + h - 1;
        let border = |ch| Cell {
            ch,
            owner,
            part: Part::Border,
        };
        if h == 1 {
            // Zoomed all the way out: `[name]` on one row.
            self.put(x0, y0, border('['));
            self.put(x1, y0, border(']'));
            let inner = (w - 2).max(0) as usize;
            self.write(x0 + 1, y0, &fit(&b.name, inner), owner, Part::Title);
            return;
        }
        for x in x0..=x1 {
            self.put(x, y0, border('\u{2500}'));
            self.put(x, y1, border('\u{2500}'));
        }
        for y in y0..=y1 {
            self.put(x0, y, border('\u{2502}'));
            self.put(x1, y, border('\u{2502}'));
        }
        self.put(x0, y0, border('\u{250c}'));
        self.put(x1, y0, border('\u{2510}'));
        self.put(x0, y1, border('\u{2514}'));
        self.put(x1, y1, border('\u{2518}'));

        let inner = (w - 2).max(0) as usize;
        for x in (x0 + 1)..x1 {
            self.put(
                x,
                y0 + 1,
                Cell {
                    ch: ' ',
                    owner,
                    part: Part::Title,
                },
            );
        }
        self.write(x0 + 1, y0 + 1, &fit(&b.name, inner), owner, Part::Title);

        let body_rows = b.rows.len() + usize::from(b.hidden_rows > 0) + usize::from(b.loading);
        if body_rows == 0 {
            return;
        }
        self.put(x0, y0 + 2, border('\u{251c}'));
        self.put(x1, y0 + 2, border('\u{2524}'));
        for x in (x0 + 1)..x1 {
            self.put(x, y0 + 2, border('\u{2500}'));
        }

        let mut y = y0 + 3;
        for row in &b.rows {
            for x in (x0 + 1)..x1 {
                self.put(
                    x,
                    y,
                    Cell {
                        ch: ' ',
                        owner,
                        part: Part::ColumnName,
                    },
                );
            }
            let glyph = match row.key {
                KeyKind::None => ' ',
                KeyKind::Primary => '\u{25cf}',
                KeyKind::Foreign => '\u{25cb}',
                KeyKind::Both => '\u{25c9}',
            };
            self.put(
                x0 + 1,
                y,
                Cell {
                    ch: glyph,
                    owner,
                    part: Part::Key(row.key),
                },
            );
            let name_room = inner.saturating_sub(2);
            let name = fit(&row.column, name_room);
            self.write(x0 + 3, y, &name, owner, Part::ColumnName);
            let used = 2 + name.chars().count();
            let type_room = inner.saturating_sub(used + 1);
            if type_room >= 3 {
                let text = fit(&row.data_type, type_room);
                let start = x1 - text.chars().count() as i32;
                self.write(start, y, &text, owner, Part::ColumnType);
            }
            y += 1;
        }
        if b.hidden_rows > 0 {
            let note = fit(
                &format!("\u{2026} {} more", b.hidden_rows),
                inner.saturating_sub(2),
            );
            self.write(x0 + 3, y, &note, owner, Part::Note);
        } else if b.loading {
            self.write(x0 + 3, y, "\u{2026}", owner, Part::Note);
        }
    }

    fn write(&mut self, x: i32, y: i32, text: &str, owner: Owner, part: Part) {
        for (i, ch) in text.chars().enumerate() {
            self.put(x + i as i32, y, Cell { ch, owner, part });
        }
    }

    /// The canvas as text, one line per row; cells nobody drew are spaces.
    #[cfg(test)]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for y in 0..self.height {
            let line: String = (0..self.width)
                .map(|x| self.cells[y * self.width + x].map(|c| c.ch).unwrap_or(' '))
                .collect();
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }
}

/// Clip `text` to `width` cells, marking the cut with an ellipsis. Characters
/// that are not exactly one cell wide are replaced so columns stay aligned.
pub fn fit(text: &str, width: usize) -> String {
    let chars: Vec<char> = text
        .chars()
        .map(|c| if c.width() == Some(1) { c } else { '?' })
        .collect();
    if chars.len() <= width {
        return chars.into_iter().collect();
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = chars[..width - 1].iter().collect();
    out.push('\u{2026}');
    out
}

/// Line segments as direction masks, merged into box-drawing characters
/// at the end.
struct Lines {
    width: usize,
    height: usize,
    mask: Vec<u8>,
    owner: Vec<Option<usize>>,
}

impl Lines {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            mask: vec![0; width * height],
            owner: vec![None; width * height],
        }
    }

    fn add(&mut self, x: i32, y: i32, bits: u8, edge: usize) {
        let (x, y) = (x + MARGIN, y + MARGIN);
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let i = y as usize * self.width + x as usize;
        self.mask[i] |= bits;
        if self.owner[i].is_none() {
            self.owner[i] = Some(edge);
        }
    }

    fn horizontal(&mut self, y: i32, x_a: i32, x_b: i32, edge: usize) {
        let (lo, hi) = (x_a.min(x_b), x_a.max(x_b));
        for x in lo..=hi {
            let mut bits = 0;
            if x > lo {
                bits |= LEFT;
            }
            if x < hi {
                bits |= RIGHT;
            }
            if bits == 0 {
                bits = LEFT | RIGHT;
            }
            self.add(x, y, bits, edge);
        }
    }

    /// A zero-length run adds nothing: the horizontal segments already
    /// meet in that cell.
    fn vertical(&mut self, x: i32, y_a: i32, y_b: i32, edge: usize) {
        let (lo, hi) = (y_a.min(y_b), y_a.max(y_b));
        if lo == hi {
            return;
        }
        for y in lo..=hi {
            let mut bits = 0;
            if y > lo {
                bits |= UP;
            }
            if y < hi {
                bits |= DOWN;
            }
            self.add(x, y, bits, edge);
        }
    }

    fn resolve(self, canvas: &mut Canvas) {
        for (i, &mask) in self.mask.iter().enumerate() {
            if mask == 0 {
                continue;
            }
            let Some(edge) = self.owner[i] else {
                continue;
            };
            let ch = match mask {
                m if m == UP || m == DOWN || m == UP | DOWN => '\u{2502}',
                m if m == LEFT || m == RIGHT || m == LEFT | RIGHT => '\u{2500}',
                m if m == DOWN | RIGHT => '\u{250c}',
                m if m == DOWN | LEFT => '\u{2510}',
                m if m == UP | RIGHT => '\u{2514}',
                m if m == UP | LEFT => '\u{2518}',
                m if m == UP | DOWN | RIGHT => '\u{251c}',
                m if m == UP | DOWN | LEFT => '\u{2524}',
                m if m == LEFT | RIGHT | DOWN => '\u{252c}',
                m if m == LEFT | RIGHT | UP => '\u{2534}',
                _ => '\u{253c}',
            };
            let x = (i % self.width) as i32;
            let y = (i / self.width) as i32;
            canvas.put(
                x,
                y,
                Cell {
                    ch,
                    owner: Owner::Edge(edge),
                    part: Part::Line,
                },
            );
        }
    }
}

/// Which side of the child the edge leaves from and which side of the
/// parent it enters.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Route {
    /// Parent fully to the left: leave left, enter the parent's right.
    Leftward,
    /// Parent fully to the right: leave right, enter the parent's left.
    Rightward,
    /// Same column (or a self reference): loop on the right.
    Loop,
}

fn route_kind(from: &DiagramBox, to: &DiagramBox) -> Route {
    if to.right() < from.x {
        Route::Leftward
    } else if from.right() < to.x {
        Route::Rightward
    } else {
        Route::Loop
    }
}

/// Lay the edge's three segments: out of the child, along a vertical
/// channel in the gap, into the parent. Each gap hands out a different
/// channel per edge so parallel edges do not pile onto one line.
fn route(
    lines: &mut Lines,
    channels: &mut HashMap<i32, i32>,
    layout: &DiagramLayout,
    index: usize,
) {
    let edge = &layout.edges[index];
    let from = &layout.boxes[edge.from];
    let to = &layout.boxes[edge.to];
    let (y0, y1) = (edge.from_y, edge.to_y);
    let next_channel = |channels: &mut HashMap<i32, i32>, key: i32| {
        let slot = channels.entry(key).or_insert(0);
        let k = *slot;
        *slot += 1;
        k % CHANNELS
    };
    match route_kind(from, to) {
        Route::Leftward => {
            let x0 = from.x - 1;
            let x1 = to.right() + 1;
            let channel = from.x - 2 - next_channel(channels, from.x);
            let channel = channel.max(x1 + 1);
            lines.horizontal(y0, x0, channel, index);
            lines.vertical(channel, y0, y1, index);
            lines.horizontal(y1, channel, x1, index);
        }
        Route::Rightward => {
            let x0 = from.right();
            let x1 = to.x - 2;
            let channel = to.x - 2 - next_channel(channels, to.x);
            let channel = channel.max(x0 + 1).min(x1);
            lines.horizontal(y0, x0, channel, index);
            lines.vertical(channel, y0, y1, index);
            lines.horizontal(y1, channel, x1, index);
        }
        Route::Loop => {
            let wall = from.right().max(to.right());
            let channel = wall + 2 + next_channel(channels, -wall);
            let (y0, y1) = if edge.from == edge.to && y0 == y1 {
                (y0, y0 + 1)
            } else {
                (y0, y1)
            };
            lines.horizontal(y0, from.right(), channel, index);
            lines.vertical(channel, y0, y1, index);
            lines.horizontal(y1, channel, to.right() + 1, index);
        }
    }
}

/// Position and glyph of the arrow head, just outside the parent's border.
fn arrow(layout: &DiagramLayout, edge: &super::DiagramEdge) -> (i32, i32, char) {
    let from = &layout.boxes[edge.from];
    let to = &layout.boxes[edge.to];
    let y = if edge.from == edge.to && edge.from_y == edge.to_y {
        edge.to_y + 1
    } else {
        edge.to_y
    };
    match route_kind(from, to) {
        Route::Rightward => (to.x - 1, y, '\u{25ba}'),
        Route::Leftward | Route::Loop => (to.right(), y, '\u{25c4}'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::diagram::{BoxRow, DiagramEdge};

    fn bx(node: usize, name: &str, x: i32, y: i32, rows: Vec<BoxRow>) -> DiagramBox {
        let height = if rows.is_empty() {
            3
        } else {
            4 + rows.len() as u16
        };
        DiagramBox {
            node,
            name: name.into(),
            x,
            y,
            width: if name == "orders" { 16 } else { 14 },
            height,
            rows,
            hidden_rows: 0,
            loading: false,
            external: false,
        }
    }

    fn row(column: &str, key: KeyKind) -> BoxRow {
        BoxRow {
            column: column.into(),
            data_type: "int".into(),
            key,
        }
    }

    #[test]
    fn two_boxes_and_one_edge() {
        let users = bx(0, "users", 0, 0, vec![row("id", KeyKind::Primary)]);
        let orders = bx(1, "orders", 24, 0, vec![row("user_id", KeyKind::Foreign)]);
        let layout = DiagramLayout {
            edges: vec![DiagramEdge {
                from: 1,
                to: 0,
                from_y: orders.row_y(0),
                to_y: users.row_y(0),
            }],
            boxes: vec![users, orders],
            width: 40,
            height: 5,
        };
        let text = Canvas::draw(&layout).to_text();
        let expected = "
 ┌────────────┐          ┌──────────────┐
 │users       │          │orders        │
 ├────────────┤          ├──────────────┤
 │● id     int│◄─────────│○ user_id  int│
 └────────────┘          └──────────────┘

";
        assert_eq!(text, expected, "\n{text}");
        let canvas = Canvas::draw(&layout);
        let arrow = canvas.get(15, 4).expect("arrow cell");
        assert_eq!((arrow.part, arrow.owner), (Part::Arrow, Owner::Edge(0)));
        let line = canvas.get(18, 4).expect("line cell");
        assert_eq!(line.part, Part::Line);
        let title = canvas.get(2, 2).expect("title cell");
        assert_eq!(
            (title.ch, title.owner, title.part),
            ('u', Owner::Node(0), Part::Title)
        );
    }

    #[test]
    fn lines_merge_where_they_cross_and_bend() {
        let a = bx(0, "a", 0, 0, vec![]);
        let b = bx(1, "b", 0, 6, vec![]);
        let c = bx(2, "c", 30, 3, vec![]);
        let layout = DiagramLayout {
            edges: vec![
                DiagramEdge {
                    from: 2,
                    to: 0,
                    from_y: 4,
                    to_y: 1,
                },
                DiagramEdge {
                    from: 2,
                    to: 1,
                    from_y: 4,
                    to_y: 7,
                },
            ],
            boxes: vec![a, b, c],
            width: 44,
            height: 9,
        };
        let text = Canvas::draw(&layout).to_text();
        assert!(
            text.contains('\u{2518}') && text.contains('\u{250c}'),
            "bends:\n{text}"
        );
        assert!(text.contains('\u{2518}'), "\n{text}");
        assert_eq!(text.matches('\u{25c4}').count(), 2, "\n{text}");
    }

    #[test]
    fn self_reference_loops_on_the_right() {
        let emp = bx(
            0,
            "emp",
            0,
            0,
            vec![row("id", KeyKind::Primary), row("boss", KeyKind::Foreign)],
        );
        let layout = DiagramLayout {
            edges: vec![DiagramEdge {
                from: 0,
                to: 0,
                from_y: emp.row_y(1),
                to_y: emp.row_y(0),
            }],
            boxes: vec![emp],
            width: 14,
            height: 6,
        };
        let text = Canvas::draw(&layout).to_text();
        assert!(
            text.contains("\u{2502}\u{25c4}\u{2500}\u{2510}"),
            "\n{text}"
        );
        assert!(
            text.contains("\u{2502}\u{2500}\u{2500}\u{2518}"),
            "\n{text}"
        );
    }

    #[test]
    fn fit_clips_with_an_ellipsis_and_replaces_wide_chars() {
        assert_eq!(fit("orders", 10), "orders");
        assert_eq!(fit("order_items", 6), "order\u{2026}");
        assert_eq!(fit("\u{4f60}\u{597d}", 4), "??");
        assert_eq!(fit("abc", 0), "");
    }
}
