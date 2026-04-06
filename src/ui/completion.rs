/// UI types for the completion popup and helpers for cache resolution.
/// The completion engine itself lives in `sql_engine::completion`.
use crate::ui::sql_tokens;
use crate::ui::state::{AppState, LeafKind, TreeNode};

// ---------------------------------------------------------------------------
// Public types (used by events.rs, state.rs, layout.rs)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionKind {
    Keyword,
    Schema,
    Table,
    View,
    Column,
    Package,
    Function,
    Procedure,
    Alias,
}

impl CompletionKind {
    pub fn tag(&self) -> &str {
        match self {
            CompletionKind::Keyword => "kw",
            CompletionKind::Schema => "sch",
            CompletionKind::Table => "tbl",
            CompletionKind::View => "view",
            CompletionKind::Column => "col",
            CompletionKind::Package => "pkg",
            CompletionKind::Function => "fn",
            CompletionKind::Procedure => "proc",
            CompletionKind::Alias => "alias",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompletionState {
    pub items: Vec<CompletionItem>,
    pub cursor: usize,
    #[allow(dead_code)]
    pub prefix: String,
    pub origin_row: usize,
    pub origin_col: usize,
}

impl CompletionState {
    pub fn selected(&self) -> Option<&CompletionItem> {
        self.items.get(self.cursor)
    }

    pub fn next(&mut self) {
        if !self.items.is_empty() {
            self.cursor = (self.cursor + 1) % self.items.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.items.is_empty() {
            self.cursor = if self.cursor == 0 {
                self.items.len() - 1
            } else {
                self.cursor - 1
            };
        }
    }
}

// ---------------------------------------------------------------------------
// Cache resolution helpers (used by events.rs for on-demand column loading)
// ---------------------------------------------------------------------------

/// Find the schema for a table name by looking in the tree metadata.
pub fn find_schema_for_table(state: &AppState, table_name: &str) -> Option<String> {
    let upper = table_name.to_uppercase();
    let lower = table_name.to_lowercase();
    for node in &state.tree {
        if let TreeNode::Leaf {
            name, schema, kind, ..
        } = node
            && matches!(kind, LeafKind::Table | LeafKind::View)
            && (name.to_uppercase() == upper || name.to_lowercase() == lower)
        {
            return Some(schema.clone());
        }
    }
    None
}

/// Resolve a table reference (possibly an alias) to the actual table name.
pub fn resolve_table_name(lines: &[String], reference: &str) -> Option<String> {
    let ref_upper = reference.to_uppercase();
    let full_text: String = lines.join(" ");
    let words: Vec<&str> = full_text.split_whitespace().collect();
    let upper_words: Vec<String> = words.iter().map(|w| w.to_uppercase()).collect();

    for i in 0..words.len() {
        if !matches!(
            upper_words[i].as_str(),
            "FROM" | "JOIN" | "INNER" | "LEFT" | "RIGHT" | "FULL" | "CROSS"
        ) {
            continue;
        }

        let mut j = i + 1;
        while j < words.len()
            && matches!(
                upper_words[j].as_str(),
                "JOIN" | "OUTER" | "INNER" | "LEFT" | "RIGHT" | "FULL" | "CROSS" | "NATURAL"
            )
        {
            j += 1;
        }

        if j >= words.len() {
            continue;
        }

        let table_token = words[j];
        let actual = table_token
            .rsplit('.')
            .next()
            .unwrap_or(table_token)
            .trim_end_matches(',');

        // Check alias
        let alias_idx = j + 1;
        if alias_idx < words.len() {
            let pot = &upper_words[alias_idx];
            let is_alias_match = if pot == "AS" {
                alias_idx + 1 < words.len()
                    && upper_words[alias_idx + 1].trim_end_matches(',') == ref_upper
            } else {
                !sql_tokens::is_sql_keyword(pot) && pot.trim_end_matches(',') == ref_upper
            };
            if is_alias_match {
                return Some(actual.to_string());
            }
        }

        if actual.to_uppercase() == ref_upper {
            return Some(actual.to_string());
        }
    }

    None
}
