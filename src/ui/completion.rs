/// Context-aware, dialect-aware SQL completion engine.
/// Understands SQL syntax position and suggests appropriate items:
/// tables after FROM/JOIN, columns after SELECT/WHERE, schemas for Oracle, etc.
use std::collections::HashSet;

use crate::core::models::DatabaseType;
use crate::ui::sql_tokens;
use crate::ui::state::{AppState, LeafKind, TreeNode};
use crate::ui::tabs::TabKind;

/// Resolve the effective connection name: script's conn_name > global connection_name.
pub fn effective_conn_name(state: &AppState) -> Option<String> {
    state
        .active_tab()
        .and_then(|t| t.kind.conn_name().map(|s| s.to_string()))
        .or_else(|| state.connection_name.clone())
}

// ---------------------------------------------------------------------------
// Public types
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
// SQL context detection
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum SqlContext {
    /// After SELECT (before FROM): columns, functions, *, DISTINCT
    SelectList,
    /// After FROM / JOIN: schemas (Oracle/PG), tables, views
    TableRef,
    /// After WHERE / AND / OR / ON / HAVING: columns, functions
    Predicate,
    /// After FROM/JOIN table [alias]: clause keywords (WHERE, JOIN, ORDER, etc.)
    AfterTable,
    /// After INSERT INTO / UPDATE: tables only
    TableTarget,
    /// After SET (in UPDATE): columns of the UPDATE target
    SetClause { update_table: String },
    /// After ORDER BY / GROUP BY: columns
    OrderGroupBy,
    /// After EXEC / EXECUTE / CALL: procedures, packages
    ExecCall,
    /// After CREATE / ALTER / DROP: DDL keywords
    DdlObject,
    /// "SCHEMA." → objects in that schema
    SchemaDot { schema_name: String },
    /// "table." or "alias." → columns of that table
    ColumnDot { table_ref: String },
    /// Unknown position: keywords only
    General,
}

/// Detect SQL context at cursor position within a query block.
pub fn detect_context(state: &AppState, lines: &[String], row: usize, col: usize) -> SqlContext {
    let line = if row < lines.len() {
        &lines[row]
    } else {
        return SqlContext::General;
    };
    let before = &line[..col.min(line.len())];

    // --- Dot context detection ---
    if let Some(ctx) = detect_dot_context(state, before) {
        return ctx;
    }

    // --- Keyword-based context ---
    find_keyword_context(lines, row, col)
}

/// Check for `identifier.` pattern and resolve to SchemaDot or ColumnDot.
fn detect_dot_context(state: &AppState, before_cursor: &str) -> Option<SqlContext> {
    let bytes = before_cursor.as_bytes();
    let mut pos = bytes.len();

    // Skip back over current word prefix (after the dot)
    while pos > 0 && (bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'_') {
        pos -= 1;
    }

    // Must have a dot immediately before
    if pos == 0 || bytes[pos - 1] != b'.' {
        return None;
    }
    let dot_pos = pos - 1;

    // Extract identifier before the dot
    let id_end = dot_pos;
    let mut id_start = id_end;
    while id_start > 0
        && (bytes[id_start - 1].is_ascii_alphanumeric() || bytes[id_start - 1] == b'_')
    {
        id_start -= 1;
    }
    if id_start >= id_end {
        return None;
    }

    let identifier = &before_cursor[id_start..id_end];

    // Resolve: is this a schema name or a table/alias?
    if is_known_schema(state, identifier) {
        Some(SqlContext::SchemaDot {
            schema_name: identifier.to_string(),
        })
    } else {
        Some(SqlContext::ColumnDot {
            table_ref: identifier.to_string(),
        })
    }
}

/// Check if an identifier matches a known schema in the tree.
pub fn is_known_schema(state: &AppState, name: &str) -> bool {
    let upper = name.to_uppercase();
    let lower = name.to_lowercase();
    state.tree.iter().any(|node| {
        if let TreeNode::Schema { name: sn, .. } = node {
            sn.to_uppercase() == upper || sn.to_lowercase() == lower
        } else {
            false
        }
    })
}

