use crate::core::models::*;
use crate::ui::state::{CategoryKind, LeafKind, TreeNode};

pub(super) trait HasName {
    fn get_name(&self) -> String;
    fn is_valid(&self) -> bool;
    fn get_privilege(&self) -> ObjectPrivilege;
}
impl HasName for Table {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        self.privilege
    }
}
impl HasName for View {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        self.valid
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        self.privilege
    }
}
impl HasName for Procedure {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        self.valid
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        self.privilege
    }
}
impl HasName for Function {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        self.valid
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        self.privilege
    }
}
impl HasName for MaterializedView {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        self.valid
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        self.privilege
    }
}
impl HasName for Index {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        ObjectPrivilege::Unknown
    }
}
impl HasName for Sequence {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        ObjectPrivilege::Unknown
    }
}
impl HasName for DbType {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        ObjectPrivilege::Unknown
    }
}
impl HasName for Trigger {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        ObjectPrivilege::Unknown
    }
}
impl HasName for DbEvent {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn get_privilege(&self) -> ObjectPrivilege {
        ObjectPrivilege::Unknown
    }
}

/// Extract FUNCTION or PROCEDURE names from a PL/SQL package declaration/body.
/// Looks for lines like "FUNCTION name" or "PROCEDURE name".
pub(super) fn extract_names(source: &str, kind: &str) -> Vec<String> {
    let kind_upper = kind.to_uppercase();
    let kind_len = kind_upper.len();
    let mut names = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        let trimmed_upper = trimmed.to_uppercase();
        if let Some(rest_upper) = trimmed_upper.strip_prefix(&kind_upper)
            && rest_upper.starts_with(|c: char| c.is_whitespace())
        {
            // Get the original-case name from the original line
            let original_rest = &trimmed[kind_len..].trim_start();
            let name: String = original_rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Word-wrap error text to fit within `max_width` columns.
pub(super) fn wrap_error_text(error: &str, max_width: usize) -> String {
    let mut lines = Vec::new();

    // Extract line number from error (MySQL: "at line N", PostgreSQL: "LINE N:", Oracle: "line N")
    let line_num = extract_error_line(error);
    let header = match line_num {
        Some(n) => format!("-- Query Error (line {n}) --"),
        None => "-- Query Error --".to_string(),
    };
    lines.push(header);
    lines.push(String::new());

    // Strip SQL snippets from error (already shown in Query pane)
    // e.g. "...near 'SELECT * FROM...' at line 1"
    let cleaned = if let Some(pos) = error.find(" near '") {
        let before = &error[..pos];
        // Try to find "at line N" after the snippet
        let after = error[pos..]
            .find("' at line ")
            .map(|p| &error[pos + p + 1..])
            .unwrap_or("");
        format!("{before}{after}")
    } else {
        error.to_string()
    };

    // Split on ": " to break long error chains into sections
    for section in cleaned.split(": ") {
        let section = section.trim();
        if section.is_empty() {
            continue;
        }
        // Word-wrap each section
        let mut current_line = String::new();
        for word in section.split_whitespace() {
            if current_line.is_empty() {
                current_line.push_str(word);
            } else if current_line.len() + 1 + word.len() > max_width {
                lines.push(current_line);
                current_line = format!("  {word}"); // indent continuation
            } else {
                current_line.push(' ');
                current_line.push_str(word);
            }
        }
        if !current_line.is_empty() {
            lines.push(current_line);
        }
    }

    lines.push(String::new());
    lines.join("\n")
}

/// Extract line number from database error messages.
/// Matches patterns: "at line N", "LINE N:", "line N,", "ORA-NNNNN: ... line N"
pub(super) fn extract_error_line(error: &str) -> Option<usize> {
    let lower = error.to_lowercase();

    // "at line N" (MySQL)
    if let Some(pos) = lower.find("at line ") {
        let after = &error[pos + 8..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = num.parse::<usize>() {
            return Some(n);
        }
    }

    // "LINE N:" (PostgreSQL)
    if let Some(pos) = lower.find("line ") {
        let after = &error[pos + 5..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = num.parse::<usize>() {
            return Some(n);
        }
    }

    None
}

use super::App;
use crate::sql_engine::metadata::ObjectKind as ObjKind;

impl App {
    /// Generic handler for metadata loaded messages (Tables, Views, Procedures, etc.)
    pub(super) fn handle_objects_loaded<T: HasName>(
        &mut self,
        conn_name: &str,
        schema: &str,
        items: Vec<T>,
        obj_kind: ObjKind,
        cat_kind: CategoryKind,
        leaf_kind: LeafKind,
    ) {
        let idx = self
            .state
            .engine
            .metadata_indexes
            .entry(conn_name.to_string())
            .or_default();
        for item in &items {
            idx.add_object(schema, &item.get_name(), obj_kind);
        }
        self.insert_leaves(conn_name, schema, cat_kind, items, leaf_kind);
        self.finish_loading();
    }

    /// Reset loading state after an async operation completes.
    pub(super) fn finish_loading(&mut self) {
        self.state.loading = false;
        self.state.loading_since = None;
    }

    pub(super) fn insert_leaves<T: HasName>(
        &mut self,
        conn_name: &str,
        schema: &str,
        category: CategoryKind,
        items: Vec<T>,
        leaf_kind: LeafKind,
    ) {
        let cat_idx = self.find_category_in_connection(conn_name, schema, &category);
        if let Some(idx) = cat_idx {
            self.remove_children_of(idx);

            if items.is_empty() {
                self.state.sidebar.tree.insert(idx + 1, TreeNode::Empty);
                return;
            }

            // Build batch and splice (O(n) instead of O(n²))
            let batch: Vec<TreeNode> = items
                .iter()
                .map(|item| TreeNode::Leaf {
                    name: item.get_name(),
                    schema: schema.to_string(),
                    kind: leaf_kind.clone(),
                    valid: item.is_valid(),
                    privilege: item.get_privilege(),
                })
                .collect();
            let insert_pos = idx + 1;
            self.state
                .sidebar
                .tree
                .splice(insert_pos..insert_pos, batch);
        }
    }

    pub(super) fn insert_package_leaves(
        &mut self,
        conn_name: &str,
        schema: &str,
        items: Vec<Package>,
    ) {
        let cat_idx = self.find_category_in_connection(conn_name, schema, &CategoryKind::Packages);
        if let Some(idx) = cat_idx {
            self.remove_children_of(idx);

            if items.is_empty() {
                self.state.sidebar.tree.insert(idx + 1, TreeNode::Empty);
                return;
            }

            let batch: Vec<TreeNode> = items
                .into_iter()
                .map(|pkg| TreeNode::Leaf {
                    name: pkg.name,
                    schema: schema.to_string(),
                    kind: LeafKind::Package,
                    valid: pkg.valid,
                    privilege: pkg.privilege,
                })
                .collect();
            let insert_pos = idx + 1;
            self.state
                .sidebar
                .tree
                .splice(insert_pos..insert_pos, batch);
        }
    }

    /// Find a Category node within a specific connection's subtree.
    pub(super) fn find_category_in_connection(
        &self,
        conn_name: &str,
        schema: &str,
        category: &CategoryKind,
    ) -> Option<usize> {
        let tree = &self.state.sidebar.tree;
        // Find the connection node first
        let conn_idx = tree
            .iter()
            .position(|n| matches!(n, TreeNode::Connection { name, .. } if name == conn_name))?;
        let conn_depth = tree[conn_idx].depth();
        // Search within this connection's subtree
        let mut i = conn_idx + 1;
        while i < tree.len() && tree[i].depth() > conn_depth {
            if matches!(&tree[i], TreeNode::Category { schema: s, kind, .. } if s == schema && kind == category)
            {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    pub(super) fn remove_children_of(&mut self, parent_idx: usize) {
        let parent_depth = self.state.sidebar.tree[parent_idx].depth();
        let start = parent_idx + 1;
        let mut end = start;
        while end < self.state.sidebar.tree.len()
            && self.state.sidebar.tree[end].depth() > parent_depth
        {
            end += 1;
        }
        if end > start {
            self.state.sidebar.tree.drain(start..end);
        }
    }
}
