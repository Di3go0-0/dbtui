/// SQL diagnostics engine (LCP).
/// Parses SQL text to extract table/schema references and validates them
/// against the database metadata loaded in the sidebar tree.
/// Only checks against filtered schemas/tables to keep it efficient.
use crate::ui::sql_tokens::{TokenKind, is_sql_keyword, tokenize_sql};
use crate::ui::state::{AppState, LeafKind, TreeNode};

/// A single diagnostic (error/warning on a specific range).
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub row: usize,
    pub col_start: usize,
    pub col_end: usize,
    pub message: String,
}

/// Run diagnostics on the active editor's content.
/// Returns a list of diagnostics for invalid table/view references.
pub fn check_sql(state: &AppState, lines: &[String]) -> Vec<Diagnostic> {
    let conn_name = match &state.connection_name {
        Some(n) => n,
        None => return vec![],
    };

    // Collect known object names (filtered) for fast lookup
    let known_objects = collect_known_objects(state, conn_name);
    let known_schemas = collect_known_schemas(state, conn_name);

    let mut diagnostics = Vec::new();
    let full_text: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();

    // Extract and validate table references
    let refs = extract_table_refs(&full_text);
    for tref in refs {
        if let Some(schema) = &tref.schema {
            // schema.table reference: check both
            let schema_upper = schema.to_uppercase();
            let schema_lower = schema.to_lowercase();
            if !known_schemas
                .iter()
                .any(|s| s.to_uppercase() == schema_upper || s.to_lowercase() == schema_lower)
            {
                diagnostics.push(Diagnostic {
                    row: tref.row,
                    col_start: tref.col_start,
                    col_end: tref.col_start + schema.len(),
                    message: format!("Unknown schema '{schema}'"),
                });
                continue;
            }
            // Check table within schema
            let table_upper = tref.name.to_uppercase();
            let table_lower = tref.name.to_lowercase();
            if !known_objects.iter().any(|(s, n)| {
                (s.to_uppercase() == schema_upper || s.to_lowercase() == schema_lower)
                    && (n.to_uppercase() == table_upper || n.to_lowercase() == table_lower)
            }) {
                let dot_len = schema.len() + 1; // "schema."
                diagnostics.push(Diagnostic {
                    row: tref.row,
                    col_start: tref.col_start + dot_len,
                    col_end: tref.col_end,
                    message: format!("Unknown table/view '{}.{}'", schema, tref.name),
                });
            }
        } else {
            // Unqualified table reference: check against all known objects
            let name_upper = tref.name.to_uppercase();
            let name_lower = tref.name.to_lowercase();
            if !known_objects
                .iter()
                .any(|(_, n)| n.to_uppercase() == name_upper || n.to_lowercase() == name_lower)
            {
                diagnostics.push(Diagnostic {
                    row: tref.row,
                    col_start: tref.col_start,
                    col_end: tref.col_end,
                    message: format!("Unknown table/view '{}'", tref.name),
                });
            }
        }
    }

    diagnostics
}

/// Collect known table/view names from the tree (respecting filters).
/// Returns Vec<(schema_name, object_name)>.
fn collect_known_objects(state: &AppState, conn_name: &str) -> Vec<(String, String)> {
    let mut objects = Vec::new();
    for node in &state.tree {
        if let TreeNode::Leaf {
            name, schema, kind, ..
        } = node
        {
            // Only tables and views are valid in FROM/JOIN
            if !matches!(kind, LeafKind::Table | LeafKind::View) {
                continue;
            }
            let cat_suffix = match kind {
                LeafKind::Table => "Tables",
                LeafKind::View => "Views",
                _ => continue,
            };
            let cat_key = format!("{conn_name}::{schema}.{cat_suffix}");
            if state.object_filter.is_enabled(&cat_key, name) {
                objects.push((schema.clone(), name.clone()));
            }
        }
    }
    objects
}

/// Collect known schema names (respecting filters).
fn collect_known_schemas(state: &AppState, conn_name: &str) -> Vec<String> {
    let key = format!("{conn_name}::schemas");
    let mut schemas = Vec::new();
    for node in &state.tree {
        if let TreeNode::Schema { name, .. } = node
            && state.object_filter.is_enabled(&key, name)
        {
            schemas.push(name.clone());
        }
    }
    schemas
}

