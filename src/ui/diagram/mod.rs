//! View state of an entity-relationship diagram tab: which tables are
//! shown (the whole schema or a table with its neighbours), their boxes,
//! the selection and the viewport. Pure state: rendering lives in
//! `widgets::er_diagram`, keys in `events::diagram`.

pub mod canvas;

use std::collections::{HashMap, HashSet};

use crate::core::er::graph::ErGraph;
use crate::core::er::layout::{self, LayoutEdge, LayoutNode};
use crate::core::models::{Column, ForeignKeyInfo};

use canvas::Canvas;

/// Column rows shown per box before the rest is folded into one line.
pub const MAX_ROWS: usize = 12;
pub const MIN_BOX_WIDTH: u16 = 16;
pub const MAX_BOX_WIDTH: u16 = 42;
/// Cells a pan key moves the viewport.
pub const PAN_STEP_X: i32 = 4;
pub const PAN_STEP_Y: i32 = 2;
pub const MAX_DEPTH: usize = 4;

/// How much of each table a box shows: the zoom level of the diagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Detail {
    /// Name only, one row, no frame.
    Mini,
    /// Framed name.
    Names,
    /// Name plus key columns.
    Keys,
    /// Name plus every column.
    Full,
}

impl Detail {
    pub fn zoom_in(self) -> Self {
        match self {
            Detail::Mini => Detail::Names,
            Detail::Names => Detail::Keys,
            Detail::Keys | Detail::Full => Detail::Full,
        }
    }

    pub fn zoom_out(self) -> Self {
        match self {
            Detail::Full => Detail::Keys,
            Detail::Keys => Detail::Names,
            Detail::Names | Detail::Mini => Detail::Mini,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Detail::Mini => "mini",
            Detail::Names => "names",
            Detail::Keys => "keys",
            Detail::Full => "full",
        }
    }

    /// Space between columns and between stacked boxes: tighter when
    /// zoomed out, so more of the schema fits on screen.
    pub fn gaps(self) -> (i32, i32) {
        match self {
            Detail::Full => (10, 1),
            Detail::Keys => (8, 1),
            Detail::Names => (6, 1),
            Detail::Mini => (4, 0),
        }
    }

    /// Boxes need the table's columns at these levels.
    pub fn wants_columns(self) -> bool {
        matches!(self, Detail::Full | Detail::Keys)
    }

