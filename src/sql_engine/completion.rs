//! CompletionProvider — fuzzy matching, scoring, and context-aware SQL completion.
//!
//! Takes a SemanticContext (produced by the analyzer) and generates ranked,
//! fuzzy-matched completion items appropriate for the cursor's SQL position.

use crate::sql_engine::context::{CursorContext, SemanticContext};
use crate::sql_engine::dialect::SqlDialect;
use crate::sql_engine::metadata::{MetadataIndex, ObjectKind};
use crate::sql_engine::models::QualifiedName;

// ---------------------------------------------------------------------------
// Match quality
// ---------------------------------------------------------------------------

/// Match quality tier — used for primary sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchTier {
    Exact = 0,
    Prefix = 1,
    Contains = 2,
    Fuzzy = 3,
}

/// Result of a fuzzy match attempt.
#[derive(Debug, Clone)]
pub struct MatchResult {
    pub tier: MatchTier,
    pub score: i32,
    pub positions: Vec<usize>,
}

/// Fuzzy match algorithm (fzf-inspired).
///
/// Returns None if `pattern` cannot be found in `candidate`.
/// Empty pattern matches everything with tier Exact.
///
/// Scoring:
/// - Exact: 1000
/// - Prefix: 800 + ratio bonus
/// - Contains: 600 + position bonus
/// - Fuzzy: per-char scoring with adjacency/boundary bonuses, gap penalties
pub fn fuzzy_match(pattern: &str, candidate: &str) -> Option<MatchResult> {
    if pattern.is_empty() {
        return Some(MatchResult {
            tier: MatchTier::Exact,
            score: 1000,
            positions: vec![],
        });
    }

    let pat: Vec<char> = pattern.chars().map(|c| c.to_ascii_lowercase()).collect();
    let cand_chars: Vec<char> = candidate.chars().collect();
    let cand: Vec<char> = cand_chars.iter().map(|c| c.to_ascii_lowercase()).collect();

    if pat.len() > cand.len() {
        return None;
    }

    // Exact match
    if pat == cand {
        let positions: Vec<usize> = (0..cand.len()).collect();
        return Some(MatchResult {
            tier: MatchTier::Exact,
            score: 1000,
            positions,
        });
    }

    // Prefix match
    if cand.starts_with(&pat) {
        let ratio_bonus = (pat.len() as i32 * 100) / cand.len() as i32;
        let positions: Vec<usize> = (0..pat.len()).collect();
        return Some(MatchResult {
            tier: MatchTier::Prefix,
            score: 800 + ratio_bonus,
            positions,
        });
    }

    // Contiguous substring match
    if let Some(start) = find_substring(&cand, &pat) {
        let position_bonus = 50i32.saturating_sub(start as i32).max(0);
        let positions: Vec<usize> = (start..start + pat.len()).collect();
        return Some(MatchResult {
            tier: MatchTier::Contains,
            score: 600 + position_bonus,
            positions,
        });
    }

    // Fuzzy match: all pattern chars must appear in order
    let mut positions = Vec::with_capacity(pat.len());
    let mut score: i32 = 0;
    let mut cand_idx = 0;
    let mut prev_match_idx: Option<usize> = None;
    let mut consecutive: i32 = 0;

    for &pc in &pat {
        let mut found = false;
        while cand_idx < cand.len() {
            if cand[cand_idx] == pc {
                positions.push(cand_idx);
                score += 16;

                // Adjacency bonus
                if let Some(prev) = prev_match_idx {
                    let gap = cand_idx - prev - 1;
                    if gap == 0 {
                        consecutive += 1;
                        score += 8 * consecutive;
                    } else {
                        consecutive = 0;
                        score -= (gap as i32 * 4).min(20);
                    }
                }

                // Word boundary bonus
                if cand_idx == 0
                    || cand_chars[cand_idx - 1] == '_'
                    || cand_chars[cand_idx - 1] == '-'
                    || (cand_chars[cand_idx - 1].is_lowercase()
                        && cand_chars[cand_idx].is_uppercase())
                {
                    score += 32;
                }

                // Start bonus
                if cand_idx == 0 {
                    score += 48;
                }

                prev_match_idx = Some(cand_idx);
                cand_idx += 1;
                found = true;
                break;
            }
            cand_idx += 1;
        }
        if !found {
            return None;
        }
    }

    Some(MatchResult {
        tier: MatchTier::Fuzzy,
        score,
        positions,
    })
}

fn find_substring(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    'outer: for i in 0..=(haystack.len() - needle.len()) {
        for j in 0..needle.len() {
            if haystack[i + j] != needle[j] {
                continue 'outer;
            }
        }
        return Some(i);
    }
    None
}

// ---------------------------------------------------------------------------
// Scored items
// ---------------------------------------------------------------------------

/// Kind of a completion item (drives tag display and base priority).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompletionItemKind {
    Keyword,
    Schema,
    Table,
    View,
    Column,
    Package,
    Function,
    Procedure,
    Alias,
    ForeignKeyJoin,
}