/// Scan backwards through the query block to find the SQL keyword context.
fn find_keyword_context(lines: &[String], row: usize, col: usize) -> SqlContext {
    let mut words = Vec::new();

    // Current line up to cursor
    if row < lines.len() {
        let before = &lines[row][..col.min(lines[row].len())];
        extract_words_reverse(before, &mut words);
    }

    // Previous lines in the block
    for r in (0..row).rev() {
        if r < lines.len() {
            extract_words_reverse(&lines[r], &mut words);
        }
    }

    // Find the first context keyword scanning backwards.
    // words[0] is the current prefix being typed (don't count it as a completed ident).
    // words[1..] are completed words between cursor and the keyword.
    // For FROM/JOIN/UPDATE: only TableRef if no completed idents between keyword and cursor.
    let mut idents_before_keyword = 0;

    for (i, word) in words.iter().enumerate() {
        let upper = word.to_uppercase();
        // words[0] is the prefix being typed — skip it for context detection
        // so that typing "or" after FROM doesn't match the OR keyword.
        if i == 0 {
            continue;
        }
        if !sql_tokens::is_sql_keyword(&upper) {
            idents_before_keyword += 1;
            continue;
        }

        match upper.as_str() {
            "SELECT" => return SqlContext::SelectList,
            "FROM" | "JOIN" => {
                if idents_before_keyword == 0 {
                    return SqlContext::TableRef;
                }
                // After "FROM table [alias]" → clause continuation
                return SqlContext::AfterTable;
            }
            "INNER" | "LEFT" | "RIGHT" | "FULL" | "CROSS" | "NATURAL" => {
                // After "LEFT JOIN table" → AfterTable
                // After "LEFT JOIN" → TableRef (waiting for table name)
                // After "LEFT" → TableRef (next word is JOIN/OUTER, handled in builder)
                if idents_before_keyword == 0 {
                    return SqlContext::TableRef;
                }
                return SqlContext::AfterTable;
            }
            "WHERE" | "AND" | "OR" | "ON" | "HAVING" => return SqlContext::Predicate,
            "INTO" => {
                if words
                    .get(i + 1)
                    .is_some_and(|w| w.to_uppercase() == "INSERT")
                {
                    if idents_before_keyword == 0 {
                        return SqlContext::TableTarget;
                    }
                    return SqlContext::General;
                }
                if idents_before_keyword == 0 {
                    return SqlContext::TableRef;
                }
                return SqlContext::SelectList;
            }
            "UPDATE" => {
                if idents_before_keyword == 0 {
                    return SqlContext::TableTarget;
                }
                return SqlContext::General;
            }
            "SET" => {
                if let Some(table) = find_update_table(&words, i) {
                    return SqlContext::SetClause {
                        update_table: table,
                    };
                }
                return SqlContext::Predicate;
            }
            "BY" => {
                if words.get(i + 1).is_some_and(|w| {
                    let u = w.to_uppercase();
                    u == "ORDER" || u == "GROUP"
                }) {
                    return SqlContext::OrderGroupBy;
                }
            }
            "ORDER" | "GROUP" => return SqlContext::OrderGroupBy,
            "EXEC" | "EXECUTE" | "CALL" => {
                if idents_before_keyword == 0 {
                    return SqlContext::ExecCall;
                }
                return SqlContext::General;
            }
            "CREATE" | "ALTER" | "DROP" => return SqlContext::DdlObject,
            _ => {}
        }
    }

    SqlContext::General
}

/// In a reverse word list, find the table name after UPDATE.
/// words[i] = "SET", words go backwards, so UPDATE is at higher index.
fn find_update_table(words: &[String], set_idx: usize) -> Option<String> {
    // After SET in reverse: table_name, UPDATE
    for j in (set_idx + 1)..words.len() {
        let upper = words[j].to_uppercase();
        if upper == "UPDATE" {
            // The word between UPDATE and SET is the table name
            if j > set_idx + 1 {
                return Some(words[set_idx + 1].clone());
            }
            return None;
        }
        if sql_tokens::is_sql_keyword(&upper) {
            break;
        }
    }
    None
}

/// Extract words from a line in reverse order.
fn extract_words_reverse(line: &str, words: &mut Vec<String>) {
    let bytes = line.as_bytes();
    let mut pos = bytes.len();

    while pos > 0 {
        while pos > 0 && !bytes[pos - 1].is_ascii_alphanumeric() && bytes[pos - 1] != b'_' {
            pos -= 1;
        }
        if pos == 0 {
            break;
        }
        let end = pos;
        while pos > 0 && (bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'_') {
            pos -= 1;
        }
        words.push(line[pos..end].to_string());
    }
}

