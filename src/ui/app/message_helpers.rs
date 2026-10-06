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

/// Word-wrap error text to fit within `max_width` columns. The caller
/// supplies its own header line.
///
/// Line breaks in the message are kept — drivers use them to separate the
/// error from its detail and hint lines — and each line is further broken on
/// `": "` so a chain like `ORA-06550: line 3, column 5: PLS-00201: …` reads as
/// one clause per row.
pub(super) fn wrap_error_text(error: &str, max_width: usize) -> String {
    let mut lines = Vec::new();

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

    for section in cleaned.lines().flat_map(|line| line.split(": ")) {
        if section.trim().is_empty() {
            continue;
        }
        wrap_section(section, max_width, &mut lines);
    }

    lines.push(String::new());
    lines.join("\n")
}

/// Wrap one clause on word boundaries, keeping its leading indentation (hint
/// bullets are indented) and indenting continuation rows a little further.
fn wrap_section(section: &str, max_width: usize, out: &mut Vec<String>) {
    let indent: String = section.chars().take_while(|c| c.is_whitespace()).collect();
    let mut current = String::new();
    for word in section.split_whitespace() {
        if current.is_empty() {
            current = format!("{indent}{word}");
        } else if current.chars().count() + 1 + word.chars().count() > max_width {
            out.push(std::mem::take(&mut current));
            current = format!("{indent}  {word}");
        } else {
            current.push(' ');
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
}

/// One error line of a failed compile: 1-based line and column inside the
/// compiled source, plus the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileMark {
    pub line: usize,
    pub col: usize,
    pub text: String,
}

/// Pull the per-line errors out of a compile failure message. Two shapes are
/// recognised, one per row: `Line 12, Col 5: text` (the `ALL_ERRORS` listing
/// built after a compile) and `12/5: text` (SQL*Plus `SHOW ERRORS` style).
pub(super) fn parse_compile_marks(message: &str) -> Vec<CompileMark> {
    message.lines().filter_map(parse_compile_mark).collect()
}

fn parse_compile_mark(row: &str) -> Option<CompileMark> {
    let row = row.trim();
    let (location, text) = row.split_once(": ")?;
    let (line, col) = match location.strip_prefix("Line ") {
        Some(rest) => {
            let (line, col) = rest.split_once(", Col ")?;
            (line, col)
        }
        None => location.split_once('/')?,
    };
    Some(CompileMark {
        line: line.trim().parse().ok()?,
        col: col.trim().parse().ok()?,
        text: text.trim().to_string(),
    })
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
                let depth = self.state.sidebar.tree[idx].depth() + 1;
                self.state
                    .sidebar
                    .tree
                    .insert(idx + 1, TreeNode::Empty { depth });
                return;
            }

            // Build batch and splice (O(n) instead of O(n²))
            let is_table_or_view = matches!(leaf_kind, LeafKind::Table | LeafKind::View);
            let catalog = self.state.sidebar.tree[idx]
                .catalog()
                .map(|c| c.to_string());
            let batch: Vec<TreeNode> = items
                .iter()
                .map(|item| TreeNode::Leaf {
                    name: item.get_name(),
                    schema: schema.to_string(),
                    catalog: catalog.clone(),
                    kind: leaf_kind.clone(),
                    valid: item.is_valid(),
                    privilege: item.get_privilege(),
                })
                .collect();
            // Update table→schema index for O(1) lookups
            if is_table_or_view {
                for item in &items {
                    self.state
                        .sidebar
                        .table_schema_index
                        .entry(item.get_name().to_uppercase())
                        .or_insert_with(|| schema.to_string());
                }
            }
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
                let depth = self.state.sidebar.tree[idx].depth() + 1;
                self.state
                    .sidebar
                    .tree
                    .insert(idx + 1, TreeNode::Empty { depth });
                return;
            }

            let catalog = self.state.sidebar.tree[idx]
                .catalog()
                .map(|c| c.to_string());
            let batch: Vec<TreeNode> = items
                .into_iter()
                .map(|pkg| TreeNode::Leaf {
                    name: pkg.name,
                    schema: schema.to_string(),
                    catalog: catalog.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_keeps_driver_line_breaks_and_indentation() {
        let text = wrap_error_text(
            "Query failed: boom\nPossible causes:\n  • first\n  • second",
            40,
        );
        assert_eq!(
            text,
            "Query failed\nboom\nPossible causes:\n  • first\n  • second\n"
        );
    }

    #[test]
    fn wrap_breaks_long_clauses_on_words() {
        let text = wrap_error_text("one two three four", 9);
        assert_eq!(text, "one two\n  three\n  four\n");
    }

    #[test]
    fn compile_marks_from_both_listing_styles() {
        let marks = parse_compile_marks(
            "Line 12, Col 5: PLS-00201: identifier 'X' must be declared\n3/1: PL/SQL: Statement ignored\nnot a mark",
        );
        assert_eq!(
            marks,
            vec![
                CompileMark {
                    line: 12,
                    col: 5,
                    text: "PLS-00201: identifier 'X' must be declared".to_string()
                },
                CompileMark {
                    line: 3,
                    col: 1,
                    text: "PL/SQL: Statement ignored".to_string()
                },
            ]
        );
    }
}