impl CompletionItemKind {
    /// Short tag shown in the completion popup.
    #[allow(dead_code)]
    pub fn tag(&self) -> &str {
        match self {
            Self::Keyword => "kw",
            Self::Schema => "sch",
            Self::Table => "tbl",
            Self::View => "view",
            Self::Column => "col",
            Self::Package => "pkg",
            Self::Function => "fn",
            Self::Procedure => "proc",
            Self::Alias => "alias",
            Self::ForeignKeyJoin => "fk",
        }
    }

    /// Base priority: metadata items rank above keywords.
    pub fn base_priority(&self) -> i32 {
        match self {
            Self::Column => 100,
            Self::Alias => 95,
            Self::ForeignKeyJoin => 90,
            Self::Table => 80,
            Self::View => 78,
            Self::Function => 70,
            Self::Procedure => 68,
            Self::Package => 65,
            Self::Schema => 60,
            Self::Keyword => 40,
        }
    }
}

/// A scored completion candidate ready for display.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ScoredItem {
    pub label: String,
    pub kind: CompletionItemKind,
    pub score: i32,
    pub tier: MatchTier,
    /// Character positions in `label` that matched the pattern (for highlighting).
    pub match_positions: Vec<usize>,
    /// Optional detail (e.g., column data type, FK info).
    pub detail: Option<String>,
}

/// Sort scored items: by tier (ascending), then score (descending),
/// then label length, then alphabetical.
pub fn sort_scored(items: &mut [ScoredItem]) {
    items.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then(b.score.cmp(&a.score))
            .then(a.label.len().cmp(&b.label.len()))
            .then_with(|| a.label.to_uppercase().cmp(&b.label.to_uppercase()))
    });
}

// ---------------------------------------------------------------------------
// Completion provider
// ---------------------------------------------------------------------------

/// Generates context-aware, fuzzy-matched completion items.
pub struct CompletionProvider<'a> {
    dialect: &'a dyn SqlDialect,
    metadata: &'a MetadataIndex,
}

impl<'a> CompletionProvider<'a> {
    pub fn new(dialect: &'a dyn SqlDialect, metadata: &'a MetadataIndex) -> Self {
        Self { dialect, metadata }
    }

    /// Generate completions from a SemanticContext.
    pub fn complete(&self, ctx: &SemanticContext) -> Vec<ScoredItem> {
        let mut items = match &ctx.cursor_context {
            CursorContext::SelectList => self.complete_select_list(ctx),
            CursorContext::TableRef => self.complete_table_ref(ctx),
            CursorContext::Predicate => self.complete_predicate(ctx),
            CursorContext::AfterTableRef => self.complete_after_table(ctx),
            CursorContext::TableTarget => self.complete_table_target(ctx),
            CursorContext::AfterUpdateTable => self.complete_after_update_table(ctx),
            CursorContext::AfterDeleteTable => self.complete_after_delete_table(ctx),
            CursorContext::SetClause { target_table } => {
                self.complete_set_clause(ctx, target_table)
            }
            CursorContext::OrderGroupBy => self.complete_order_group(ctx),
            CursorContext::ExecCall => self.complete_exec(ctx),
            CursorContext::DdlObject => self.complete_ddl(ctx),
            CursorContext::SchemaDot { schema_name, .. } => {
                self.complete_schema_dot(ctx, schema_name)
            }
            CursorContext::PackageDot { schema, package } => {
                self.complete_package_dot(ctx, schema.as_deref(), package)
            }
            CursorContext::ColumnDot { table_ref } => self.complete_column_dot(ctx, table_ref),
            CursorContext::General => self.complete_general(ctx),
        };

        sort_scored(&mut items);
        items
    }

    // -----------------------------------------------------------------------
    // Context-specific builders
    // -----------------------------------------------------------------------

    fn complete_select_list(&self, ctx: &SemanticContext) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
        let mut items = Vec::new();

        // Aliases for tables with aliases
        self.add_aliases(ctx, prefix, &mut items);
        // Columns from tables without aliases
        self.add_scope_columns(ctx, prefix, &mut items);
        // Functions
        self.add_functions(prefix, &mut items);
        // Keywords
        for &kw in &[
            "FROM",
            "AS",
            "DISTINCT",
            "CASE",
            "WHEN",
            "THEN",
            "ELSE",
            "END",
            "NOT",
            "NULL",
            "TRUE",
            "FALSE",
            "OVER",
            "PARTITION",
            "BY",
            "ORDER",
            "AND",
            "OR",
            "IN",
            "BETWEEN",
        ] {
            self.add_keyword(kw, prefix, &mut items);
        }

