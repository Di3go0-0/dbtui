//! Layered placement of diagram boxes: referenced tables to the left,
//! referencing tables to the right, boxes stacked inside each column and
//! ordered to keep edges short.
//!
//! Coordinates are terminal cells. The caller decides box sizes; this
//! module only positions them.

use std::collections::{HashMap, HashSet, VecDeque};

/// Horizontal space between columns, where the edges run.
#[cfg(test)]
pub const LAYER_GAP: i32 = 10;
/// Vertical space between boxes of one column.
#[cfg(test)]
pub const NODE_GAP: i32 = 1;

#[derive(Debug, Clone)]
pub struct LayoutNode {
    pub id: usize,
    pub width: u16,
    pub height: u16,
    /// Column the caller wants the node in; `None` lets the longest path
    /// from the referenced tables decide. Either all nodes carry a layer or
    /// none does.
    pub layer: Option<i32>,
}

#[derive(Debug, Clone, Copy)]
pub struct LayoutEdge {
    /// Node holding the foreign key.
    pub from: usize,
    /// Node it references.
    pub to: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub id: usize,
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
    pub layer: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    /// In reading order: column by column, top to bottom.
    pub nodes: Vec<Placed>,
    pub width: i32,
    pub height: i32,
}

#[cfg(test)]
pub fn layout(nodes: &[LayoutNode], edges: &[LayoutEdge]) -> Layout {
    layout_with_gaps(nodes, edges, LAYER_GAP, NODE_GAP)
}

/// Like [`layout`], with the space between columns and between stacked
/// boxes chosen by the caller (a zoomed-out diagram packs tighter).
pub fn layout_with_gaps(
    nodes: &[LayoutNode],
    edges: &[LayoutEdge],
    layer_gap: i32,
    node_gap: i32,
) -> Layout {
    if nodes.is_empty() {
        return Layout::default();
    }
    let gaps = Gaps {
        layer: layer_gap.max(2),
        node: node_gap.max(0),
    };
    let local: HashMap<usize, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    let edges: Vec<(usize, usize)> = edges
        .iter()
        .filter_map(|e| Some((*local.get(&e.from)?, *local.get(&e.to)?)))
        .filter(|(a, b)| a != b)
        .collect();

    let layers = assign_layers(nodes, &edges);
    let mut columns = group_by_layer(&layers);
    order_columns(&mut columns, &edges);
    place(nodes, &columns, &edges, gaps)
}

#[derive(Clone, Copy)]
struct Gaps {
    layer: i32,
    node: i32,
}

/// Column index per node (by local index).
fn assign_layers(nodes: &[LayoutNode], edges: &[(usize, usize)]) -> Vec<usize> {
    if nodes.iter().all(|n| n.layer.is_some()) {
        let min = nodes.iter().filter_map(|n| n.layer).min().unwrap_or(0);
        return nodes
            .iter()
            .map(|n| (n.layer.unwrap_or(min) - min) as usize)
            .collect();
    }

    let n = nodes.len();
    // Parents (edge targets) before children: a child's rank is one past
    // its deepest parent. Back edges of cycles are dropped first so the
    // ranks are finite.
    let forward = drop_back_edges(n, edges);
    let mut parents_of: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(child, parent) in &forward {
        parents_of[child].push(parent);
        children_of[parent].push(child);
    }
    let mut remaining: Vec<usize> = parents_of.iter().map(Vec::len).collect();
    let mut rank = vec![0usize; n];
    let mut queue: VecDeque<usize> = (0..n).filter(|&i| remaining[i] == 0).collect();
    while let Some(node) = queue.pop_front() {
        for &child in &children_of[node] {
            rank[child] = rank[child].max(rank[node] + 1);
            remaining[child] -= 1;
            if remaining[child] == 0 {
                queue.push_back(child);
            }
        }
    }

    // Tables without any relation go in their own columns on the right,
    // several per column, instead of swelling the first one.
    let linked: HashSet<usize> = edges.iter().flat_map(|&(a, b)| [a, b]).collect();
    let isolated: Vec<usize> = (0..n).filter(|i| !linked.contains(i)).collect();
    if !isolated.is_empty() {
        let first_free = if linked.is_empty() {
            0
        } else {
            rank.iter()
                .enumerate()
                .filter(|(i, _)| linked.contains(i))
                .map(|(_, r)| *r)
                .max()
                .unwrap_or(0)
                + 1
        };
        let per_column = ((isolated.len() as f64).sqrt().ceil() as usize).max(4);
        for (i, node) in isolated.iter().enumerate() {
            rank[*node] = first_free + i / per_column;
        }
    }
    rank
}

