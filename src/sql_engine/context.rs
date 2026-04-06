//! SemanticContext — the resolved semantic model of a SQL statement.
//!
//! Produced by the SemanticAnalyzer, consumed by CompletionProvider and
//! DiagnosticProvider. Contains resolved table references, aliases,
//! available columns, cursor context, and resolution errors.

use std::collections::HashMap;

use crate::sql_engine::models::{Location, QualifiedName, ResolvedColumn, TableReference};

// ---------------------------------------------------------------------------
// Cursor context
// ---------------------------------------------------------------------------

/// Identifies what the cursor is positioned at in the SQL.
/// Drives which completion items to suggest and how to validate.
#[derive(Debug, Clone)]
pub enum CursorContext {
    /// After SELECT (before FROM): columns, functions, expressions.
    SelectList,
    /// After FROM / JOIN: table references.
    TableRef,
    /// After WHERE / AND / OR / ON / HAVING: predicates.
    Predicate,
    /// After a complete table ref (suggest clause keywords).
    AfterTableRef,
    /// INSERT INTO / UPDATE target.
    TableTarget,
    /// SET clause in UPDATE.
    SetClause { target_table: QualifiedName },
    /// ORDER BY / GROUP BY.
    OrderGroupBy,
    /// EXEC / EXECUTE / CALL.
    ExecCall,
    /// After CREATE / ALTER / DROP.
    DdlObject,
    /// After "schema." — objects within that schema.
    SchemaDot { schema_name: String },
    /// After "table." or "alias." — columns of that table.
    ColumnDot { table_ref: String },
    /// No recognizable context.
    General,
}

// ---------------------------------------------------------------------------
// Semantic context
// ---------------------------------------------------------------------------

/// A table reference with its resolved schema and existence status.
#[derive(Debug, Clone)]
pub struct ResolvedTableRef {
    /// The original reference as found in the SQL text.
    pub reference: TableReference,
    /// The schema resolved by the engine (filled in from MetadataIndex if the
    /// reference was unqualified).
    pub resolved_schema: Option<String>,
    /// Whether this table was found in metadata. None = not yet checked.
    pub exists: Option<bool>,
}

/// An error found during semantic resolution.
#[derive(Debug, Clone)]
pub struct ResolutionError {
    pub location: Location,
    pub message: String,
    pub kind: ResolutionErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionErrorKind {
    UnknownSchema,
    UnknownTable,
    UnknownColumn,
    AmbiguousColumn,
}

/// Fully resolved semantic model of a SQL query block.
///
/// This is the shared intermediate representation consumed by both
/// CompletionProvider and DiagnosticProvider. Built once per query block
/// by the SemanticAnalyzer.
#[derive(Debug, Clone)]
pub struct SemanticContext {
    /// All table/view references in FROM/JOIN clauses, with resolved schemas.
    pub table_refs: Vec<ResolvedTableRef>,

    /// Alias → table mapping. Key is normalized (upper or lower per dialect).
    pub aliases: HashMap<String, QualifiedName>,

    /// The cursor's position context (for completion).
    pub cursor_context: CursorContext,

    /// Columns available in scope (union of all resolved table refs' columns).
    pub available_columns: Vec<ResolvedColumn>,

    /// Errors found during semantic resolution (unknown tables, schemas, etc.).
    pub resolution_errors: Vec<ResolutionError>,

    /// The prefix being typed at cursor (for filtering completions).
    pub prefix: String,

    /// Whether this is a partial/incomplete statement (SQL that sqlparser
    /// couldn't parse — fell back to token-based analysis).
    pub is_partial: bool,
}

impl SemanticContext {
    pub fn empty() -> Self {
        Self {
            table_refs: Vec::new(),
            aliases: HashMap::new(),
            cursor_context: CursorContext::General,
            available_columns: Vec::new(),
            resolution_errors: Vec::new(),
            prefix: String::new(),
            is_partial: true,
        }
    }

    /// Resolve an alias or table name to its qualified name.
    pub fn resolve_alias(
        &self,
        name: &str,
        normalize: &dyn Fn(&str) -> String,
    ) -> Option<&QualifiedName> {
        let normalized = normalize(name);
        self.aliases.get(&normalized)
    }

    /// Get columns for a specific table reference (by alias or name).
    pub fn columns_for(
        &self,
        table_ref: &str,
        normalize: &dyn Fn(&str) -> String,
    ) -> Vec<&ResolvedColumn> {
        let normalized = normalize(table_ref);
        // First check aliases
        let target = self
            .aliases
            .get(&normalized)
            .map(|qn| normalize(&qn.name))
            .unwrap_or_else(|| normalized.clone());

        self.available_columns
            .iter()
            .filter(|c| normalize(&c.table_name) == target)
            .collect()
    }
}
