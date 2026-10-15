//! Tables and foreign keys as a graph: one node per table, one edge per
//! constraint, pointing from the table that holds the key to the table it
//! references.

use std::collections::{HashMap, HashSet, VecDeque};

use super::{Relationship, group_constraints};
use crate::core::models::ForeignKeyInfo;

#[derive(Debug, Clone)]
pub struct ErEdge {
    /// Node holding the foreign key.
    pub from: usize,
    /// Node the key references.
    pub to: usize,
    pub rel: Relationship,
}

#[derive(Debug, Clone, Default)]
pub struct ErGraph {
    /// Table names. Tables of the diagram's own schema keep their bare
    /// name; referenced tables from other schemas are `schema.table`.
    pub nodes: Vec<String>,
    pub edges: Vec<ErEdge>,
    /// `(schema, table)` of the nodes that live in another schema, by
    /// node; `None` for the diagram's own tables. A name is never parsed
    /// to tell the two apart, so a dot inside a table name is harmless.
    external: Vec<Option<(String, String)>>,
    index: HashMap<String, usize>,
    outgoing: Vec<Vec<usize>>,
    incoming: Vec<Vec<usize>>,
}

impl ErGraph {
    /// Build the graph of `schema`: `tables` in the given order, then any
    /// table a foreign key mentions that the list does not.
    pub fn build(schema: &str, tables: &[String], fks: &[ForeignKeyInfo]) -> Self {
        let mut graph = Self::default();
        for table in tables {
            graph.intern(table);
        }
        for rel in group_constraints(fks) {
            let from = graph.intern(&rel.table);
            let to = if rel.referenced_schema.is_empty() || rel.referenced_schema == schema {
                graph.intern(&rel.referenced_table)
            } else {
                graph.intern_external(&rel.referenced_schema, &rel.referenced_table)
            };
            let edge = graph.edges.len();
            graph.edges.push(ErEdge { from, to, rel });
            graph.outgoing[from].push(edge);
            graph.incoming[to].push(edge);
        }
        graph
    }

    fn intern(&mut self, name: &str) -> usize {
        if let Some(&id) = self.index.get(name) {
            return id;
        }
        let id = self.nodes.len();
        self.nodes.push(name.to_string());
        self.external.push(None);
        self.index.insert(name.to_string(), id);
        self.outgoing.push(Vec::new());
        self.incoming.push(Vec::new());
        id
    }

    fn intern_external(&mut self, schema: &str, table: &str) -> usize {
        let name = format!("{schema}.{table}");
        let id = self.intern(&name);
        self.external[id] = Some((schema.to_string(), table.to_string()));
        id
    }

    /// `(schema, table)` when the node is a table of another schema.
    pub fn external(&self, node: usize) -> Option<&(String, String)> {
        self.external.get(node).and_then(|e| e.as_ref())
    }

    pub fn is_external(&self, node: usize) -> bool {
        self.external(node).is_some()
    }

    /// Tables of the diagram's own schema.
    pub fn own_table_count(&self) -> usize {
        self.external.iter().filter(|e| e.is_none()).count()
    }

    pub fn node_id(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Edges leaving `node` (keys it holds).
    pub fn outgoing(&self, node: usize) -> &[usize] {
        self.outgoing.get(node).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Edges arriving at `node` (keys pointing at it).
    pub fn incoming(&self, node: usize) -> &[usize] {
        self.incoming.get(node).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Nodes within `depth` constraints of `focus`, each with a signed
    /// layer: referenced tables sit at negative layers, referencing tables
    /// at positive ones, the focus at zero. A node reachable both ways
    /// keeps the layer of its shortest path.
    pub fn neighborhood(&self, focus: usize, depth: usize) -> Vec<(usize, i32)> {
        let mut seen: HashSet<usize> = HashSet::new();
        let mut out = Vec::new();
        let mut queue = VecDeque::new();
        if focus >= self.nodes.len() {
            return out;
        }
        seen.insert(focus);
        queue.push_back((focus, 0i32, 0usize));
        while let Some((node, layer, dist)) = queue.pop_front() {
            out.push((node, layer));
            if dist == depth {
                continue;
            }
            let parents = self
                .outgoing(node)
                .iter()
                .map(|&e| (self.edges[e].to, layer - 1));
            let children = self
                .incoming(node)
                .iter()
                .map(|&e| (self.edges[e].from, layer + 1));
            for (next, next_layer) in parents.chain(children) {
                if seen.insert(next) {
                    queue.push_back((next, next_layer, dist + 1));
                }
            }
        }
        out
    }

    /// Edges with both ends inside `nodes`.
    pub fn edges_within(&self, nodes: &[usize]) -> Vec<usize> {
        let set: HashSet<usize> = nodes.iter().copied().collect();
        (0..self.edges.len())
            .filter(|&e| set.contains(&self.edges[e].from) && set.contains(&self.edges[e].to))
            .collect()
    }
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

    fn shop() -> ErGraph {
        let tables: Vec<String> = [
            "countries",
            "users",
            "orders",
            "order_items",
            "products",
            "logs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mut hr = fk("users", "manager_id", "people");
        hr.referenced_schema = "hr".into();
        ErGraph::build(
            "app",
            &tables,
            &[
                fk("users", "country_id", "countries"),
                fk("orders", "user_id", "users"),
                fk("order_items", "order_id", "orders"),
                fk("order_items", "product_id", "products"),
                hr,
            ],
        )
    }

    #[test]
    fn external_tables_are_appended_qualified() {
        let g = shop();
        assert_eq!(g.nodes.len(), 7);
        assert_eq!(g.nodes[6], "hr.people");
        assert_eq!(
            g.external(6),
            Some(&("hr".to_string(), "people".to_string()))
        );
        assert!(!g.is_external(0));
        assert_eq!(g.own_table_count(), 6);
        assert_eq!(g.node_id("logs"), Some(5));
        assert!(g.outgoing(5).is_empty() && g.incoming(5).is_empty());
        assert_eq!(
            g.outgoing(1).len() + g.incoming(1).len(),
            3,
            "users: country, manager, orders"
        );
    }

    #[test]
    fn neighborhood_layers_parents_left_and_children_right() {
        let g = shop();
        let orders = g.node_id("orders").expect("orders");
        let mut near = g.neighborhood(orders, 1);
        near.sort_by_key(|(n, _)| *n);
        let named: Vec<(&str, i32)> = near
            .iter()
            .map(|(n, l)| (g.nodes[*n].as_str(), *l))
            .collect();
        assert_eq!(named, [("users", -1), ("orders", 0), ("order_items", 1)]);

        let deep = g.neighborhood(orders, 2);
        let layer = |name: &str| {
            deep.iter()
                .find(|(n, _)| g.nodes[*n] == name)
                .map(|(_, l)| *l)
        };
        assert_eq!(layer("countries"), Some(-2));
        assert_eq!(layer("hr.people"), Some(-2));
        assert_eq!(layer("products"), Some(0), "sibling parent of order_items");
        assert_eq!(layer("logs"), None);
    }

    #[test]
    fn self_reference_counts_once_and_stays_in_the_neighborhood() {
        let g = ErGraph::build(
            "app",
            &["emp".to_string()],
            &[fk("emp", "manager_id", "emp")],
        );
        assert_eq!(g.neighborhood(0, 1), vec![(0, 0)]);
        assert_eq!(g.edges_within(&[0]), vec![0]);
    }
}