/// Edges that close a cycle when walking from children to parents.
fn drop_back_edges(n: usize, edges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, &(from, _)) in edges.iter().enumerate() {
        adjacency[from].push(i);
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Open,
        Done,
    }
    let mut mark = vec![Mark::New; n];
    let mut back: HashSet<usize> = HashSet::new();
    for start in 0..n {
        if mark[start] != Mark::New {
            continue;
        }
        // Iterative DFS: (node, next adjacency index).
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        mark[start] = Mark::Open;
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            if *next < adjacency[node].len() {
                let edge = adjacency[node][*next];
                *next += 1;
                let target = edges[edge].1;
                match mark[target] {
                    Mark::New => {
                        mark[target] = Mark::Open;
                        stack.push((target, 0));
                    }
                    Mark::Open => {
                        back.insert(edge);
                    }
                    Mark::Done => {}
                }
            } else {
                mark[node] = Mark::Done;
                stack.pop();
            }
        }
    }
    edges
        .iter()
        .enumerate()
        .filter(|(i, _)| !back.contains(i))
        .map(|(_, e)| *e)
        .collect()
}

fn group_by_layer(layers: &[usize]) -> Vec<Vec<usize>> {
    let count = layers.iter().max().map(|m| m + 1).unwrap_or(0);
    let mut columns = vec![Vec::new(); count];
    for (node, &layer) in layers.iter().enumerate() {
        columns[layer].push(node);
    }
    columns
}