// ---------------------------------------------------------------------------
// Prefix extraction
// ---------------------------------------------------------------------------

/// Extract the word prefix at cursor. Returns (prefix, start_col).
pub fn word_prefix_at_cursor(line: &str, col: usize) -> (&str, usize) {
    let bytes = line.as_bytes();
    let end = col.min(bytes.len());
    let mut start = end;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    (&line[start..end], start)
}

/// Check if cursor is immediately after a dot (for empty-prefix dot completions).
pub fn is_after_dot(line: &str, col: usize) -> bool {
    let bytes = line.as_bytes();
    col > 0 && col <= bytes.len() && bytes[col - 1] == b'.'
}

// ---------------------------------------------------------------------------
// Query block scoping
// ---------------------------------------------------------------------------

/// Find the range of the current query block around `row`.
/// Queries are separated by one or more blank lines.
fn query_block(lines: &[String], row: usize) -> (usize, usize) {
    let mut start = row;
    while start > 0 && !lines[start - 1].trim().is_empty() {
        start -= 1;
    }
    let mut end = row + 1;
    while end < lines.len() && !lines[end].trim().is_empty() {
        end += 1;
    }
    (start, end)
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Build context-aware completions.
pub fn build_completions(
    state: &AppState,
    lines: &[String],
    row: usize,
    col: usize,
) -> Vec<CompletionItem> {
    build_completions_inner(state, lines, row, col, false)
}

/// Build completions with force mode (Ctrl+Space opens even without prefix).
pub fn build_completions_forced(
    state: &AppState,
    lines: &[String],
    row: usize,
    col: usize,
) -> Vec<CompletionItem> {
    build_completions_inner(state, lines, row, col, true)
}

fn build_completions_inner(
    state: &AppState,
    lines: &[String],
    row: usize,
    col: usize,
    force: bool,
) -> Vec<CompletionItem> {
    if row >= lines.len() {
        return vec![];
    }
    let line = &lines[row];
    let (prefix, _) = word_prefix_at_cursor(line, col);
    let dot_mode = prefix.is_empty() && is_after_dot(line, col);

    if prefix.is_empty() && !dot_mode && !force {
        return vec![];
    }

    // Scope to the current query block
    let (block_start, block_end) = query_block(lines, row);
    let block: Vec<String> = lines[block_start..block_end].to_vec();
    let block_row = row - block_start;

    let context = detect_context(state, &block, block_row, col);

    match context {
        // SELECT col1, col2 ... (before FROM)
        SqlContext::SelectList => {
            let mut items = build_columns_from_query(state, &block, prefix);
            add_function_keywords(prefix, &mut items);
            for &kw in &[
                "FROM", "AS", "DISTINCT", "CASE", "WHEN", "THEN", "ELSE", "END", "NOT", "NULL",
                "TRUE", "FALSE",
            ] {
                add_keyword_if_match(kw, prefix, &mut items);
            }
            items
        }
        // WHERE x = 1 AND ... / ON a.id = b.id / HAVING ...
        SqlContext::Predicate => {
            let mut items = build_columns_from_query(state, &block, prefix);
            add_function_keywords(prefix, &mut items);
            for &kw in &[
                "AND",
                "OR",
                "NOT",
                "IN",
                "IS",
                "NULL",
                "TRUE",
                "FALSE",
                "LIKE",
                "BETWEEN",
                "EXISTS",
                "CASE",
                "WHEN",
                "THEN",
                "ELSE",
                "END",
                // Clause continuation (end the predicate, start new clause)
                "WHERE",
                "ORDER",
                "GROUP",
                "HAVING",
                "LIMIT",
                "OFFSET",
                "UNION",
                "INTERSECT",
                "EXCEPT",
            ] {
                add_keyword_if_match(kw, prefix, &mut items);
            }
            items
        }
        // FROM table [alias] / JOIN table [alias] ON ...
        // Clause continuation: only structural keywords
        SqlContext::AfterTable => {
            let mut items = Vec::new();
            for &kw in &[
                "WHERE",
                "JOIN",
                "LEFT",
                "RIGHT",
                "INNER",
                "CROSS",
                "FULL",
                "NATURAL",
                "ON",
                "ORDER",
                "GROUP",
                "HAVING",
                "LIMIT",
                "OFFSET",
                "UNION",
                "INTERSECT",
                "EXCEPT",
                "AS",
            ] {
                add_keyword_if_match(kw, prefix, &mut items);
            }
            items
        }
        // FROM / JOIN (no table written yet), also after LEFT/RIGHT/etc.
        SqlContext::TableRef => {
            let mut items = build_table_ref_completions(state, prefix);
            // Include JOIN keywords so "LEFT j|" suggests JOIN
            for &kw in &["JOIN", "OUTER"] {
                add_keyword_if_match(kw, prefix, &mut items);
            }
            items
        }
        // INSERT INTO / UPDATE (table expected)
        SqlContext::TableTarget => build_table_only_completions(state, prefix),
        // SET col = ... (in UPDATE)
        SqlContext::SetClause { ref update_table } => {
            build_column_completions(state, &block, update_table, prefix)
        }
        // ORDER BY / GROUP BY: columns + ASC/DESC/HAVING
        SqlContext::OrderGroupBy => {
            let mut items = build_columns_from_query(state, &block, prefix);
            for &kw in &["ASC", "DESC", "HAVING", "LIMIT", "OFFSET", "NULLS"] {
                add_keyword_if_match(kw, prefix, &mut items);
            }
            items
        }
        // EXEC / CALL: procedures and packages
        SqlContext::ExecCall => build_exec_completions(state, prefix),
        // CREATE / ALTER / DROP: object type keywords
        SqlContext::DdlObject => ddl_keyword_completions(prefix),
        // SCHEMA. → objects in that schema
        SqlContext::SchemaDot { ref schema_name } => {
            build_schema_dot_completions(state, schema_name, prefix)
        }
        // table. or alias. → columns
        SqlContext::ColumnDot { ref table_ref } => {
            build_column_completions(state, &block, table_ref, prefix)
        }
        // Unknown position: common statement starters
        SqlContext::General => {
            let mut items = Vec::new();
            for &kw in &[
                "SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "ALTER", "DROP", "BEGIN",
                "COMMIT", "ROLLBACK", "WITH", "EXPLAIN", "EXEC", "EXECUTE", "CALL", "GRANT",
                "REVOKE", "TRUNCATE", "DECLARE", "SET",
            ] {
                add_keyword_if_match(kw, prefix, &mut items);
            }
            items
        }
    }
}

// ---------------------------------------------------------------------------
// Builder functions
// ---------------------------------------------------------------------------

/// Columns or aliases from tables referenced in FROM/JOIN of the current query block.
/// - If a table has an alias → suggest the alias (user types `alias.` for columns)
/// - If no alias → suggest columns directly
fn build_columns_from_query(
    state: &AppState,
    block: &[String],
    prefix: &str,
) -> Vec<CompletionItem> {
    let prefix_upper = prefix.to_uppercase();
    let prefix_lower = prefix.to_lowercase();
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    let tables = extract_referenced_tables(block);
    for tref in &tables {
        if let Some(alias) = &tref.alias {
            // Table has alias → suggest the alias, user will do alias. for columns
            if matches_prefix(alias, &prefix_upper, &prefix_lower) && seen.insert(alias.clone()) {
                items.push(CompletionItem {
                    label: alias.clone(),
                    kind: CompletionKind::Alias,
                });
            }
        } else {
            // No alias → suggest columns directly
            add_columns_for_table(
                state,
                &tref.table_name,
                &prefix_upper,
                &prefix_lower,
                &mut items,
                &mut seen,
            );
        }
    }

    items
}

/// Columns of a specific table (for "table." or SET clause).
fn build_column_completions(
    state: &AppState,
    block: &[String],
    table_ref: &str,
    prefix: &str,
) -> Vec<CompletionItem> {
    let prefix_upper = prefix.to_uppercase();
    let prefix_lower = prefix.to_lowercase();
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    let resolved = resolve_table_name(block, table_ref);
    let table_name = resolved.as_deref().unwrap_or(table_ref);
    add_columns_for_table(
        state,
        table_name,
        &prefix_upper,
        &prefix_lower,
        &mut items,
        &mut seen,
    );

    // Also check QueryResult columns from script result tabs
    if items.is_empty() {
        for tab in &state.tabs {
            for rt in &tab.result_tabs {
                for col_name in &rt.result.columns {
                    if matches_prefix(col_name, &prefix_upper, &prefix_lower)
                        && seen.insert(col_name.clone())
                    {
                        items.push(CompletionItem {
                            label: col_name.clone(),
                            kind: CompletionKind::Column,
                        });
                    }
                }
            }
        }
    }

    items
}

/// Look up columns for a table name in open Table tabs + column_cache.
fn add_columns_for_table(
    state: &AppState,
    table_name: &str,
    prefix_upper: &str,
    prefix_lower: &str,
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<String>,
) {
    let tbl_upper = table_name.to_uppercase();
    let tbl_lower = table_name.to_lowercase();

    // 1. Search in open Table tabs
    for tab in &state.tabs {
        if let TabKind::Table { table, .. } = &tab.kind
            && (table.to_uppercase() == tbl_upper || table.to_lowercase() == tbl_lower)
        {
            for col in &tab.columns {
                if (prefix_upper.is_empty()
                    || matches_prefix(&col.name, prefix_upper, prefix_lower))
                    && seen.insert(col.name.clone())
                {
                    items.push(CompletionItem {
                        label: col.name.clone(),
                        kind: CompletionKind::Column,
                    });
                }
            }
            if !items.is_empty() {
                return;
            }
        }
    }

    // 2. Search in column_cache
    for (cache_key, cols) in &state.column_cache {
        // Key format: "SCHEMA.TABLE"
        if let Some(dot) = cache_key.find('.') {
            let cached_table = &cache_key[dot + 1..];
            if cached_table == tbl_upper {
                for col in cols {
                    if (prefix_upper.is_empty()
                        || matches_prefix(&col.name, prefix_upper, prefix_lower))
                        && seen.insert(col.name.clone())
                    {
                        items.push(CompletionItem {
                            label: col.name.clone(),
                            kind: CompletionKind::Column,
                        });
                    }
                }
                return;
            }
        }
    }
}

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

/// Tables/views after FROM/JOIN — dialect-aware.
/// Oracle/PG: schemas + tables/views from current_schema + filtered schemas.
/// MySQL: tables/views directly.
fn build_table_ref_completions(state: &AppState, prefix: &str) -> Vec<CompletionItem> {
    let prefix_upper = prefix.to_uppercase();
    let prefix_lower = prefix.to_lowercase();
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    let eff_conn = effective_conn_name(state);
    let conn_name = match &eff_conn {
        Some(n) => n.as_str(),
        None => return items,
    };

    let is_oracle_or_pg = matches!(
        state.db_type,
        Some(DatabaseType::Oracle) | Some(DatabaseType::PostgreSQL)
    );

    if is_oracle_or_pg {
        // Suggest schemas
        add_schemas(
            state,
            conn_name,
            &prefix_upper,
            &prefix_lower,
            &mut items,
            &mut seen,
        );
    }

    // Tables and views (prioritize current_schema)
    add_filtered_leaves(
        state,
        conn_name,
        &prefix_upper,
        &prefix_lower,
        &[LeafKind::Table, LeafKind::View],
        &mut items,
        &mut seen,
    );

    items
}

/// Tables only (INSERT INTO / UPDATE) — no schemas.
fn build_table_only_completions(state: &AppState, prefix: &str) -> Vec<CompletionItem> {
    let prefix_upper = prefix.to_uppercase();
    let prefix_lower = prefix.to_lowercase();
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    let eff_conn = effective_conn_name(state);
    let conn_name = match &eff_conn {
        Some(n) => n.as_str(),
        None => return items,
    };

    add_filtered_leaves(
        state,
        conn_name,
        &prefix_upper,
        &prefix_lower,
        &[LeafKind::Table, LeafKind::View],
        &mut items,
        &mut seen,
    );

    items
}

/// All objects inside a specific schema (after "SCHEMA.").
/// Does NOT check schema filter (user explicitly typed the schema).
fn build_schema_dot_completions(
    state: &AppState,
    schema_name: &str,
    prefix: &str,
) -> Vec<CompletionItem> {
    let prefix_upper = prefix.to_uppercase();
    let prefix_lower = prefix.to_lowercase();
    let schema_upper = schema_name.to_uppercase();
    let schema_lower = schema_name.to_lowercase();
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    let eff_conn = effective_conn_name(state);
    let conn_name = match &eff_conn {
        Some(n) => n.as_str(),
        None => return items,
    };

    for node in &state.tree {
        if let TreeNode::Leaf {
            name, schema, kind, ..
        } = node
        {
            if schema.to_uppercase() != schema_upper && schema.to_lowercase() != schema_lower {
                continue;
            }
            // Check leaf-level filter
            let (cat_suffix, comp_kind) = leaf_kind_info(kind);
            let cat_key = format!("{conn_name}::{schema}.{cat_suffix}");
            if !state.object_filter.is_enabled(&cat_key, name) {
                continue;
            }
            if (prefix_upper.is_empty() || matches_prefix(name, &prefix_upper, &prefix_lower))
                && seen.insert(name.clone())
            {
                items.push(CompletionItem {
                    label: name.clone(),
                    kind: comp_kind,
                });
            }
        }
    }

    items
}

/// Procedures and packages (after EXEC/CALL).
fn build_exec_completions(state: &AppState, prefix: &str) -> Vec<CompletionItem> {
    let prefix_upper = prefix.to_uppercase();
    let prefix_lower = prefix.to_lowercase();
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    let eff_conn = effective_conn_name(state);
    let conn_name = match &eff_conn {
        Some(n) => n.as_str(),
        None => return items,
    };

    add_filtered_leaves(
        state,
        conn_name,
        &prefix_upper,
        &prefix_lower,
        &[LeafKind::Procedure, LeafKind::Package, LeafKind::Function],
        &mut items,
        &mut seen,
    );

    items
}

/// DDL keywords after CREATE/ALTER/DROP.
fn ddl_keyword_completions(prefix: &str) -> Vec<CompletionItem> {
    let prefix_upper = prefix.to_uppercase();
    let mut items = Vec::new();
    for &kw in &[
        "TABLE",
        "VIEW",
        "INDEX",
        "SEQUENCE",
        "TRIGGER",
        "SCHEMA",
        "DATABASE",
        "PROCEDURE",
        "FUNCTION",
        "PACKAGE",
        "TYPE",
    ] {
        if kw.starts_with(&prefix_upper) {
            items.push(CompletionItem {
                label: kw.to_string(),
                kind: CompletionKind::Keyword,
            });
        }
    }
    items
}

/// Add SQL function keywords to items.
fn add_function_keywords(prefix: &str, items: &mut Vec<CompletionItem>) {
    let prefix_upper = prefix.to_uppercase();
    for &kw in FUNCTION_KEYWORDS {
        if kw.starts_with(&prefix_upper) {
            items.push(CompletionItem {
                label: kw.to_string(),
                kind: CompletionKind::Function,
            });
        }
    }
}

fn add_keyword_if_match(kw: &str, prefix: &str, items: &mut Vec<CompletionItem>) {
    if kw.starts_with(&prefix.to_uppercase()) {
        items.push(CompletionItem {
            label: kw.to_string(),
            kind: CompletionKind::Keyword,
        });
    }
}

// ---------------------------------------------------------------------------
// Helpers: tree traversal
// ---------------------------------------------------------------------------

/// Add schemas from tree (filtered).
fn add_schemas(
    state: &AppState,
    conn_name: &str,
    prefix_upper: &str,
    prefix_lower: &str,
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<String>,
) {
    let key = format!("{conn_name}::schemas");
    for node in &state.tree {
        if let TreeNode::Schema { name, .. } = node
            && state.object_filter.is_enabled(&key, name)
            && matches_prefix(name, prefix_upper, prefix_lower)
            && seen.insert(name.clone())
        {
            items.push(CompletionItem {
                label: name.clone(),
                kind: CompletionKind::Schema,
            });
        }
    }
}

/// Add filtered leaves of specified kinds from all schemas.
fn add_filtered_leaves(
    state: &AppState,
    conn_name: &str,
    prefix_upper: &str,
    prefix_lower: &str,
    kinds: &[LeafKind],
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<String>,
) {
    for node in &state.tree {
        if let TreeNode::Leaf {
            name, schema, kind, ..
        } = node
        {
            if !kinds.contains(kind) {
                continue;
            }
            let (cat_suffix, comp_kind) = leaf_kind_info(kind);
            let cat_key = format!("{conn_name}::{schema}.{cat_suffix}");
            if !state.object_filter.is_enabled(&cat_key, name) {
                continue;
            }
            if matches_prefix(name, prefix_upper, prefix_lower) && seen.insert(name.clone()) {
                items.push(CompletionItem {
                    label: name.clone(),
                    kind: comp_kind,
                });
            }
        }
    }
}

fn leaf_kind_info(kind: &LeafKind) -> (&str, CompletionKind) {
    match kind {
        LeafKind::Table => ("Tables", CompletionKind::Table),
        LeafKind::View => ("Views", CompletionKind::View),
        LeafKind::MaterializedView => ("Materialized Views", CompletionKind::Table),
        LeafKind::Package => ("Packages", CompletionKind::Package),
        LeafKind::Function => ("Functions", CompletionKind::Function),
        LeafKind::Procedure => ("Procedures", CompletionKind::Procedure),
        LeafKind::Index
        | LeafKind::Sequence
        | LeafKind::Type
        | LeafKind::Trigger
        | LeafKind::Event => {
            ("", CompletionKind::Table) // not relevant for completion
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers: table reference resolution
// ---------------------------------------------------------------------------

/// A table reference with optional alias from FROM/JOIN.
struct TableRef {
    table_name: String,
    alias: Option<String>,
}

/// Extract all table references with aliases from FROM/JOIN clauses.
fn extract_referenced_tables(lines: &[String]) -> Vec<TableRef> {
    let full_text: String = lines.join(" ");
    let words: Vec<&str> = full_text.split_whitespace().collect();
    let upper_words: Vec<String> = words.iter().map(|w| w.to_uppercase()).collect();
    let mut tables = Vec::new();

    let mut i = 0;
    while i < words.len() {
        if matches!(upper_words[i].as_str(), "FROM" | "JOIN") {
            i += 1;
            while i < words.len() {
                if sql_tokens::is_sql_keyword(&upper_words[i]) {
                    break;
                }
                let token = words[i].trim_end_matches(',');
                let actual = token.rsplit('.').next().unwrap_or(token);
                if actual.is_empty() {
                    i += 1;
                    continue;
                }
                let table_name = actual.to_string();
                i += 1;

                // Check for alias
                let alias = if i < words.len() && !sql_tokens::is_sql_keyword(&upper_words[i]) {
                    if upper_words[i] == "AS" {
                        i += 1; // skip AS
                        if i < words.len() && !sql_tokens::is_sql_keyword(&upper_words[i]) {
                            let a = words[i].trim_end_matches(',').to_string();
                            i += 1;
                            Some(a)
                        } else {
                            None
                        }
                    } else if !words[i - 1].ends_with(',') && words[i] != "," {
                        let a = words[i].trim_end_matches(',').to_string();
                        i += 1;
                        Some(a)
                    } else {
                        None
                    }
                } else {
                    None
                };

                tables.push(TableRef { table_name, alias });

                // Check for comma continuation
                if i > 0 && words[i - 1].ends_with(',') {
                    continue;
                }
                if i < words.len() && words[i] == "," {
                    i += 1;
                    continue;
                }
                break;
            }
        } else {
            i += 1;
        }
    }

    tables
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

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

fn matches_prefix(name: &str, prefix_upper: &str, prefix_lower: &str) -> bool {
    if prefix_upper.is_empty() {
        return true;
    }
    name.to_uppercase().starts_with(prefix_upper) || name.to_lowercase().starts_with(prefix_lower)
}

// ---------------------------------------------------------------------------
// Keyword lists
// ---------------------------------------------------------------------------

const FUNCTION_KEYWORDS: &[&str] = &[
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "NVL",
    "NVL2",
    "DECODE",
    "COALESCE",
    "NULLIF",
    "TO_CHAR",
    "TO_DATE",
    "TO_NUMBER",
    "SUBSTR",
    "INSTR",
    "LENGTH",
    "TRIM",
    "UPPER",
    "LOWER",
    "CONCAT",
    "REPLACE",
    "LPAD",
    "RPAD",
    "ROUND",
    "TRUNC",
    "CAST",
    "CASE",
];