/// A parsed table reference found in SQL.
#[derive(Debug)]
struct TableRef {
    schema: Option<String>,
    name: String,
    row: usize,
    col_start: usize,
    col_end: usize,
}

/// SQL keywords that precede table/view names.
const TABLE_CONTEXT_KEYWORDS: &[&str] = &[
    "FROM", "JOIN", "INTO", "UPDATE", "TABLE", "VIEW", "INNER", "LEFT", "RIGHT", "FULL", "CROSS",
    "NATURAL",
];

/// Extract table references from SQL lines.
/// Looks for identifiers after FROM, JOIN, INTO, UPDATE, TABLE keywords.
fn extract_table_refs(lines: &[&str]) -> Vec<TableRef> {
    let mut refs = Vec::new();

    // Tokenize all lines, tracking positions
    let tokens = tokenize_sql(lines);

    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];

        // Check if this token is a table-context keyword
        if token.kind == TokenKind::Word {
            let upper = token.text.to_uppercase();

            // Skip "JOIN" variants: "LEFT JOIN", "INNER JOIN" etc.
            // The actual table comes after JOIN
            if TABLE_CONTEXT_KEYWORDS.contains(&upper.as_str()) {
                // For compound joins (LEFT JOIN), skip to JOIN then take next
                let next_idx = if matches!(
                    upper.as_str(),
                    "INNER" | "LEFT" | "RIGHT" | "FULL" | "CROSS" | "NATURAL"
                ) {
                    // Skip optional OUTER, then JOIN
                    let mut j = i + 1;
                    while j < tokens.len() && tokens[j].kind == TokenKind::Whitespace {
                        j += 1;
                    }
                    if j < tokens.len()
                        && tokens[j].kind == TokenKind::Word
                        && tokens[j].text.to_uppercase() == "OUTER"
                    {
                        j += 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Whitespace {
                            j += 1;
                        }
                    }
                    if j < tokens.len()
                        && tokens[j].kind == TokenKind::Word
                        && tokens[j].text.to_uppercase() == "JOIN"
                    {
                        j + 1
                    } else {
                        i + 1
                    }
                } else {
                    i + 1
                };

                // Skip whitespace after keyword
                let mut j = next_idx;
                while j < tokens.len() && tokens[j].kind == TokenKind::Whitespace {
                    j += 1;
                }

                // Comma-separated table list (FROM t1, t2, ...)
                while j < tokens.len() {
                    if tokens[j].kind != TokenKind::Word {
                        break;
                    }

                    // Check for schema.table pattern
                    let first = &tokens[j];
                    let mut k = j + 1;

                    if k < tokens.len()
                        && tokens[k].kind == TokenKind::Dot
                        && k + 1 < tokens.len()
                        && tokens[k + 1].kind == TokenKind::Word
                    {
                        // schema.table
                        let second = &tokens[k + 1];
                        refs.push(TableRef {
                            schema: Some(first.text.to_string()),
                            name: second.text.to_string(),
                            row: first.row,
                            col_start: first.col,
                            col_end: second.col + second.text.len(),
                        });
                        k += 2;
                    } else {
                        // Just table name (skip SQL keywords that follow)
                        let upper_name = first.text.to_uppercase();
                        if !is_sql_keyword(&upper_name) {
                            refs.push(TableRef {
                                schema: None,
                                name: first.text.to_string(),
                                row: first.row,
                                col_start: first.col,
                                col_end: first.col + first.text.len(),
                            });
                        }
                        k = j + 1;
                    }

                    // Skip optional alias
                    let mut m = k;
                    while m < tokens.len() && tokens[m].kind == TokenKind::Whitespace {
                        m += 1;
                    }
                    if m < tokens.len()
                        && tokens[m].kind == TokenKind::Word
                        && !is_sql_keyword(&tokens[m].text.to_uppercase())
                    {
                        m += 1;
                    }

                    // Check for comma (more tables)
                    while m < tokens.len() && tokens[m].kind == TokenKind::Whitespace {
                        m += 1;
                    }
                    if m < tokens.len() && tokens[m].kind == TokenKind::Comma {
                        m += 1;
                        while m < tokens.len() && tokens[m].kind == TokenKind::Whitespace {
                            m += 1;
                        }
                        j = m;
                    } else {
                        break;
                    }
                }

                i = j;
                continue;
            }
        }

        i += 1;
    }

    refs
}
