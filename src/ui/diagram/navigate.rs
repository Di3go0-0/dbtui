//! Moving around a diagram: the selection, a table's columns, focus and
//! depth, zoom, the viewport and search. Pure state changes; the keys
//! that drive them live in `events::diagram`.

use super::{BoxRow, DiagramBox, DiagramState, MAX_DEPTH, ScrollRequest, SearchHit, SearchKind};

impl DiagramState {
    // --- Columns of the selected table -------------------------------

    /// Step into the selected table's columns. False when it shows none.
    pub fn enter_columns(&mut self) -> bool {
        let Some(index) = self.selected_box() else {
            return false;
        };
        if self.layout().boxes[index].rows.is_empty() {
            return false;
        }
        self.column_cursor = Some(0);
        true
    }

    pub fn leave_columns(&mut self) {
        self.column_cursor = None;
    }

    pub fn move_column(&mut self, delta: isize) {
        let Some(cursor) = self.column_cursor else {
            return;
        };
        let Some(index) = self.selected_box() else {
            return;
        };
        let rows = self.layout().boxes[index].rows.len();
        if rows == 0 {
            return;
        }
        self.column_cursor = Some((cursor as isize + delta).rem_euclid(rows as isize) as usize);
    }

    /// The column under the cursor, with its key kind.
    pub fn current_column(&mut self) -> Option<BoxRow> {
        let cursor = self.column_cursor?;
        let index = self.selected_box()?;
        self.layout().boxes[index].rows.get(cursor).cloned()
    }

