//! View state of an entity-relationship diagram tab: which tables are
//! shown (the whole schema or a table with its neighbours), their boxes,
//! the selection and the viewport. Pure state: rendering lives in
//! `widgets::er_diagram`, keys in `events::diagram`.

pub mod canvas;
mod navigate;
#[cfg(test)]
mod tests;

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
    /// When the current load started; `None` while nothing is in flight.
    /// Kept apart from the tab's own streaming state so a table's data
    /// load and its Relations diagram never confuse each other.
    pub loading_since: Option<std::time::Instant>,
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
    /// Focus and selection when the search opened, restored on Esc so a
    /// live preview that refocused the diagram leaves no trace.
    pub search_origin: Option<(Option<String>, Option<String>)>,
    /// `z` was pressed; the next key picks zoom in, zoom out or centre.
    pub z_pending: bool,
    /// Row of the selected box the cursor is on, while walking its
    /// columns. `None` while the selection is the table as a whole.
    pub column_cursor: Option<usize>,
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
            loading_since: None,
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
            search_origin: None,
            z_pending: false,
            column_cursor: None,
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
        self.loading_since = None;
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
        self.column_cursor = None;
    }

    /// Select `name` as a whole table, forgetting any column cursor.
    fn select(&mut self, name: String, request: ScrollRequest) {
        self.selected = Some(name);
        self.column_cursor = None;
        self.scroll_request = Some(request);
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
}

/// Canvas row an edge attaches to on `b`: the row of `column` when shown,
/// the middle of the box otherwise.
fn attach_row(b: &DiagramBox, column: Option<&String>) -> i32 {
    column
        .and_then(|c| b.rows.iter().position(|r| &r.column == c))
        .map(|i| b.row_y(i))
        .unwrap_or_else(|| b.middle_y())
}