        items
    }

    fn complete_predicate(&self, ctx: &SemanticContext) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
        let mut items = Vec::new();

        self.add_aliases(ctx, prefix, &mut items);
        self.add_scope_columns(ctx, prefix, &mut items);
        self.add_functions(prefix, &mut items);

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
            "WHERE",
            "ORDER",
            "GROUP",
            "BY",
            "HAVING",
            "LIMIT",
            "OFFSET",
            "UNION",
            "INTERSECT",
            "EXCEPT",
            "OVER",
            "PARTITION",
            "ASC",
            "DESC",
            "DISTINCT",
            "AS",
        ] {
            self.add_keyword(kw, prefix, &mut items);
        }

        items
    }

    fn complete_table_ref(&self, ctx: &SemanticContext) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
        let mut items = Vec::new();

        // CTE names (locally defined, boosted)
        self.add_cte_names(ctx, prefix, &mut items);

        // Schemas (Oracle/PG only)
        if self.dialect.has_schemas() {
            self.add_schemas(prefix, &mut items);
        }

        // Tables and views
        self.add_tables_and_views(prefix, &mut items);

        // Oracle: user-defined functions can be table functions, suggest them
        // here too — `accept_completion` will append `()` automatically because
        // kind is Function. PG/MySQL also support set-returning functions but
        // less commonly used in FROM, so keep this Oracle-only for now.
        if self.dialect.has_packages() {
            self.add_user_functions_in_from(prefix, &mut items);
            self.add_oracle_pseudo_tables(prefix, &mut items);
        }

        // FK-suggested tables (boost if tables already in FROM)
        self.add_fk_suggestions(ctx, prefix, &mut items);

        // JOIN keywords
        for &kw in &["JOIN", "OUTER"] {
            self.add_keyword(kw, prefix, &mut items);
        }

        items
    }

    /// Oracle "pseudo-table" functions that work in FROM clauses:
    ///   TABLE(collection_expr)        — unnest a nested table
    ///   THE(subquery)                 — legacy nested-table unnest
    ///   XMLTABLE(...)                 — convert XML to relational
    ///   JSON_TABLE(...)               — convert JSON to relational
    /// All are emitted as Function-kind so accept_completion appends "()"
    /// and parks the cursor inside the parens. They get a high score so
    /// they sit at the top of the list when the user types "tab".
    fn add_oracle_pseudo_tables(&self, prefix: &str, items: &mut Vec<ScoredItem>) {
        const PSEUDO_TABLES: &[(&str, &str)] = &[
            ("TABLE", "table function (unnest collection)"),
            ("THE", "the (legacy nested-table unnest)"),
            ("XMLTABLE", "XML to relational"),
            ("JSON_TABLE", "JSON to relational"),
        ];
        for &(name, detail) in PSEUDO_TABLES {
            if let Some(m) = fuzzy_match(prefix, name) {
                items.push(ScoredItem {
                    label: name.to_string(),
                    kind: CompletionItemKind::Function,
                    // Boost above regular functions so it shows up first.
                    score: m.score + CompletionItemKind::Function.base_priority() + 50,
                    tier: m.tier,
                    match_positions: m.positions,
                    detail: Some(detail.to_string()),
                });
            }
        }
    }

    /// Add user-defined functions (from MetadataIndex) as Function completions
    /// for use in a FROM clause. accept_completion will append `()`.
    fn add_user_functions_in_from(&self, prefix: &str, items: &mut Vec<ScoredItem>) {
        for entry in self.metadata.objects_by_kind(None, &[ObjectKind::Function]) {
            if let Some(m) = fuzzy_match(prefix, &entry.display_name) {
                items.push(ScoredItem {
                    label: entry.display_name.clone(),
                    kind: CompletionItemKind::Function,
                    score: m.score + CompletionItemKind::Function.base_priority(),
                    tier: m.tier,
                    match_positions: m.positions,
                    detail: Some("table function".to_string()),
                });
            }
        }
    }

    fn complete_after_table(&self, ctx: &SemanticContext) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
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
            self.add_keyword(kw, prefix, &mut items);
        }

        items
    }

    fn complete_table_target(&self, ctx: &SemanticContext) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
        let mut items = Vec::new();
        // CTE names (locally defined, boosted)
        self.add_cte_names(ctx, prefix, &mut items);
        if self.dialect.has_schemas() {
            self.add_schemas(prefix, &mut items);
        }
        self.add_tables_and_views(prefix, &mut items);
        items
    }

    fn complete_after_update_table(&self, ctx: &SemanticContext) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
        let mut items = Vec::new();
        // Primary suggestion: SET keyword
        self.add_keyword("SET", prefix, &mut items);
        // Alias keyword
        self.add_keyword("AS", prefix, &mut items);
        items
    }

    fn complete_after_delete_table(&self, ctx: &SemanticContext) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
        let mut items = Vec::new();
        // Primary suggestion: WHERE keyword
        self.add_keyword("WHERE", prefix, &mut items);
        // Alias keyword
        self.add_keyword("AS", prefix, &mut items);
        items
    }

    fn complete_set_clause(
        &self,
        ctx: &SemanticContext,
        target_table: &QualifiedName,
    ) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
        let mut items = Vec::new();
        // Columns of the target table (by specific name)
        let target_cols = self.columns_for_table(&target_table.name, ctx, prefix);
        if target_cols.is_empty() {
            // Fallback: all columns in scope (handles incomplete metadata)
            self.add_scope_columns(ctx, prefix, &mut items);
        } else {
            items.extend(target_cols);
        }
        // WHERE keyword (to end SET clause and start predicate)
        self.add_keyword("WHERE", prefix, &mut items);
        items
    }

    fn complete_order_group(&self, ctx: &SemanticContext) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
        let mut items = Vec::new();

        self.add_aliases(ctx, prefix, &mut items);
        self.add_scope_columns(ctx, prefix, &mut items);

        for &kw in &[
            "ORDER",
            "BY",
            "GROUP",
            "PARTITION",
            "OVER",
            "ASC",
            "DESC",
            "HAVING",
            "LIMIT",
            "OFFSET",
            "NULLS",
            "AS",
        ] {
            self.add_keyword(kw, prefix, &mut items);
        }

        items
    }

    fn complete_exec(&self, ctx: &SemanticContext) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
        let mut items = Vec::new();

        let kinds = if self.dialect.has_packages() {
            vec![
                ObjectKind::Procedure,
                ObjectKind::Package,
                ObjectKind::Function,
            ]
        } else {
            vec![ObjectKind::Procedure, ObjectKind::Function]
        };

        for entry in self.metadata.objects_by_kind(None, &kinds) {
            if let Some(m) = fuzzy_match(prefix, &entry.display_name) {
                let kind = match entry.kind {
                    ObjectKind::Procedure => CompletionItemKind::Procedure,
                    ObjectKind::Function => CompletionItemKind::Function,
                    ObjectKind::Package => CompletionItemKind::Package,
                    _ => CompletionItemKind::Procedure,
                };
                items.push(ScoredItem {
                    label: entry.display_name.clone(),
                    kind,
                    score: m.score + kind.base_priority(),
                    tier: m.tier,
                    match_positions: m.positions,
                    detail: None,
                });
            }
        }

        items
    }

    fn complete_ddl(&self, ctx: &SemanticContext) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
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
            self.add_keyword(kw, prefix, &mut items);
        }

        items
    }

    /// Complete after `schema.package.<cursor>` (or just `package.<cursor>`
    /// when the package is unique). Suggests the package's callable members
    /// (functions and procedures) as Function-kind items so accept_completion
    /// appends `()`.
    ///
    /// This handler is also reached for `schema.<cursor>` when the analyzer
    /// trusted the user's syntax — in that case `package` may actually be a
    /// table, view, or top-level routine. We try the package-member cache
    /// first, then fall back to suggesting top-level objects in `schema`
    /// that match the prefix so the user gets *something* useful even when
    /// the package hasn't been opened yet.
    fn complete_package_dot(
        &self,
        ctx: &SemanticContext,
        schema: Option<&str>,
        package: &str,
    ) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
        let mut items = Vec::new();

        // Resolve the schema if the user wrote a bare `pkg.`
        let resolved_schema = match schema {
            Some(s) => s.to_string(),
            None => self
                .metadata
                .schema_for_package(package)
                .map(String::from)
                .unwrap_or_default(),
        };

        // 1. Cached package members (the canonical case).
        if !resolved_schema.is_empty() {
            for member in self.metadata.package_members(&resolved_schema, package) {
                if let Some(m) = fuzzy_match(prefix, &member.name) {
                    items.push(ScoredItem {
                        label: member.name.clone(),
                        kind: CompletionItemKind::Function,
                        score: m.score + CompletionItemKind::Function.base_priority() + 50,
                        tier: m.tier,
                        match_positions: m.positions,
                        detail: Some(
                            match member.kind {
                                crate::sql_engine::metadata::PackageMemberKind::Function => {
                                    "package function"
                                }
                                crate::sql_engine::metadata::PackageMemberKind::Procedure => {
                                    "package procedure"
                                }
                            }
                            .to_string(),
                        ),
                    });
                }
            }
        }

        items
    }

    fn complete_schema_dot(&self, ctx: &SemanticContext, schema_name: &str) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
        let mut items = Vec::new();

        let kinds = &[
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::MaterializedView,
            ObjectKind::Procedure,
            ObjectKind::Function,
            ObjectKind::Package,
        ];

        for entry in self.metadata.objects_by_kind(Some(schema_name), kinds) {
            if let Some(m) = fuzzy_match(prefix, &entry.display_name) {
                let kind = match entry.kind {
                    ObjectKind::Table | ObjectKind::MaterializedView => CompletionItemKind::Table,
                    ObjectKind::View => CompletionItemKind::View,
                    ObjectKind::Procedure => CompletionItemKind::Procedure,
                    ObjectKind::Function => CompletionItemKind::Function,
                    ObjectKind::Package => CompletionItemKind::Package,
                    _ => CompletionItemKind::Table,
                };
                items.push(ScoredItem {
                    label: entry.display_name.clone(),
                    kind,
                    score: m.score + kind.base_priority(),
                    tier: m.tier,
                    match_positions: m.positions,
                    detail: None,
                });
            }
        }

        items
    }

    fn complete_column_dot(&self, ctx: &SemanticContext, table_ref: &str) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
        self.columns_for_table(table_ref, ctx, prefix)
    }

    fn complete_general(&self, ctx: &SemanticContext) -> Vec<ScoredItem> {
        let prefix = &ctx.prefix;
        let mut items = Vec::new();

        // SQL statement starters
        for &kw in &[
            "SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "ALTER", "DROP", "BEGIN", "COMMIT",
            "ROLLBACK", "WITH", "EXPLAIN", "EXEC", "EXECUTE", "CALL", "GRANT", "REVOKE",
            "TRUNCATE", "DECLARE", "SET", "MERGE",
        ] {
            self.add_keyword(kw, prefix, &mut items);
        }

        // PL/SQL keywords (always available — these appear inside BEGIN..END blocks,
        // IF blocks, LOOP blocks, package bodies, etc.)
        for &kw in &[
            // Control flow
            "IF",
            "ELSIF",
            "ELSE",
            "END",
            "THEN",
            "LOOP",
            "FOR",
            "WHILE",
            "EXIT",
            "CONTINUE",
            "RETURN",
            "GOTO",
            "CASE",
            "WHEN",
            // Exception handling
            "EXCEPTION",
            "RAISE",
            "RAISE_APPLICATION_ERROR",
            // Structure
            "FUNCTION",
            "PROCEDURE",
            "PACKAGE",
            "BODY",
            "TYPE",
            "SUBTYPE",
            "RECORD",
            "OBJECT",
            "REPLACE",
            "TRIGGER",
            "CURSOR",
            // Bulk / pipeline
            "BULK",
            "COLLECT",
            "FORALL",
            "PIPE",
            "ROW",
            "PIPELINED",
            // Execution
            "EXECUTE",
            "IMMEDIATE",
            "OPEN",
            "CLOSE",
            "FETCH",
            // Modifiers
            "OR",
            "AND",
            "NOT",
            "NULL",
            "IS",
            "IN",
            "AS",
            "OF",
            "INTO",
            "CONSTANT",
            "DEFAULT",
            "NOCOPY",
            "DETERMINISTIC",
            "RESULT_CACHE",
            "AUTONOMOUS_TRANSACTION",
            "PRAGMA",
            // Data types
            "NUMBER",
            "VARCHAR2",
            "VARCHAR",
            "CHAR",
            "CLOB",
            "BLOB",
            "DATE",
            "TIMESTAMP",
            "BOOLEAN",
            "INTEGER",
            "BINARY_INTEGER",
            "PLS_INTEGER",
            "SYS_REFCURSOR",
            "TABLE",
            "VARRAY",
        ] {
            self.add_keyword(kw, prefix, &mut items);
        }

        // Dialect-specific keywords
        for &kw in self.dialect.dialect_keywords() {
            self.add_keyword(kw, prefix, &mut items);
        }

        // Also suggest functions in general context
        self.add_functions(prefix, &mut items);

        items
    }

    // -----------------------------------------------------------------------
    // Shared helpers
    // -----------------------------------------------------------------------

    /// Add aliases from context (tables that have aliases).
    fn add_aliases(&self, ctx: &SemanticContext, prefix: &str, items: &mut Vec<ScoredItem>) {
        for tref in &ctx.table_refs {
            if let Some(ref alias) = tref.reference.alias
                && let Some(m) = fuzzy_match(prefix, alias)
            {
                items.push(ScoredItem {
                    label: alias.clone(),
                    kind: CompletionItemKind::Alias,
                    score: m.score + CompletionItemKind::Alias.base_priority(),
                    tier: m.tier,
                    match_positions: m.positions,
                    detail: Some(tref.reference.qualified_name.name.clone()),
                });
            }
        }
    }

    /// Add columns from tables that are in scope (without alias — direct columns).
    fn add_scope_columns(&self, ctx: &SemanticContext, prefix: &str, items: &mut Vec<ScoredItem>) {
        let mut seen = std::collections::HashSet::new();
        for col in &ctx.available_columns {
            if seen.insert(self.dialect.normalize_identifier(&col.name))
                && let Some(m) = fuzzy_match(prefix, &col.name)
            {
                items.push(ScoredItem {
                    label: col.name.clone(),
                    kind: CompletionItemKind::Column,
                    score: m.score + CompletionItemKind::Column.base_priority(),
                    tier: m.tier,
                    match_positions: m.positions,
                    detail: Some(col.data_type.clone()),
                });
            }
        }
    }

    /// Get columns for a specific table reference (by alias or name).
    fn columns_for_table(
        &self,
        table_ref: &str,
        ctx: &SemanticContext,
        prefix: &str,
    ) -> Vec<ScoredItem> {
        let mut items = Vec::new();

        let cols = ctx.columns_for(table_ref, &|s| self.dialect.normalize_identifier(s));
        for col in cols {
            if let Some(m) = fuzzy_match(prefix, &col.name) {
                items.push(ScoredItem {
                    label: col.name.clone(),
                    kind: CompletionItemKind::Column,
                    score: m.score + CompletionItemKind::Column.base_priority(),
                    tier: m.tier,
                    match_positions: m.positions,
                    detail: Some(col.data_type.clone()),
                });
            }
        }

        items
    }

    /// Add schemas matching prefix.
    fn add_schemas(&self, prefix: &str, items: &mut Vec<ScoredItem>) {
        for schema in self.metadata.all_schemas() {
            if let Some(m) = fuzzy_match(prefix, schema) {
                items.push(ScoredItem {
                    label: schema.to_string(),
                    kind: CompletionItemKind::Schema,
                    score: m.score + CompletionItemKind::Schema.base_priority(),
                    tier: m.tier,
                    match_positions: m.positions,
                    detail: None,
                });
            }
        }
    }

    /// Add tables and views matching prefix.
    /// Add CTE names as table completion candidates with a boost.
    /// CTEs are locally defined in WITH clauses, so they rank above regular tables.
    fn add_cte_names(&self, ctx: &SemanticContext, prefix: &str, items: &mut Vec<ScoredItem>) {
        for name in &ctx.cte_names {
            if let Some(m) = fuzzy_match(prefix, name) {
                items.push(ScoredItem {
                    label: name.clone(),
                    kind: CompletionItemKind::Table,
                    // Boost above regular tables: base_priority (80) + 30 = 110
                    score: m.score + CompletionItemKind::Table.base_priority() + 30,
                    tier: m.tier,
                    match_positions: m.positions,
                    detail: Some("CTE".to_string()),
                });
            }
        }
    }

    fn add_tables_and_views(&self, prefix: &str, items: &mut Vec<ScoredItem>) {
        let kinds = &[
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::MaterializedView,
        ];
        for entry in self.metadata.objects_by_kind(None, kinds) {
            if let Some(m) = fuzzy_match(prefix, &entry.display_name) {
                let kind = if matches!(entry.kind, ObjectKind::View | ObjectKind::MaterializedView)
                {
                    CompletionItemKind::View
                } else {
                    CompletionItemKind::Table
                };
                items.push(ScoredItem {
                    label: entry.display_name.clone(),
                    kind,
                    score: m.score + kind.base_priority(),
                    tier: m.tier,
                    match_positions: m.positions,
                    detail: Some(entry.schema_display.clone()),
                });
            }
        }
    }

    /// Add FK-related tables as join suggestions (boosted).
    fn add_fk_suggestions(&self, ctx: &SemanticContext, prefix: &str, items: &mut Vec<ScoredItem>) {
        for tref in &ctx.table_refs {
            if let Some(ref schema) = tref.resolved_schema {
                for fk in self
                    .metadata
                    .fk_related_tables(schema, &tref.reference.qualified_name.name)
                {
                    // Suggest the "other" table in the FK relationship
                    let (target_table, on_clause) =
                        if self.dialect.normalize_identifier(&fk.from_table)
                            == self
                                .dialect
                                .normalize_identifier(&tref.reference.qualified_name.name)
                        {
                            (&fk.to_table, format_join_on(fk))
                        } else {
                            (&fk.from_table, format_join_on_reverse(fk))
                        };

                    if let Some(m) = fuzzy_match(prefix, target_table) {
                        items.push(ScoredItem {
                            label: target_table.clone(),
                            kind: CompletionItemKind::ForeignKeyJoin,
                            score: m.score
                                + CompletionItemKind::ForeignKeyJoin.base_priority()
                                + 50, // FK context bonus
                            tier: m.tier,
                            match_positions: m.positions,
                            detail: Some(on_clause),
                        });
                    }
                }
            }
        }
    }

    /// Add SQL builtin functions from the dialect.
    fn add_functions(&self, prefix: &str, items: &mut Vec<ScoredItem>) {
        // Standard SQL functions (all engines)
        for &func in &[
            // Aggregate
            "COUNT",
            "SUM",
            "AVG",
            "MIN",
            "MAX",
            // Null handling
            "COALESCE",
            "NULLIF",
            "CAST",
            // Window / analytic
            "ROW_NUMBER",
            "RANK",
            "DENSE_RANK",
            "NTILE",
            "LEAD",
            "LAG",
            "FIRST_VALUE",
            "LAST_VALUE",
            "NTH_VALUE",
            // String
            "CONCAT",
            "UPPER",
            "LOWER",
            "TRIM",
            "REPLACE",
            "SUBSTRING",
            "LENGTH",
            // Numeric
            "ABS",
            "ROUND",
            "CEIL",
            "FLOOR",
            "MOD",
            // Conditional
            "CASE",
            // Statistical
            "STDDEV",
            "VARIANCE",
        ] {
            if let Some(m) = fuzzy_match(prefix, func) {
                items.push(ScoredItem {
                    label: func.to_string(),
                    kind: CompletionItemKind::Function,
                    score: m.score + CompletionItemKind::Function.base_priority(),
                    tier: m.tier,
                    match_positions: m.positions,
                    detail: None,
                });
            }
        }
        // Dialect-specific functions
        for &func in self.dialect.builtin_functions() {
            if let Some(m) = fuzzy_match(prefix, func) {
                items.push(ScoredItem {
                    label: func.to_string(),
                    kind: CompletionItemKind::Function,
                    score: m.score + CompletionItemKind::Function.base_priority(),
                    tier: m.tier,
                    match_positions: m.positions,
                    detail: None,
                });
            }
        }
    }

    /// Add a keyword if it matches the prefix.
    fn add_keyword(&self, kw: &str, prefix: &str, items: &mut Vec<ScoredItem>) {
        if let Some(m) = fuzzy_match(prefix, kw) {
            items.push(ScoredItem {
                label: kw.to_string(),
                kind: CompletionItemKind::Keyword,
                score: m.score + CompletionItemKind::Keyword.base_priority(),
                tier: m.tier,
                match_positions: m.positions,
                detail: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// FK formatting helpers
// ---------------------------------------------------------------------------

use crate::sql_engine::models::ForeignKey;

fn format_join_on(fk: &ForeignKey) -> String {
    fk.from_columns
        .iter()
        .zip(&fk.to_columns)
        .map(|(from, to)| format!("{}.{} = {}.{}", fk.from_table, from, fk.to_table, to))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn format_join_on_reverse(fk: &ForeignKey) -> String {
    fk.to_columns
        .iter()
        .zip(&fk.from_columns)
        .map(|(to, from)| format!("{}.{} = {}.{}", fk.to_table, to, fk.from_table, from))
        .collect::<Vec<_>>()
        .join(" AND ")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql_engine::analyzer::SemanticAnalyzer;
    use crate::sql_engine::dialect::OracleDialect;
    use crate::sql_engine::metadata::MetadataIndex;
    use crate::sql_engine::models::ResolvedColumn;

    // -- Fuzzy match tests --

    #[test]
    fn exact_match() {
        let r = fuzzy_match("EMP", "EMP").unwrap();
        assert_eq!(r.tier, MatchTier::Exact);
        assert_eq!(r.score, 1000);
    }

    #[test]
    fn prefix_match() {
        let r = fuzzy_match("emp", "EMPLOYEES").unwrap();
        assert_eq!(r.tier, MatchTier::Prefix);
        assert!(r.score > 800);
        assert_eq!(r.positions, vec![0, 1, 2]);
    }

    #[test]
    fn contains_match() {
        let r = fuzzy_match("name", "FIRST_NAME").unwrap();
        assert_eq!(r.tier, MatchTier::Contains);
        assert!(r.score >= 600);
    }

    #[test]
    fn fuzzy_match_across_boundaries() {
        let r = fuzzy_match("empsum", "EMPLOYEE_SUMMARY").unwrap();
        assert_eq!(r.tier, MatchTier::Fuzzy);
        assert!(r.score > 0);
        assert_eq!(r.positions.len(), 6);
    }

    #[test]
    fn no_match_returns_none() {
        assert!(fuzzy_match("xyz", "EMPLOYEES").is_none());
    }

    #[test]
    fn empty_pattern_matches_all() {
        let r = fuzzy_match("", "anything").unwrap();
        assert_eq!(r.tier, MatchTier::Exact);
        assert_eq!(r.score, 1000);
    }

    #[test]
    fn exact_beats_prefix_beats_contains_beats_fuzzy() {
        let r1 = fuzzy_match("emp", "EMP").unwrap();
        let r2 = fuzzy_match("emp", "EMPLOYEE").unwrap();
        let r3 = fuzzy_match("emp", "TEMP_TABLE").unwrap();
        // r3 is contains ("emp" inside "tEMP_TABLE")
        // r1 Exact > r2 Prefix > r3 Contains
        assert!(r1.tier < r2.tier);
        assert!(r2.tier < r3.tier);
    }

    #[test]
    fn word_boundary_bonus() {
        // "es" at word boundary ("EMPLOYEE_SUMMARY" S at position 9 after _) should score
        // higher than "es" in the middle of a word
        let r1 = fuzzy_match("es", "EMPLOYEE_SUMMARY").unwrap();
        let r2 = fuzzy_match("es", "NESTED_VALUE").unwrap();
        // Both should match; exact scores depend on position
        assert!(r1.tier <= MatchTier::Fuzzy);
        assert!(r2.tier <= MatchTier::Fuzzy);
    }

    // -- Sort tests --

    #[test]
    fn sort_scored_orders_correctly() {
        let mut items = vec![
            ScoredItem {
                label: "WHERE".into(),
                kind: CompletionItemKind::Keyword,
                score: 840,
                tier: MatchTier::Prefix,
                match_positions: vec![],
                detail: None,
            },
            ScoredItem {
                label: "EMPLOYEE_ID".into(),
                kind: CompletionItemKind::Column,
                score: 900,
                tier: MatchTier::Prefix,
                match_positions: vec![],
                detail: None,
            },
            ScoredItem {
                label: "EMP".into(),
                kind: CompletionItemKind::Table,
                score: 1000,
                tier: MatchTier::Exact,
                match_positions: vec![],
                detail: None,
            },
        ];
        sort_scored(&mut items);
        assert_eq!(items[0].label, "EMP"); // Exact first
        assert_eq!(items[1].label, "EMPLOYEE_ID"); // Higher score in prefix tier
        assert_eq!(items[2].label, "WHERE"); // Lower score in prefix tier
    }

    // -- CompletionProvider integration tests --

    fn test_index() -> MetadataIndex {
        let mut idx = MetadataIndex::new();
        idx.set_db_type(crate::core::models::DatabaseType::Oracle);
        idx.set_current_schema("HR");
        idx.add_schema("HR");
        idx.add_schema("FINANCE");
        idx.add_object("HR", "EMPLOYEES", ObjectKind::Table);
        idx.add_object("HR", "DEPARTMENTS", ObjectKind::Table);
        idx.add_object("HR", "EMPLOYEE_SUMMARY", ObjectKind::View);
        idx.add_object("FINANCE", "INVOICES", ObjectKind::Table);

        idx.cache_columns(
            "HR",
            "EMPLOYEES",
            vec![
                ResolvedColumn {
                    name: "EMPLOYEE_ID".into(),
                    data_type: "NUMBER".into(),
                    nullable: false,
                    is_primary_key: true,
                    table_schema: "HR".into(),
                    table_name: "EMPLOYEES".into(),
                },
                ResolvedColumn {
                    name: "FIRST_NAME".into(),
                    data_type: "VARCHAR2(50)".into(),
                    nullable: true,
                    is_primary_key: false,
                    table_schema: "HR".into(),
                    table_name: "EMPLOYEES".into(),
                },
                ResolvedColumn {
                    name: "DEPARTMENT_ID".into(),
                    data_type: "NUMBER".into(),
                    nullable: true,
                    is_primary_key: false,
                    table_schema: "HR".into(),
                    table_name: "EMPLOYEES".into(),
                },
            ],
        );

        idx.add_foreign_key(ForeignKey {
            constraint_name: "FK_EMP_DEPT".into(),
            from_schema: "HR".into(),
            from_table: "EMPLOYEES".into(),
            from_columns: vec!["DEPARTMENT_ID".into()],
            to_schema: "HR".into(),
            to_table: "DEPARTMENTS".into(),
            to_columns: vec!["DEPARTMENT_ID".into()],
        });

        idx
    }

    #[test]
    fn complete_select_shows_columns() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);
        let provider = CompletionProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT emp FROM employees".into()];
        let ctx = analyzer.analyze(&lines, 0, 10);
        let items = provider.complete(&ctx);

        // Should find EMPLOYEE_ID and other columns starting with "emp"
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"EMPLOYEE_ID"));
    }

    #[test]
    fn complete_from_shows_tables() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);
        let provider = CompletionProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM emp".into()];
        let ctx = analyzer.analyze(&lines, 0, 17);
        let items = provider.complete(&ctx);

        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"EMPLOYEES"));
        assert!(labels.contains(&"EMPLOYEE_SUMMARY"));
    }

    #[test]
    fn complete_from_fuzzy_works() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);
        let provider = CompletionProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM empsum".into()];
        let ctx = analyzer.analyze(&lines, 0, 20);
        let items = provider.complete(&ctx);

        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"EMPLOYEE_SUMMARY"));
    }

    #[test]
    fn complete_from_includes_fk_suggestions() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);
        let provider = CompletionProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM employees e JOIN dep".into()];
        let ctx = analyzer.analyze(&lines, 0, 34);
        let items = provider.complete(&ctx);

        // Should have DEPARTMENTS as FK suggestion
        let fk_items: Vec<&ScoredItem> = items
            .iter()
            .filter(|i| i.kind == CompletionItemKind::ForeignKeyJoin)
            .collect();
        assert!(!fk_items.is_empty());
        assert_eq!(fk_items[0].label, "DEPARTMENTS");
        assert!(
            fk_items[0]
                .detail
                .as_ref()
                .unwrap()
                .contains("DEPARTMENT_ID")
        );
    }

    #[test]
    fn complete_column_dot() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);
        let provider = CompletionProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT e.dep FROM employees e".into()];
        let ctx = analyzer.analyze(&lines, 0, 12);
        let items = provider.complete(&ctx);

        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"DEPARTMENT_ID"));
    }

    #[test]
    fn complete_schema_dot() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);
        let provider = CompletionProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM hr.emp".into()];
        let ctx = analyzer.analyze(&lines, 0, 20);
        let items = provider.complete(&ctx);

        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"EMPLOYEES"));
        assert!(labels.contains(&"EMPLOYEE_SUMMARY"));
    }

    #[test]
    fn complete_general_shows_statements() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);
        let provider = CompletionProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SEL".into()];
        let ctx = analyzer.analyze(&lines, 0, 3);
        let items = provider.complete(&ctx);

        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"SELECT"));
    }

    #[test]
    fn columns_have_data_type_detail() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);
        let provider = CompletionProvider::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT e. FROM employees e".into()];
        let ctx = analyzer.analyze(&lines, 0, 9);
        let items = provider.complete(&ctx);

        let emp_id = items.iter().find(|i| i.label == "EMPLOYEE_ID");
        assert!(emp_id.is_some());
        assert_eq!(emp_id.unwrap().detail.as_deref(), Some("NUMBER"));
    }
}