/// Barycenter sweeps: sort each column by the mean position of the
/// neighbours in the column just visited.
fn order_columns(columns: &mut [Vec<usize>], edges: &[(usize, usize)]) {
    let n = columns.iter().map(Vec::len).sum::<usize>();
    let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in edges {
        neighbours[a].push(b);
        neighbours[b].push(a);
    }
    let mut position = vec![0usize; n];
    let refresh = |column: &[usize], position: &mut Vec<usize>| {
        for (i, &node) in column.iter().enumerate() {
            position[node] = i;
        }
    };
    for column in columns.iter() {
        refresh(column, &mut position);
    }

    for sweep in 0..4 {
        let forward = sweep % 2 == 0;
        let count = columns.len();
        for step in 1..count {
            let (current, reference) = if forward {
                (step, step - 1)
            } else {
                (count - 1 - step, count - step)
            };
            let in_reference: HashSet<usize> = columns[reference].iter().copied().collect();
            let mut keyed: Vec<(f64, usize)> = columns[current]
                .iter()
                .map(|&node| {
                    let near: Vec<f64> = neighbours[node]
                        .iter()
                        .filter(|m| in_reference.contains(m))
                        .map(|&m| position[m] as f64)
                        .collect();
                    let key = if near.is_empty() {
                        position[node] as f64
                    } else {
                        near.iter().sum::<f64>() / near.len() as f64
                    };
                    (key, node)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            columns[current] = keyed.into_iter().map(|(_, node)| node).collect();
            refresh(&columns[current], &mut position);
        }
    }
}

/// Column x from the widths, box y from stacking, then two passes pulling
/// each box toward the vertical centre of its neighbours without
/// reordering or overlapping.
fn place(
    nodes: &[LayoutNode],
    columns: &[Vec<usize>],
    edges: &[(usize, usize)],
    gaps: Gaps,
) -> Layout {
    let n = nodes.len();
    let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in edges {
        neighbours[a].push(b);
        neighbours[b].push(a);
    }

    let mut x = vec![0i32; n];
    let mut y = vec![0i32; n];
    let mut column_x = 0i32;
    for column in columns {
        let width = column
            .iter()
            .map(|&i| nodes[i].width as i32)
            .max()
            .unwrap_or(0);
        let mut cursor = 0i32;
        for &node in column {
            x[node] = column_x;
            y[node] = cursor;
            cursor += nodes[node].height as i32 + gaps.node;
        }
        column_x += width + gaps.layer;
    }

    // Centre the columns against the tallest one, so a single parent of
    // ten children sits level with the middle child.
    let tallest = columns
        .iter()
        .map(|c| column_height(c, nodes, gaps.node))
        .max()
        .unwrap_or(0);
    for column in columns {
        let offset = (tallest - column_height(column, nodes, gaps.node)) / 2;
        for &node in column {
            y[node] += offset;
        }
    }

    let centre = |node: usize, y: &[i32]| y[node] + nodes[node].height as i32 / 2;
    for pass in 0..2 {
        let order: Vec<usize> = if pass == 0 {
            (0..columns.len()).collect()
        } else {
            (0..columns.len()).rev().collect()
        };
        for &col in &order {
            let mut floor = i32::MIN;
            for &node in &columns[col] {
                let near: Vec<i32> = neighbours[node]
                    .iter()
                    .filter(|&&m| x[m] != x[node])
                    .map(|&m| centre(m, &y))
                    .collect();
                let wanted = if near.is_empty() {
                    y[node]
                } else {
                    near.iter().sum::<i32>() / near.len() as i32 - nodes[node].height as i32 / 2
                };
                let placed = if floor == i32::MIN {
                    wanted
                } else {
                    wanted.max(floor)
                };
                y[node] = placed;
                floor = placed + nodes[node].height as i32 + gaps.node;
            }
        }
    }

    let min_y = y.iter().copied().min().unwrap_or(0);
    let mut placed = Vec::with_capacity(n);
    for (layer, column) in columns.iter().enumerate() {
        for &node in column {
            placed.push(Placed {
                id: nodes[node].id,
                x: x[node],
                y: y[node] - min_y,
                width: nodes[node].width,
                height: nodes[node].height,
                layer,
            });
        }
    }
    let width = placed
        .iter()
        .map(|p| p.x + p.width as i32)
        .max()
        .unwrap_or(0);
    let height = placed
        .iter()
        .map(|p| p.y + p.height as i32)
        .max()
        .unwrap_or(0);
    Layout {
        nodes: placed,
        width,
        height,
    }
}

fn column_height(column: &[usize], nodes: &[LayoutNode], node_gap: i32) -> i32 {
    let boxes: i32 = column.iter().map(|&i| nodes[i].height as i32).sum();
    boxes + node_gap * (column.len().saturating_sub(1) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: usize) -> LayoutNode {
        LayoutNode {
            id,
            width: 20,
            height: 5,
            layer: None,
        }
    }

    fn edge(from: usize, to: usize) -> LayoutEdge {
        LayoutEdge { from, to }
    }

    fn find(layout: &Layout, id: usize) -> &Placed {
        layout.nodes.iter().find(|p| p.id == id).expect("placed")
    }

    #[test]
    fn parents_go_left_of_children() {
        // 0 <- 1 <- 2, plus 3 referencing 0.
        let nodes: Vec<LayoutNode> = (0..4).map(node).collect();
        let l = layout(&nodes, &[edge(1, 0), edge(2, 1), edge(3, 0)]);
        assert_eq!(find(&l, 0).layer, 0);
        assert_eq!(find(&l, 1).layer, 1);
        assert_eq!(find(&l, 3).layer, 1);
        assert_eq!(find(&l, 2).layer, 2);
        assert_eq!(find(&l, 1).x, 20 + LAYER_GAP);
        assert_eq!(find(&l, 2).x, 2 * (20 + LAYER_GAP));
        assert_eq!(l.width, 3 * 20 + 2 * LAYER_GAP);
    }

    #[test]
    fn boxes_in_one_column_never_overlap() {
        let nodes: Vec<LayoutNode> = (0..6).map(node).collect();
        let edges: Vec<LayoutEdge> = (1..6).map(|i| edge(i, 0)).collect();
        let l = layout(&nodes, &edges);
        let mut children: Vec<&Placed> = l.nodes.iter().filter(|p| p.layer == 1).collect();
        children.sort_by_key(|p| p.y);
        assert_eq!(children.len(), 5);
        for pair in children.windows(2) {
            assert!(pair[1].y >= pair[0].y + pair[0].height as i32 + NODE_GAP);
        }
        let parent = find(&l, 0);
        let first = children[0].y;
        let last = children[4].y + children[4].height as i32;
        assert!(
            parent.y >= first && parent.y + 5 <= last,
            "parent centred on its children"
        );
    }

    #[test]
    fn cycles_still_get_finite_layers() {
        let nodes: Vec<LayoutNode> = (0..3).map(node).collect();
        let l = layout(&nodes, &[edge(0, 1), edge(1, 2), edge(2, 0)]);
        assert_eq!(l.nodes.len(), 3);
        let layers: HashSet<usize> = l.nodes.iter().map(|p| p.layer).collect();
        assert_eq!(layers.len(), 3);
    }

    #[test]
    fn isolated_tables_fill_extra_columns_on_the_right() {
        let mut nodes: Vec<LayoutNode> = (0..2).map(node).collect();
        nodes.extend((2..22).map(node));
        let l = layout(&nodes, &[edge(1, 0)]);
        assert_eq!(find(&l, 1).layer, 1);
        let isolated_layers: HashSet<usize> = l
            .nodes
            .iter()
            .filter(|p| p.id >= 2)
            .map(|p| p.layer)
            .collect();
        assert!(isolated_layers.iter().all(|&layer| layer >= 2));
        assert!(
            isolated_layers.len() > 1,
            "20 tables spread over several columns"
        );
    }

    #[test]
    fn explicit_layers_are_honoured_and_normalised() {
        let nodes = vec![
            LayoutNode {
                layer: Some(-1),
                ..node(0)
            },
            LayoutNode {
                layer: Some(0),
                ..node(1)
            },
            LayoutNode {
                layer: Some(1),
                ..node(2)
            },
        ];
        let l = layout(&nodes, &[edge(1, 0), edge(2, 1)]);
        assert_eq!(find(&l, 0).layer, 0);
        assert_eq!(find(&l, 1).layer, 1);
        assert_eq!(find(&l, 2).layer, 2);
        assert_eq!(l.nodes.iter().map(|p| p.y).min(), Some(0));
    }

    #[test]
    fn empty_input_gives_empty_layout() {
        let l = layout(&[], &[]);
        assert!(l.nodes.is_empty());
        assert_eq!((l.width, l.height), (0, 0));
    }
}
