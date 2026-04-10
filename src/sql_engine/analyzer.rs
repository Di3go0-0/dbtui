//! SemanticAnalyzer — converts SQL text + cursor position into a SemanticContext.
//!
//! Uses sqlparser for the heavy lifting (extracting FROM/JOIN table refs from
//! the AST), with token-based fallback for incomplete SQL and Oracle PL/SQL
//! that sqlparser cannot parse.

use sqlparser::ast::{self, SetExpr, Statement, TableFactor, TableWithJoins};
use sqlparser::parser::Parser;

use crate::sql_engine::context::*;
use crate::sql_engine::dialect::SqlDialect;
use crate::sql_engine::metadata::MetadataIndex;
use crate::sql_engine::models::*;
use crate::sql_engine::tokenizer;

/// Analyzes SQL text and produces a SemanticContext.
///
/// The analyzer never does I/O — it reads metadata from the MetadataIndex
/// which has been pre-populated by the UI layer.
pub struct SemanticAnalyzer<'a> {
    dialect: &'a dyn SqlDialect,
    metadata: &'a MetadataIndex,
}

impl<'a> SemanticAnalyzer<'a> {
    pub fn new(dialect: &'a dyn SqlDialect, metadata: &'a MetadataIndex) -> Self {
        Self { dialect, metadata }
    }

    /// Analyze SQL text at a cursor position within a query block.
    ///
    /// `lines` is the query block (not the entire editor buffer).
    /// `cursor_row` and `cursor_col` are relative to the block.
    pub fn analyze(
        &self,
        lines: &[String],
        cursor_row: usize,
        cursor_col: usize,
    ) -> SemanticContext {
        let full_text = lines.join("\n");

        // Try sqlparser first
        let ast_result = Parser::parse_sql(self.dialect.parser_dialect().as_ref(), &full_text);

        let mut ctx = match ast_result {
            Ok(ref statements) if !statements.is_empty() => {
                self.analyze_from_ast(statements, lines, cursor_row, cursor_col)
            }
            _ => {
                // Fallback: token-based analysis for incomplete/unparseable SQL
                self.analyze_from_tokens(lines, cursor_row, cursor_col)
            }
        };

        // Resolve metadata: fill in schemas for unqualified refs, check existence
        self.resolve_metadata(&mut ctx);

        // Populate available columns from resolved table refs
        self.populate_columns(&mut ctx);

        ctx
    }

    /// Analyze for diagnostics only (no cursor position needed).
    pub fn analyze_for_diagnostics(&self, lines: &[String]) -> SemanticContext {
        let mut ctx = self.analyze(lines, 0, 0);
        ctx.is_partial = false;

        // Check that column qualifiers (e.g. `ord.column`) reference valid aliases
        if !self.metadata.all_schemas().is_empty() {
            self.check_column_qualifiers(lines, &mut ctx);
        }

        ctx
    }

    /// Validate `qualifier.column` patterns — the qualifier must be a known
    /// alias, table name, or schema. Produces UnknownTable errors for invalid ones.
    fn check_column_qualifiers(&self, lines: &[String], ctx: &mut SemanticContext) {
        let line_strs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let tokens = tokenizer::tokenize_sql(&line_strs);

        let mut i = 0;
        while i + 2 < tokens.len() {
            // Pattern: Word . Word
            if tokens[i].kind == tokenizer::TokenKind::Word
                && tokens[i + 1].kind == tokenizer::TokenKind::Dot
                && tokens[i + 2].kind == tokenizer::TokenKind::Word
            {
                let qualifier = &tokens[i].text;
                let norm = self.dialect.normalize_identifier(qualifier);

                // Skip if qualifier is a known alias, table, or schema
                let is_known = ctx.aliases.contains_key(&norm)
                    || self.metadata.is_known_schema(qualifier)
                    || self.metadata.resolve_schema_for(qualifier).is_some();

                if !is_known {
                    // Check it's not a keyword used as a qualifier (e.g. SUM.something)
                    let upper = qualifier.to_uppercase();
                    if !tokenizer::is_sql_keyword(&upper) {
                        ctx.resolution_errors.push(ResolutionError {
                            location: Location {
                                row: tokens[i].row,
                                col_start: tokens[i].col,
                                col_end: tokens[i].col + tokens[i].text.len(),
                            },
                            message: format!("Unknown alias or table '{qualifier}'"),
                            kind: ResolutionErrorKind::UnknownTable,
                        });
                    }
                }
                i += 3;
            } else {
                i += 1;
            }
        }
    }

    // -----------------------------------------------------------------------
    // AST-based analysis (primary path)
    // -----------------------------------------------------------------------