    /// The level a schema of `tables` tables opens at: every column while
    /// it fits comfortably, less as it grows.
    pub fn for_size(tables: usize) -> Self {
        match tables {
            0..=24 => Detail::Full,
            25..=60 => Detail::Keys,
            _ => Detail::Names,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    None,
    Primary,
    Foreign,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxRow {
    pub column: String,
    pub data_type: String,
    pub key: KeyKind,
}

#[derive(Debug, Clone)]
pub struct DiagramBox {
    pub node: usize,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
    pub rows: Vec<BoxRow>,
    /// Rows the box had no room for.
    pub hidden_rows: usize,
    /// Columns requested but not loaded yet.
    pub loading: bool,
    /// Table of another schema, drawn qualified.
    pub external: bool,
}

impl DiagramBox {
    /// Canvas row of the `index`th column line.
    pub fn row_y(&self, index: usize) -> i32 {
        self.y + 3 + index as i32
    }

    /// Row an edge attaches to when its column is not on display.
    pub fn middle_y(&self) -> i32 {
        if self.height == 1 {
            self.y
        } else if self.rows.is_empty() {
            self.y + 1
        } else {
            self.y + self.height as i32 / 2
        }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }
}

#[derive(Debug, Clone)]
pub struct DiagramEdge {
    /// Box indices.
    pub from: usize,
    pub to: usize,
    /// Canvas rows the edge leaves from and arrives at.
    pub from_y: i32,
    pub to_y: i32,
}

#[derive(Debug, Clone, Default)]
pub struct DiagramLayout {
    /// Reading order: column by column, top to bottom.
    pub boxes: Vec<DiagramBox>,
    pub edges: Vec<DiagramEdge>,
    pub width: i32,
    pub height: i32,
}

impl DiagramLayout {
    pub fn box_index(&self, name: &str) -> Option<usize> {
        self.boxes.iter().position(|b| b.name == name)
    }

    /// Boxes joined to `index` by an edge.
    pub fn neighbours(&self, index: usize) -> HashSet<usize> {
        self.edges
            .iter()
            .filter_map(|e| {
                if e.from == index {
                    Some(e.to)
                } else if e.to == index {
                    Some(e.from)
                } else {
                    None
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollRequest {
    /// Bring the selected box into view, moving as little as possible.
    Reveal,
    /// Put the selected box in the middle of the viewport.
    Center,
}

pub struct DiagramState {
    pub schema: String,
    pub tables: Vec<String>,
    pub fks: Vec<ForeignKeyInfo>,
    pub columns: HashMap<String, Vec<Column>>,
    pub pending_columns: HashSet<String>,
    /// Tables and keys have arrived.
    pub loaded: bool,
    /// Error of the last load, shown instead of the diagram.
    pub error: Option<String>,
    /// Table at the centre when only its neighbourhood is drawn.
    pub focus: Option<String>,
    pub depth: usize,
    pub detail: Detail,
    pub selected: Option<String>,
    pub scroll_x: i32,
    pub scroll_y: i32,
    /// Inner size of the last frame, for paging and clamping.
    pub viewport: (u16, u16),
    pub scroll_request: Option<ScrollRequest>,
    /// Search text while `/` is being typed.
    pub search: Option<String>,
    /// `z` was pressed; the next key picks zoom in, zoom out or centre.
    pub z_pending: bool,
    pub last_search: String,
    graph: ErGraph,
    layout: Option<DiagramLayout>,
    canvas: Option<Canvas>,
}

impl DiagramState {
    pub fn new(schema: String, focus: Option<String>) -> Self {
        Self {
            schema,
            tables: Vec::new(),
            fks: Vec::new(),
            columns: HashMap::new(),
            pending_columns: HashSet::new(),
            loaded: false,
            error: None,
            selected: focus.clone(),
            focus,
            depth: 1,
            detail: Detail::Full,
            scroll_x: 0,
            scroll_y: 0,
            viewport: (0, 0),
            scroll_request: Some(ScrollRequest::Center),
            search: None,
            z_pending: false,
            last_search: String::new(),
            graph: ErGraph::default(),
            layout: None,
            canvas: None,
        }
    }

    /// Forget everything loaded so the next load starts clean.
    pub fn reset(&mut self) {
        self.tables.clear();
        self.fks.clear();
        self.columns.clear();
        self.pending_columns.clear();
        self.loaded = false;
        self.error = None;
        self.graph = ErGraph::default();
        self.invalidate();
    }

    pub fn set_data(&mut self, tables: Vec<String>, fks: Vec<ForeignKeyInfo>) {
        self.graph = ErGraph::build(&self.schema, &tables, &fks);
        self.tables = tables;
        self.fks = fks;
        self.loaded = true;
        self.error = None;
        // A focus on a table that is not there falls back to the schema.
        if let Some(focus) = &self.focus
            && self.graph.node_id(focus).is_none()
        {
            self.focus = None;
        }
        // The whole schema is the opening view; the zoom level starts
        // where the schema's size still reads well. A focused opening
        // (from a table) keeps every column.
        self.detail = if self.focus.is_some() {
            Detail::Full
        } else {
            Detail::for_size(self.graph.nodes.len())
        };
        if self
            .selected
            .as_ref()
            .is_none_or(|s| self.graph.node_id(s).is_none())
        {
            self.selected = self
                .focus
                .clone()
                .or_else(|| self.graph.nodes.first().cloned());
        }
        self.scroll_request = Some(ScrollRequest::Center);
        self.invalidate();
    }

    pub fn set_columns(&mut self, table: String, columns: Vec<Column>) {
        self.pending_columns.remove(&table);
        self.columns.insert(table, columns);
        self.invalidate();
    }

    pub fn graph(&self) -> &ErGraph {
        &self.graph
    }

    fn invalidate(&mut self) {
        self.layout = None;
        self.canvas = None;
    }

    /// Nodes on display with the column each wants, `None` when the
    /// layout decides.
    fn visible_nodes(&self) -> Vec<(usize, Option<i32>)> {
        if let Some(focus) = self.focus.as_ref().and_then(|f| self.graph.node_id(f)) {
            return self
                .graph
                .neighborhood(focus, self.depth)
                .into_iter()
                .map(|(n, layer)| (n, Some(layer)))
                .collect();
        }
        (0..self.graph.nodes.len()).map(|n| (n, None)).collect()
    }

    /// Tables whose columns the current view needs and nobody fetched yet.
    pub fn tables_needing_columns(&self) -> Vec<String> {
        if !self.loaded || !self.detail.wants_columns() {
            return Vec::new();
        }
        self.visible_nodes()
            .into_iter()
            .map(|(n, _)| &self.graph.nodes[n])
            .filter(|name| {
                !name.contains('.')
                    && !self.columns.contains_key(*name)
                    && !self.pending_columns.contains(*name)
            })
            .cloned()
            .collect()
    }

    pub fn layout(&mut self) -> &DiagramLayout {
        if self.layout.is_none() {
            self.layout = Some(self.build_layout());
        }
        self.layout.get_or_insert_with(DiagramLayout::default)
    }

    pub fn canvas(&mut self) -> &Canvas {
        if self.canvas.is_none() {
            let layout = self.layout().clone();
            self.canvas = Some(Canvas::draw(&layout));
        }
        self.canvas.get_or_insert_with(Canvas::default)
    }

    fn build_layout(&self) -> DiagramLayout {
        let visible = self.visible_nodes();
        let mut boxes: Vec<DiagramBox> = visible
            .iter()
            .map(|&(node, _)| self.make_box(node))
            .collect();
        let nodes: Vec<LayoutNode> = boxes
            .iter()
            .zip(visible.iter())
            .map(|(b, &(_, layer))| LayoutNode {
                id: b.node,
                width: b.width,
                height: b.height,
                layer,
            })
            .collect();
        let graph_edges = self
            .graph
            .edges_within(&visible.iter().map(|(n, _)| *n).collect::<Vec<_>>());
        let edges: Vec<LayoutEdge> = graph_edges
            .iter()
            .map(|&e| LayoutEdge {
                from: self.graph.edges[e].from,
                to: self.graph.edges[e].to,
            })
            .collect();
        let (layer_gap, node_gap) = self.detail.gaps();
        let placed = layout::layout_with_gaps(&nodes, &edges, layer_gap, node_gap);

        // Reorder the boxes to the layout's reading order and position them.
        let mut by_node: HashMap<usize, DiagramBox> =
            boxes.drain(..).map(|b| (b.node, b)).collect();
        let mut ordered = Vec::with_capacity(placed.nodes.len());
        for p in &placed.nodes {
            if let Some(mut b) = by_node.remove(&p.id) {
                b.x = p.x;
                b.y = p.y;
                ordered.push(b);
            }
        }
        let index_of: HashMap<usize, usize> = ordered
            .iter()
            .enumerate()
            .map(|(i, b)| (b.node, i))
            .collect();
        let edges = graph_edges
            .iter()
            .filter_map(|&e| {
                let edge = &self.graph.edges[e];
                let from = *index_of.get(&edge.from)?;
                let to = *index_of.get(&edge.to)?;
                let from_y = attach_row(&ordered[from], edge.rel.columns.first());
                let to_y = attach_row(&ordered[to], edge.rel.referenced_columns.first());
                Some(DiagramEdge {
                    from,
                    to,
                    from_y,
                    to_y,
                })
            })
            .collect();
        DiagramLayout {
            boxes: ordered,
            edges,
            width: placed.width,
            height: placed.height,
        }
    }

    fn make_box(&self, node: usize) -> DiagramBox {
        let name = self.graph.nodes[node].clone();
        let external = name.contains('.');
        let fk_columns: HashSet<&str> = self
            .graph
            .outgoing(node)
            .iter()
            .flat_map(|&e| self.graph.edges[e].rel.columns.iter().map(String::as_str))
            .collect();
        let (rows, hidden_rows, loading) = if !self.detail.wants_columns() || external {
            (Vec::new(), 0, false)
        } else if let Some(columns) = self.columns.get(&name) {
            let all: Vec<BoxRow> = columns
                .iter()
                .map(|c| BoxRow {
                    key: match (c.is_primary_key, fk_columns.contains(c.name.as_str())) {
                        (true, true) => KeyKind::Both,
                        (true, false) => KeyKind::Primary,
                        (false, true) => KeyKind::Foreign,
                        (false, false) => KeyKind::None,
                    },
                    column: c.name.clone(),
                    data_type: c.data_type.clone(),
                })
                .filter(|row| self.detail == Detail::Full || row.key != KeyKind::None)
                .collect();
            if all.len() > MAX_ROWS {
                let shown = MAX_ROWS - 1;
                let hidden = all.len() - shown;
                (all.into_iter().take(shown).collect(), hidden, false)
            } else {
                (all, 0, false)
            }
        } else {
            (Vec::new(), 0, true)
        };

        let text_width = rows
            .iter()
            .map(|r| r.column.chars().count() + 2 + r.data_type.chars().count() + 2)
            .chain(std::iter::once(name.chars().count() + 2))
            .max()
            .unwrap_or(0) as u16;
        let body_rows = rows.len() + usize::from(hidden_rows > 0) + usize::from(loading);
        let (width, height) = if self.detail == Detail::Mini {
            // A chip: `[name]`, one row.
            ((name.chars().count() as u16 + 2).min(MAX_BOX_WIDTH), 1)
        } else if body_rows == 0 {
            ((text_width + 2).clamp(MIN_BOX_WIDTH, MAX_BOX_WIDTH), 3)
        } else {
            (
                (text_width + 2).clamp(MIN_BOX_WIDTH, MAX_BOX_WIDTH),
                4 + body_rows as u16,
            )
        };
        DiagramBox {
            node,
            name,
            x: 0,
            y: 0,
            width,
            height,
            rows,
            hidden_rows,
            loading,
            external,
        }
    }

    // --- Selection and navigation ------------------------------------

    pub fn selected_box(&mut self) -> Option<usize> {
        let selected = self.selected.clone()?;
        self.layout().box_index(&selected)
    }

    pub fn select_offset(&mut self, delta: isize) {
        let count = self.layout().boxes.len();
        if count == 0 {
            return;
        }
        let current = self.selected_box().unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(count as isize) as usize;
        self.selected = Some(self.layout().boxes[next].name.clone());
        self.scroll_request = Some(ScrollRequest::Reveal);
    }

    /// Move the selection to the nearest table in direction `(dx, dy)`,
    /// one of the four axis directions. Boxes straight ahead win over
    /// boxes off to the side; nothing happens when no box lies that way.
    pub fn select_direction(&mut self, dx: i32, dy: i32) {
        let Some(current) = self.selected_box() else {
            self.select_first();
            return;
        };
        let layout = self.layout();
        let centre = |b: &DiagramBox| (b.x + b.width as i32 / 2, b.y + b.height as i32 / 2);
        let (cx, cy) = centre(&layout.boxes[current]);
        let best = layout
            .boxes
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != current)
            .filter_map(|(i, b)| {
                let (bx, by) = centre(b);
                let (ahead, aside) = if dx != 0 {
                    ((bx - cx) * dx.signum(), (by - cy).abs())
                } else {
                    ((by - cy) * dy.signum(), (bx - cx).abs())
                };
                // Terminal cells are twice as tall as wide: weigh the
                // vertical offset so the two axes feel alike.
                let (ahead, aside) = if dx != 0 {
                    (ahead, aside * 2)
                } else {
                    (ahead * 2, aside)
                };
                // Only boxes inside a wide cone ahead count: a table far
                // off to the side is not "up" just because it sits a few
                // rows higher.
                (ahead > 0 && aside <= ahead * 2).then_some((ahead + aside * 2, i))
            })
            .min_by_key(|(score, _)| *score);
        if let Some((_, index)) = best {
            self.selected = Some(layout.boxes[index].name.clone());
            self.scroll_request = Some(ScrollRequest::Reveal);
        }
    }

    pub fn select_first(&mut self) {
        if let Some(first) = self.layout().boxes.first() {
            self.selected = Some(first.name.clone());
            self.scroll_request = Some(ScrollRequest::Reveal);
        }
    }

    pub fn select_last(&mut self) {
        if let Some(last) = self.layout().boxes.last() {
            self.selected = Some(last.name.clone());
            self.scroll_request = Some(ScrollRequest::Reveal);
        }
    }

    /// Make the selected table the focus. Returns true when the set of
    /// visible tables may have changed.
    pub fn focus_selected(&mut self) -> bool {
        let Some(selected) = self.selected.clone() else {
            return false;
        };
        if self.focus.as_deref() == Some(selected.as_str()) {
            return false;
        }
        self.focus = Some(selected);
        self.scroll_request = Some(ScrollRequest::Center);
        self.invalidate();
        true
    }

    /// Switch between the focused neighbourhood and the whole schema.
    pub fn toggle_focus(&mut self) -> bool {
        if self.focus.is_some() {
            self.focus = None;
        } else {
            let Some(selected) = self.selected.clone() else {
                return false;
            };
            self.focus = Some(selected);
        }
        self.scroll_request = Some(ScrollRequest::Center);
        self.invalidate();
        true
    }

    pub fn change_depth(&mut self, delta: isize) -> bool {
        let next = (self.depth as isize + delta).clamp(1, MAX_DEPTH as isize) as usize;
        if next == self.depth || self.focus.is_none() {
            return false;
        }
        self.depth = next;
        self.scroll_request = Some(ScrollRequest::Center);
        self.invalidate();
        true
    }

    /// Show more (or less) of every table. Returns true when the boxes
    /// now need columns they did not need before.
    pub fn zoom(&mut self, closer: bool) -> bool {
        let next = if closer {
            self.detail.zoom_in()
        } else {
            self.detail.zoom_out()
        };
        if next == self.detail {
            return false;
        }
        let needs_more = next.wants_columns() && !self.detail.wants_columns();
        self.detail = next;
        self.scroll_request = Some(ScrollRequest::Center);
        self.invalidate();
        needs_more
    }

    pub fn pan(&mut self, dx: i32, dy: i32) {
        self.scroll_x += dx;
        self.scroll_y += dy;
        self.clamp_scroll();
    }

    pub fn half_page(&mut self, down: bool) {
        let step = (self.viewport.1 as i32 / 2).max(1);
        self.pan(0, if down { step } else { -step });
    }

    /// Keep the viewport inside the canvas (plus a one-cell margin).
    pub fn clamp_scroll(&mut self) {
        let (vw, vh) = (self.viewport.0 as i32, self.viewport.1 as i32);
        let (cw, ch) = {
            let c = self.canvas();
            (c.width as i32, c.height as i32)
        };
        self.scroll_x = self.scroll_x.clamp(0, (cw - vw).max(0));
        self.scroll_y = self.scroll_y.clamp(0, (ch - vh).max(0));
    }

    /// Apply a pending reveal/centre request against the current viewport.
    pub fn apply_scroll_request(&mut self) {
        let Some(request) = self.scroll_request.take() else {
            return;
        };
        let Some(index) = self.selected_box() else {
            return;
        };
        let (bx, by, bw, bh) = {
            let b = &self.layout().boxes[index];
            (b.x, b.y, b.width as i32, b.height as i32)
        };
        let (vw, vh) = (self.viewport.0 as i32, self.viewport.1 as i32);
        match request {
            ScrollRequest::Center => {
                self.scroll_x = bx + bw / 2 - vw / 2;
                self.scroll_y = by + bh / 2 - vh / 2;
            }
            ScrollRequest::Reveal => {
                if bx < self.scroll_x {
                    self.scroll_x = bx - 1;
                } else if bx + bw > self.scroll_x + vw {
                    self.scroll_x = bx + bw - vw + 1;
                }
                if by < self.scroll_y {
                    self.scroll_y = by - 1;
                } else if by + bh > self.scroll_y + vh {
                    self.scroll_y = by + bh - vh + 1;
                }
            }
        }
        self.clamp_scroll();
    }

    // --- Search ------------------------------------------------------

    /// Tables matching `query`, the visible ones first.
    pub fn matches(&mut self, query: &str) -> Vec<String> {
        let needle = query.to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let visible: Vec<String> = self
            .layout()
            .boxes
            .iter()
            .map(|b| b.name.clone())
            .filter(|n| n.to_lowercase().contains(&needle))
            .collect();
        let mut out = visible.clone();
        for name in &self.graph.nodes {
            if name.to_lowercase().contains(&needle) && !out.contains(name) {
                out.push(name.clone());
            }
        }
        out
    }

    /// Select `name`, refocusing when it is outside the drawn
    /// neighbourhood. Returns true when the visible set may have changed.
    pub fn jump_to(&mut self, name: &str) -> bool {
        if self.graph.node_id(name).is_none() {
            return false;
        }
        self.selected = Some(name.to_string());
        if self.layout().box_index(name).is_some() {
            self.scroll_request = Some(ScrollRequest::Reveal);
            return false;
        }
        self.focus = Some(name.to_string());
        self.scroll_request = Some(ScrollRequest::Center);
        self.invalidate();
        true
    }

    /// Next (or previous) match of the last search after the selection.
    pub fn jump_to_match(&mut self, forward: bool) -> bool {
        let query = self.last_search.clone();
        let found = self.matches(&query);
        if found.is_empty() {
            return false;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|s| found.iter().position(|f| f == s));
        let next = match (current, forward) {
            (Some(i), true) => (i + 1) % found.len(),
            (Some(i), false) => (i + found.len() - 1) % found.len(),
            (None, _) => 0,
        };
        let name = found[next].clone();
        self.jump_to(&name)
    }

    /// `(schema, table)` of the selected box, for opening it.
    pub fn selected_table(&self) -> Option<(String, String)> {
        let name = self.selected.clone()?;
        match name.rsplit_once('.') {
            Some((schema, table)) => Some((schema.to_string(), table.to_string())),
            None => Some((self.schema.clone(), name)),
        }
    }

    /// Short description for the status line.
    pub fn summary(&self) -> String {
        let tables = self.graph.nodes.iter().filter(|n| !n.contains('.')).count();
        let relations = self.graph.edges.len();
        let zoom = self.detail.label();
        match &self.focus {
            Some(focus) => format!(
                "{focus} \u{00b7} depth {} \u{00b7} {tables} tables \u{00b7} {relations} fks \u{00b7} zoom {zoom}",
                self.depth
            ),
            None => format!("{tables} tables \u{00b7} {relations} fks \u{00b7} zoom {zoom}"),
        }
    }
}

/// Canvas row an edge attaches to on `b`: the row of `column` when shown,
/// the middle of the box otherwise.
fn attach_row(b: &DiagramBox, column: Option<&String>) -> i32 {
    column
        .and_then(|c| b.rows.iter().position(|r| &r.column == c))
        .map(|i| b.row_y(i))
        .unwrap_or_else(|| b.middle_y())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fk(table: &str, col: &str, ref_table: &str) -> ForeignKeyInfo {
        ForeignKeyInfo {
            constraint_name: format!("fk_{table}_{col}"),
            table_name: table.into(),
            column_name: col.into(),
            referenced_schema: "app".into(),
            referenced_table: ref_table.into(),
            referenced_column: "id".into(),
        }
    }

    fn col(name: &str, pk: bool) -> Column {
        Column {
            name: name.into(),
            data_type: "integer".into(),
            nullable: !pk,
            is_primary_key: pk,
        }
    }

    fn shop(focus: Option<&str>) -> DiagramState {
        let mut d = DiagramState::new("app".into(), focus.map(str::to_string));
        let tables = ["users", "orders", "order_items", "products", "logs"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        d.set_data(
            tables,
            vec![
                fk("orders", "user_id", "users"),
                fk("order_items", "order_id", "orders"),
                fk("order_items", "product_id", "products"),
            ],
        );
        d.viewport = (80, 24);
        d
    }

    #[test]
    fn small_schema_opens_whole_with_columns() {
        let mut d = shop(None);
        assert!(d.focus.is_none());
        assert_eq!(d.detail, Detail::Full);
        assert_eq!(d.layout().boxes.len(), 5);
        assert_eq!(d.layout().edges.len(), 3);
        let mut needed = d.tables_needing_columns();
        needed.sort();
        assert_eq!(
            needed,
            ["logs", "order_items", "orders", "products", "users"]
        );
    }

    #[test]
    fn focus_limits_the_boxes_and_the_column_requests() {
        let mut d = shop(Some("orders"));
        let mut names: Vec<String> = d.layout().boxes.iter().map(|b| b.name.clone()).collect();
        names.sort();
        assert_eq!(names, ["order_items", "orders", "users"]);
        assert_eq!(d.layout().edges.len(), 2);
        let mut needed = d.tables_needing_columns();
        needed.sort();
        assert_eq!(needed, ["order_items", "orders", "users"]);

        d.pending_columns.insert("users".into());
        d.set_columns(
            "orders".into(),
            vec![col("id", true), col("user_id", false)],
        );
        assert_eq!(d.tables_needing_columns(), ["order_items"]);
    }

    #[test]
    fn edges_attach_to_the_column_rows_once_loaded() {
        let mut d = shop(Some("orders"));
        d.set_columns(
            "orders".into(),
            vec![col("id", true), col("user_id", false)],
        );
        let layout = d.layout().clone();
        let orders = layout.box_index("orders").expect("orders box");
        let b = &layout.boxes[orders];
        assert_eq!(b.rows[0].key, KeyKind::Primary);
        assert_eq!(b.rows[1].key, KeyKind::Foreign);
        assert_eq!(b.height, 6);
        let to_users = layout
            .edges
            .iter()
            .find(|e| layout.boxes[e.to].name == "users")
            .expect("orders -> users");
        assert_eq!(to_users.from_y, b.row_y(1), "leaves from the user_id row");
        let from_items = layout
            .edges
            .iter()
            .find(|e| layout.boxes[e.from].name == "order_items")
            .expect("order_items -> orders");
        assert_eq!(from_items.to_y, b.row_y(0), "arrives at the id row");
    }

    #[test]
    fn hjkl_select_the_nearest_table_in_that_direction() {
        let mut d = shop(None);
        // Columns: users and products (referenced), orders, order_items, logs.
        assert_eq!(d.selected.as_deref(), Some("users"));
        d.select_direction(1, 0);
        assert_eq!(d.selected.as_deref(), Some("orders"));
        d.select_direction(1, 0);
        assert_eq!(d.selected.as_deref(), Some("order_items"));
        d.select_direction(-1, 0);
        assert_eq!(d.selected.as_deref(), Some("orders"));
        d.select_direction(-1, 0);
        assert_eq!(d.selected.as_deref(), Some("users"));
        d.select_direction(0, 1);
        assert_eq!(
            d.selected.as_deref(),
            Some("products"),
            "below users in the same column"
        );
        d.select_direction(0, -1);
        assert_eq!(d.selected.as_deref(), Some("users"));
        d.select_direction(0, -1);
        assert_eq!(d.selected.as_deref(), Some("users"), "nothing above: stays");
    }

    #[test]
    fn selection_wraps_and_jump_refocuses_hidden_tables() {
        let mut d = shop(Some("orders"));
        assert_eq!(d.selected.as_deref(), Some("orders"));
        for _ in 0..3 {
            d.select_offset(1);
        }
        assert_eq!(
            d.selected.as_deref(),
            Some("orders"),
            "three steps wrap around three boxes"
        );

        assert!(!d.jump_to("users"), "users is already drawn");
        assert_eq!(d.focus.as_deref(), Some("orders"));
        assert!(d.jump_to("logs"), "logs is outside the neighbourhood");
        assert_eq!(d.focus.as_deref(), Some("logs"));
        assert_eq!(d.selected_table(), Some(("app".into(), "logs".into())));
    }

    #[test]
    fn search_lists_visible_matches_first() {
        let mut d = shop(Some("orders"));
        assert_eq!(d.matches("order"), ["orders", "order_items"]);
        assert_eq!(d.matches("prod"), ["products"]);
        assert!(d.matches("").is_empty());
        d.last_search = "o".into();
        assert!(d.jump_to_match(true) || d.selected.is_some());
    }

    #[test]
    fn depth_only_changes_in_focus_mode() {
        let mut d = shop(None);
        assert!(!d.change_depth(1));
        assert!(d.toggle_focus());
        assert!(d.change_depth(1));
        assert_eq!(d.depth, 2);
        assert!(d.change_depth(10));
        assert_eq!(d.depth, MAX_DEPTH, "clamped");
        assert!(!d.change_depth(1), "already at the maximum");
    }

    #[test]
    fn scroll_stays_inside_the_canvas() {
        let mut d = shop(None);
        d.viewport = (40, 10);
        d.pan(-100, -100);
        assert_eq!((d.scroll_x, d.scroll_y), (0, 0));
        d.pan(10_000, 10_000);
        let c = d.canvas();
        let (w, h) = (c.width as i32, c.height as i32);
        assert_eq!((d.scroll_x, d.scroll_y), ((w - 40).max(0), (h - 10).max(0)));
        d.scroll_request = Some(ScrollRequest::Center);
        d.apply_scroll_request();
        assert!(d.scroll_request.is_none());
    }

    #[test]
    fn zoom_levels_change_box_shapes_and_column_needs() {
        let mut d = shop(None);
        d.set_columns(
            "orders".into(),
            vec![col("id", true), col("user_id", false)],
        );
        d.set_columns("users".into(), vec![col("id", true), col("name", false)]);
        let height = |d: &mut DiagramState, name: &str| {
            let i = d.layout().box_index(name).expect(name);
            d.layout().boxes[i].height
        };
        assert_eq!(height(&mut d, "orders"), 6);
        assert!(!d.zoom(false), "keys still want columns");
        assert_eq!(d.detail, Detail::Keys);
        assert_eq!(height(&mut d, "orders"), 6, "both columns are keys");
        assert_eq!(height(&mut d, "users"), 5, "name is not a key");
        d.zoom(false);
        assert_eq!(d.detail, Detail::Names);
        assert_eq!(height(&mut d, "orders"), 3);
        assert!(d.tables_needing_columns().is_empty());
        d.zoom(false);
        assert_eq!(d.detail, Detail::Mini);
        assert_eq!(height(&mut d, "orders"), 1);
        assert!(!d.zoom(false), "already at the smallest");
        assert!(!d.zoom(true), "names need no columns");
        assert!(d.zoom(true), "keys need columns again");
        assert_eq!(d.detail, Detail::Keys);
        assert_eq!(Detail::for_size(100), Detail::Names);
    }
}