    /// On a foreign key column, select the table it references (and land
    /// on the referenced column when it is on display). Returns true when
    /// the visible set may have changed.
    pub fn follow_column(&mut self) -> bool {
        let Some(row) = self.current_column() else {
            return false;
        };
        let Some(name) = self.selected.clone() else {
            return false;
        };
        let Some(node) = self.graph.node_id(&name) else {
            return false;
        };
        let target = self
            .graph
            .outgoing(node)
            .iter()
            .map(|&e| &self.graph.edges[e])
            .find(|edge| edge.rel.columns.contains(&row.column))
            .map(|edge| {
                (
                    self.graph.nodes[edge.to].clone(),
                    edge.rel.referenced_columns.first().cloned(),
                )
            });
        let Some((target, referenced)) = target else {
            return false;
        };
        let changed = self.jump_to(&target);
        if let Some(column) = referenced
            && let Some(index) = self.selected_box()
        {
            self.column_cursor = self.layout().boxes[index]
                .rows
                .iter()
                .position(|r| r.column == column);
        }
        changed
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
        let name = self.layout().boxes[next].name.clone();
        self.select(name, ScrollRequest::Reveal);
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
            let name = layout.boxes[index].name.clone();
            self.select(name, ScrollRequest::Reveal);
        }
    }

    pub fn select_first(&mut self) {
        if let Some(first) = self.layout().boxes.first().map(|b| b.name.clone()) {
            self.select(first, ScrollRequest::Reveal);
        }
    }

    pub fn select_last(&mut self) {
        if let Some(last) = self.layout().boxes.last().map(|b| b.name.clone()) {
            self.select(last, ScrollRequest::Reveal);
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

    /// Open the search prompt, remembering where the diagram stood. A
    /// column search also asks for every table's columns, so tables folded
    /// by the zoom can be searched too.
    pub fn start_search(&mut self, kind: SearchKind) {
        self.search = Some(String::new());
        self.search_kind = kind;
        self.last_hit = None;
        self.columns_everywhere = kind == SearchKind::Column;
        self.search_origin = Some((self.focus.clone(), self.selected.clone()));
    }

    /// Close the prompt without keeping its preview. Returns true when the
    /// visible set may have changed.
    pub fn cancel_search(&mut self) -> bool {
        self.search = None;
        self.last_hit = None;
        self.columns_everywhere = false;
        let Some((focus, selected)) = self.search_origin.take() else {
            return false;
        };
        let refocus = focus != self.focus;
        self.focus = focus;
        self.selected = selected;
        self.column_cursor = None;
        self.scroll_request = Some(ScrollRequest::Reveal);
        if refocus {
            self.invalidate();
        }
        refocus
    }

    /// Close the prompt keeping its result as the new search.
    pub fn accept_search(&mut self) -> bool {
        self.search_origin = None;
        self.columns_everywhere = false;
        let Some(query) = self.search.take() else {
            return false;
        };
        self.last_search = query.clone();
        // The live preview already landed on a match: keep it rather than
        // stepping past it to the next one.
        let found = self.matches(&query);
        if self.current_hit_index(&found).is_some() {
            return false;
        }
        self.jump_to_match(true)
    }

    /// Hits for `query` by the current search kind, the visible tables
    /// first; a column search lists one hit per matching column.
    pub fn matches(&mut self, query: &str) -> Vec<SearchHit> {
        let needle = query.to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let visible: Vec<String> = self.layout().boxes.iter().map(|b| b.name.clone()).collect();
        let mut order = visible;
        for name in &self.graph.nodes {
            if !order.contains(name) {
                order.push(name.clone());
            }
        }
        match self.search_kind {
            SearchKind::Table => order
                .into_iter()
                .filter(|n| n.to_lowercase().contains(&needle))
                .map(|table| SearchHit {
                    table,
                    column: None,
                })
                .collect(),
            SearchKind::Column => order
                .into_iter()
                .flat_map(|table| {
                    let columns = self.columns.get(&table).map(Vec::as_slice).unwrap_or(&[]);
                    columns
                        .iter()
                        .filter(|c| c.name.to_lowercase().contains(&needle))
                        .map(|c| SearchHit {
                            table: table.clone(),
                            column: Some(c.name.clone()),
                        })
                        .collect::<Vec<_>>()
                })
                .collect(),
        }
    }

    /// Index of the hit the selection (and column cursor) is on.
    fn current_hit_index(&mut self, found: &[SearchHit]) -> Option<usize> {
        let selected = self.selected.clone()?;
        // The hit the search last landed on, when the selection is still
        // there: a box folded by the zoom draws no column cursor, and the
        // cursor alone could not tell two matching columns apart.
        if let Some(last) = self.last_hit.clone()
            && last.table == selected
            && let Some(i) = found.iter().position(|h| *h == last)
        {
            return Some(i);
        }
        let column = self.current_column().map(|r| r.column);
        found
            .iter()
            .position(|h| h.table == selected && (h.column.is_none() || h.column == column))
            .or_else(|| found.iter().position(|h| h.table == selected))
    }

    /// Land on `hit`: select its table and, for a column hit, put the
    /// cursor on the column when the box shows it.
    fn jump_to_hit(&mut self, hit: &SearchHit) -> bool {
        let changed = self.jump_to(&hit.table);
        if let Some(column) = &hit.column
            && let Some(index) = self.selected_box()
        {
            self.column_cursor = self.layout().boxes[index]
                .rows
                .iter()
                .position(|r| &r.column == column);
        }
        self.last_hit = Some(hit.clone());
        changed
    }

    /// Select `name`, refocusing when it is outside the drawn
    /// neighbourhood. Returns true when the visible set may have changed.
    pub fn jump_to(&mut self, name: &str) -> bool {
        if self.graph.node_id(name).is_none() {
            return false;
        }
        self.select(name.to_string(), ScrollRequest::Reveal);
        if self.layout().box_index(name).is_some() {
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
        let current = self.current_hit_index(&found);
        let next = match (current, forward) {
            (Some(i), true) => (i + 1) % found.len(),
            (Some(i), false) => (i + found.len() - 1) % found.len(),
            (None, _) => 0,
        };
        let hit = found[next].clone();
        self.jump_to_hit(&hit)
    }

    /// First hit of `query`, as the prompt previews while typing.
    pub fn preview_search(&mut self, query: &str) -> bool {
        match self.matches(query).into_iter().next() {
            Some(hit) => self.jump_to_hit(&hit),
            None => {
                self.last_hit = None;
                false
            }
        }
    }

    /// `(schema, table)` of the selected box, for opening it.
    pub fn selected_table(&self) -> Option<(String, String)> {
        let name = self.selected.clone()?;
        let node = self.graph.node_id(&name)?;
        match self.graph.external(node) {
            Some((schema, table)) => Some((schema.clone(), table.clone())),
            None => Some((self.schema.clone(), name)),
        }
    }

    /// Short description for the status line.
    pub fn summary(&self) -> String {
        let tables = self.graph.own_table_count();
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