    fn analyze_from_ast(
        &self,
        statements: &[Statement],
        lines: &[String],
        cursor_row: usize,
        cursor_col: usize,
    ) -> SemanticContext {
        let mut ctx = SemanticContext::empty();
        ctx.is_partial = false;

        for stmt in statements {
            self.extract_table_refs_from_statement(stmt, &mut ctx);
        }

        // Determine cursor context using token scanning (AST doesn't track cursor)
        ctx.cursor_context = self.detect_cursor_context(lines, cursor_row, cursor_col);
        ctx.prefix = tokenizer::word_prefix_at(lines, cursor_row, cursor_col)
            .0
            .to_string();
        ctx
    }

    /// Walk sqlparser AST to extract FROM/JOIN table references.
    fn extract_table_refs_from_statement(&self, stmt: &Statement, ctx: &mut SemanticContext) {
        match stmt {
            Statement::Query(query) => {
                self.extract_from_query(query, ctx);
            }
            Statement::Insert { table_name, .. } => {
                self.add_table_from_object_name(table_name, None, ctx);
            }
            Statement::Update { table, .. } => {
                self.extract_from_table_with_joins(table, ctx);
            }
            Statement::Delete { from, .. } => {
                let tables = match from {
                    ast::FromTable::WithFromKeyword(t) => t,
                    ast::FromTable::WithoutKeyword(t) => t,
                };
                for twj in tables {
                    self.extract_from_table_with_joins(twj, ctx);
                }
            }
            _ => {} // DDL, etc. — no table refs to resolve
        }
    }

    fn extract_from_query(&self, query: &ast::Query, ctx: &mut SemanticContext) {
        // Handle CTEs (WITH clauses)
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                let alias = cte.alias.name.value.clone();
                let normalized = self.dialect.normalize_identifier(&alias);
                ctx.aliases.insert(
                    normalized,
                    QualifiedName {
                        schema: None,
                        name: alias,
                    },
                );
            }
        }

        match query.body.as_ref() {
            SetExpr::Select(select) => {
                for twj in &select.from {
                    self.extract_from_table_with_joins(twj, ctx);
                }
            }
            SetExpr::SetOperation { left, right, .. } => {
                self.extract_from_set_expr(left, ctx);
                self.extract_from_set_expr(right, ctx);
            }
            _ => {}
        }
    }

    fn extract_from_set_expr(&self, expr: &SetExpr, ctx: &mut SemanticContext) {
        match expr {
            SetExpr::Select(select) => {
                for twj in &select.from {
                    self.extract_from_table_with_joins(twj, ctx);
                }
            }
            SetExpr::SetOperation { left, right, .. } => {
                self.extract_from_set_expr(left, ctx);
                self.extract_from_set_expr(right, ctx);
            }
            SetExpr::Query(q) => self.extract_from_query(q, ctx),
            _ => {}
        }
    }

    fn extract_from_table_with_joins(&self, twj: &TableWithJoins, ctx: &mut SemanticContext) {
        self.extract_from_table_factor(&twj.relation, ctx);
        for join in &twj.joins {
            self.extract_from_table_factor(&join.relation, ctx);
        }
    }

    fn extract_from_table_factor(&self, factor: &TableFactor, ctx: &mut SemanticContext) {
        match factor {
            TableFactor::Table { name, alias, .. } => {
                let alias_str = alias.as_ref().map(|a| a.name.value.clone());
                self.add_table_from_object_name(name, alias_str, ctx);
            }
            TableFactor::Derived {
                alias, subquery, ..
            } => {
                // Subquery in FROM — register alias, recurse
                if let Some(a) = alias {
                    let norm = self.dialect.normalize_identifier(&a.name.value);
                    ctx.aliases.insert(
                        norm,
                        QualifiedName {
                            schema: None,
                            name: a.name.value.clone(),
                        },
                    );
                }
                self.extract_from_query(subquery, ctx);
            }
            _ => {}
        }
    }

    fn add_table_from_object_name(
        &self,
        name: &ast::ObjectName,
        alias: Option<String>,
        ctx: &mut SemanticContext,
    ) {
        let parts: Vec<&str> = name.0.iter().map(|i| i.value.as_str()).collect();
        let (schema, table_name) = match parts.len() {
            1 => (None, parts[0].to_string()),
            2 => (Some(parts[0].to_string()), parts[1].to_string()),
            3 => (Some(parts[1].to_string()), parts[2].to_string()), // catalog.schema.table
            _ => return,
        };

        let qn = QualifiedName {
            schema: schema.clone(),
            name: table_name.clone(),
        };

        // Register alias
        if let Some(ref a) = alias {
            let norm = self.dialect.normalize_identifier(a);
            ctx.aliases.insert(norm, qn.clone());
        }
        // Also register table name as self-alias
        let norm = self.dialect.normalize_identifier(&table_name);
        ctx.aliases.insert(norm, qn.clone());

        ctx.table_refs.push(ResolvedTableRef {
            reference: TableReference {
                qualified_name: qn,
                alias,
                location: Location {
                    row: 0,
                    col_start: 0,
                    col_end: 0,
                }, // AST doesn't provide spans
                function_call: None,
            },
            resolved_schema: schema,
            exists: None,
        });
    }

    // -----------------------------------------------------------------------
    // Token-based analysis (fallback for incomplete SQL / PL/SQL)
    // -----------------------------------------------------------------------

    fn analyze_from_tokens(
        &self,
        lines: &[String],
        cursor_row: usize,
        cursor_col: usize,
    ) -> SemanticContext {
        let mut ctx = SemanticContext::empty();
        ctx.is_partial = true;

        let line_strs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let tokens = tokenizer::tokenize_sql(&line_strs);
        let raw_refs = tokenizer::extract_table_refs_from_tokens(&tokens);
        ctx.cte_names = tokenizer::extract_cte_names(&tokens)
            .into_iter()
            .map(|n| self.dialect.normalize_identifier(&n))
            .collect();

        for raw in raw_refs {
            // TABLE(pkg.fn()) uses a synthetic unique name so the alias
            // can't collide with a real table called "TABLE" (or with
            // another TABLE() ref in the same query).
            let synthetic_name = raw.function_call.as_ref().map(|fc| {
                format!(
                    "__TABLEFN__{}__{}__{}",
                    fc.schema.as_deref().unwrap_or(""),
                    fc.package.as_deref().unwrap_or(""),
                    fc.function,
                )
            });
            let effective_name = synthetic_name.clone().unwrap_or_else(|| raw.name.clone());

            let qn = QualifiedName {
                schema: if synthetic_name.is_some() {
                    None
                } else {
                    raw.schema.clone()
                },
                name: effective_name.clone(),
            };

            // Register aliases
            if let Some(ref a) = raw.alias {
                let norm = self.dialect.normalize_identifier(a);
                ctx.aliases.insert(norm, qn.clone());
            }
            let norm = self.dialect.normalize_identifier(&effective_name);
            ctx.aliases.insert(norm, qn.clone());

            ctx.table_refs.push(ResolvedTableRef {
                reference: TableReference {
                    qualified_name: qn,
                    alias: raw.alias,
                    location: Location {
                        row: raw.row,
                        col_start: raw.col_start,
                        col_end: raw.col_end,
                    },
                    function_call: raw.function_call,
                },
                resolved_schema: if synthetic_name.is_some() {
                    None
                } else {
                    raw.schema
                },
                exists: None,
            });
        }

        ctx.cursor_context = self.detect_cursor_context(lines, cursor_row, cursor_col);
        ctx.prefix = tokenizer::word_prefix_at(lines, cursor_row, cursor_col)
            .0
            .to_string();
        ctx
    }

    // -----------------------------------------------------------------------
    // Cursor context detection
    // -----------------------------------------------------------------------

    fn detect_cursor_context(&self, lines: &[String], row: usize, col: usize) -> CursorContext {
        if row < lines.len() {
            let before = &lines[row][..col.min(lines[row].len())];

            // Two-level dot chain: `schema.package.<cursor>`. We trust the
            // syntax even if metadata doesn't know about either qualifier
            // yet — the completion engine will surface what it can find,
            // and an unloaded package won't poison anything.
            if let Some((q1, q2)) = tokenizer::two_identifiers_before_dot(before) {
                return CursorContext::PackageDot {
                    schema: Some(q1.to_string()),
                    package: q2.to_string(),
                };
            }

            // Single-level dot: `schema.` / `package.` / `table.` / `alias.`
            if let Some((identifier, _)) = tokenizer::identifier_before_dot(before) {
                if self.metadata.is_known_schema(identifier) {
                    let in_table_ref = matches!(
                        tokenizer::find_keyword_context(lines, row, col),
                        CursorContext::TableRef | CursorContext::AfterTableRef
                    );
                    return CursorContext::SchemaDot {
                        schema_name: identifier.to_string(),
                        in_table_ref,
                    };
                }
                if self.metadata.has_package(None, identifier) {
                    return CursorContext::PackageDot {
                        schema: None,
                        package: identifier.to_string(),
                    };
                }
                return CursorContext::ColumnDot {
                    table_ref: identifier.to_string(),
                };
            }
        }

        // Fall back to keyword-based backward scan
        tokenizer::find_keyword_context(lines, row, col)
    }

    // -----------------------------------------------------------------------
    // Metadata resolution
    // -----------------------------------------------------------------------

    /// Resolve unqualified table references and check existence.
    fn resolve_metadata(&self, ctx: &mut SemanticContext) {
        let current_schema = self.metadata.current_schema().map(|s| s.to_string());

        let cte_set: Vec<String> = ctx.cte_names.clone();

        for tref in &mut ctx.table_refs {
            if tref.resolved_schema.is_none() {
                // Unqualified: try current schema first, then any schema
                let norm_name = self
                    .dialect
                    .normalize_identifier(&tref.reference.qualified_name.name);

                // Skip CTE references — they're virtual tables defined in WITH
                if cte_set.contains(&norm_name) {
                    tref.exists = Some(true);
                    continue;
                }

                if let Some(ref cs) = current_schema {
                    let objects = self.metadata.tables_and_views(cs);
                    let found = objects
                        .iter()
                        .any(|(n, _)| self.dialect.normalize_identifier(n) == norm_name);
                    if found {
                        tref.resolved_schema = Some(cs.clone());
                        tref.exists = Some(true);
                        continue;
                    }
                }

                // Try resolving via any schema
                if let Some(schema) = self
                    .metadata
                    .resolve_schema_for(&tref.reference.qualified_name.name)
                {
                    tref.resolved_schema = Some(schema.to_string());
                    tref.exists = Some(true);
                } else {
                    tref.exists = Some(false);
                }
            } else {
                // Qualified: check if schema exists, then if object exists
                let schema = tref.resolved_schema.as_ref().unwrap();
                if !self.metadata.is_known_schema(schema) {
                    tref.exists = Some(false);
                    ctx.resolution_errors.push(ResolutionError {
                        location: tref.reference.location,
                        message: format!("Unknown schema '{schema}'"),
                        kind: ResolutionErrorKind::UnknownSchema,
                    });
                    continue;
                }

                // If schema is known but its objects haven't been loaded yet,
                // don't mark the table as non-existent — we simply don't know.
                if !self.metadata.has_objects_loaded(schema) {
                    continue;
                }

                let norm_name = self
                    .dialect
                    .normalize_identifier(&tref.reference.qualified_name.name);
                let objects = self.metadata.tables_and_views(schema);
                let found = objects
                    .iter()
                    .any(|(n, _)| self.dialect.normalize_identifier(n) == norm_name);
                tref.exists = Some(found);
            }

            // Add resolution error for non-existent tables
            if tref.exists == Some(false) {
                ctx.resolution_errors.push(ResolutionError {
                    location: tref.reference.location,
                    message: format!("Unknown table/view '{}'", tref.reference.qualified_name),
                    kind: ResolutionErrorKind::UnknownTable,
                });
            }
        }
    }

    /// Populate available_columns from MetadataIndex for all resolved table refs.
    fn populate_columns(&self, ctx: &mut SemanticContext) {
        for tref in &ctx.table_refs {
            // TABLE(pkg.fn()) refs — pull pseudo-columns from the function's
            // cached return type. The synthetic name on the ref is what the
            // alias resolves to via ctx.aliases.
            if let Some(ref fc) = tref.reference.function_call {
                let synthetic = tref.reference.qualified_name.name.clone();
                if let Some(cols) = self.metadata.get_function_return_columns(
                    fc.schema.as_deref(),
                    fc.package.as_deref(),
                    &fc.function,
                ) {
                    for col in cols {
                        let mut c = col.clone();
                        c.table_name = synthetic.clone();
                        ctx.available_columns.push(c);
                    }
                }
                continue;
            }
            if let Some(ref schema) = tref.resolved_schema
                && let Some(cols) = self
                    .metadata
                    .get_columns(schema, &tref.reference.qualified_name.name)
            {
                for col in cols {
                    ctx.available_columns.push(col.clone());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MetadataIndex helper: tables_and_views (used by resolve_metadata)
// ---------------------------------------------------------------------------

impl MetadataIndex {
    /// Table and view names in a schema. Returns (name, is_view).
    pub fn tables_and_views(&self, schema: &str) -> Vec<(String, bool)> {
        use crate::sql_engine::metadata::ObjectKind;
        self.objects_by_kind(
            Some(schema),
            &[
                ObjectKind::Table,
                ObjectKind::View,
                ObjectKind::MaterializedView,
            ],
        )
        .iter()
        .map(|e| {
            (
                e.display_name.clone(),
                matches!(e.kind, ObjectKind::View | ObjectKind::MaterializedView),
            )
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql_engine::dialect::OracleDialect;
    use crate::sql_engine::metadata::{MetadataIndex, ObjectKind};

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
        idx
    }

    #[test]
    fn select_from_resolves_table() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM employees".into()];
        let ctx = analyzer.analyze(&lines, 0, 23);

        assert_eq!(ctx.table_refs.len(), 1);
        assert_eq!(ctx.table_refs[0].exists, Some(true));
        assert_eq!(ctx.table_refs[0].resolved_schema.as_deref(), Some("HR"));
        assert_eq!(ctx.available_columns.len(), 3);
        assert!(ctx.resolution_errors.is_empty());
    }

    #[test]
    fn alias_resolution() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT e.first_name FROM employees e".into()];
        let ctx = analyzer.analyze(&lines, 0, 36);

        // "e" should be registered as alias for employees
        assert!(ctx.aliases.contains_key("E"));
        let resolved = ctx.aliases.get("E").unwrap();
        assert_eq!(resolved.name.to_uppercase(), "EMPLOYEES");
    }

    #[test]
    fn dot_context_schema() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM hr.".into()];
        let ctx = analyzer.analyze(&lines, 0, 17);

        matches!(ctx.cursor_context, CursorContext::SchemaDot { .. });
    }

    #[test]
    fn dot_context_column() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT e.".into(), "FROM employees e".into()];
        let ctx = analyzer.analyze(&lines, 0, 9);

        matches!(ctx.cursor_context, CursorContext::ColumnDot { .. });
    }

    #[test]
    fn unknown_table_produces_error() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM nonexistent".into()];
        let ctx = analyzer.analyze_for_diagnostics(&lines);

        assert!(!ctx.resolution_errors.is_empty());
        assert_eq!(
            ctx.resolution_errors[0].kind,
            ResolutionErrorKind::UnknownTable
        );
    }

    #[test]
    fn unknown_schema_produces_error() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM unknown_schema.employees".into()];
        let ctx = analyzer.analyze_for_diagnostics(&lines);

        assert!(!ctx.resolution_errors.is_empty());
        assert_eq!(
            ctx.resolution_errors[0].kind,
            ResolutionErrorKind::UnknownSchema
        );
    }

    #[test]
    fn incomplete_sql_uses_token_fallback() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);

        // This is incomplete SQL — sqlparser will fail, token fallback kicks in
        let lines: Vec<String> = vec!["SELECT * FROM employees e WHERE e.".into()];
        let ctx = analyzer.analyze(&lines, 0, 34);

        assert!(ctx.is_partial);
        // Should still find "employees" reference
        assert!(!ctx.table_refs.is_empty());
        assert!(ctx.aliases.contains_key("E"));
    }

    #[test]
    fn qualified_table_ref() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM hr.employees".into()];
        let ctx = analyzer.analyze(&lines, 0, 26);

        assert_eq!(ctx.table_refs.len(), 1);
        assert_eq!(ctx.table_refs[0].exists, Some(true));
        assert_eq!(
            ctx.table_refs[0].reference.qualified_name.schema.as_deref(),
            Some("hr")
        );
    }

    #[test]
    fn select_context_detected() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT emp".into()];
        let ctx = analyzer.analyze(&lines, 0, 10);

        assert!(matches!(ctx.cursor_context, CursorContext::SelectList));
        assert_eq!(ctx.prefix, "emp");
    }

    #[test]
    fn where_context_detected() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM employees".into(), "WHERE dep".into()];
        let ctx = analyzer.analyze(&lines, 1, 9);

        assert!(matches!(ctx.cursor_context, CursorContext::Predicate));
        assert_eq!(ctx.prefix, "dep");
    }

    #[test]
    fn columns_for_alias() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);

        let lines: Vec<String> = vec!["SELECT * FROM employees e".into()];
        let ctx = analyzer.analyze(&lines, 0, 25);

        let cols = ctx.columns_for("e", &|s| s.to_uppercase());
        assert_eq!(cols.len(), 3);
    }

    #[test]
    fn multiple_tables_with_join() {
        let idx = test_index();
        let dialect = OracleDialect;
        let analyzer = SemanticAnalyzer::new(&dialect, &idx);

        let lines: Vec<String> = vec![
            "SELECT * FROM employees e JOIN departments d ON e.department_id = d.department_id"
                .into(),
        ];
        let ctx = analyzer.analyze(&lines, 0, 80);

        assert_eq!(ctx.table_refs.len(), 2);
        assert!(ctx.aliases.contains_key("E"));
        assert!(ctx.aliases.contains_key("D"));
    }
}
