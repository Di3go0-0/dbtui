/// SQL diagnostics engine (LCP).
/// Parses SQL text to extract table/schema references and validates them
/// against the database metadata loaded in the sidebar tree.
/// Also runs sqlparser-based syntax validation per query block.
use crate::core::models::DatabaseType;
use crate::ui::sql_tokens::{TokenKind, is_sql_keyword, tokenize_sql};
use crate::ui::state::{AppState, LeafKind, TreeNode};

use sqlparser::dialect::{GenericDialect, MySqlDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;

/// A single diagnostic (error/warning on a specific range).
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub row: usize,
    pub col_start: usize,
    pub col_end: usize,
    pub message: String,
}

/// Run diagnostics on the given lines scoped to one query block.
/// `script_conn` overrides the global connection when set (for script tabs).
pub fn check_sql(
    state: &AppState,
    lines: &[String],
    script_conn: Option<&str>,
) -> Vec<Diagnostic> {
    let conn_name = match script_conn.or(state.connection_name.as_deref()) {
        Some(n) => n,
        None => return vec![],
    };

    let mut diagnostics = Vec::new();

    // Syntax validation via sqlparser (per query block)
    check_syntax(state, lines, &mut diagnostics);

    let known_objects = collect_known_objects(state, conn_name);
    let known_schemas = collect_known_schemas(state, conn_name);
    let full_text: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();

    // Extract table refs AND aliases from the query block
    let (refs, aliases) = extract_table_refs(&full_text);

    for tref in refs {
        if let Some(schema) = &tref.schema {
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
            let table_upper = tref.name.to_uppercase();
            let table_lower = tref.name.to_lowercase();
            if !known_objects.iter().any(|(s, n)| {
                (s.to_uppercase() == schema_upper || s.to_lowercase() == schema_lower)
                    && (n.to_uppercase() == table_upper || n.to_lowercase() == table_lower)
            }) {
                let dot_len = schema.len() + 1;
                diagnostics.push(Diagnostic {
                    row: tref.row,
                    col_start: tref.col_start + dot_len,
                    col_end: tref.col_end,
                    message: format!("Unknown table/view '{}.{}'", schema, tref.name),
                });
            }
        } else {
            // Skip if it's a known alias in this query block
            let name_upper = tref.name.to_uppercase();
            if aliases.iter().any(|a| a.to_uppercase() == name_upper) {
                continue;
            }
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

/// Run sqlparser syntax validation on each query block (separated by blank lines).
/// Errors are mapped back to the original line positions.
fn check_syntax(state: &AppState, lines: &[String], diagnostics: &mut Vec<Diagnostic>) {
    let dialect: Box<dyn sqlparser::dialect::Dialect> = match state.db_type {
        Some(DatabaseType::PostgreSQL) => Box::new(PostgreSqlDialect {}),
        Some(DatabaseType::MySQL) => Box::new(MySqlDialect {}),
        _ => Box::new(GenericDialect {}),
    };

    // Split into query blocks separated by blank lines
    let mut block_start = 0;
    let mut i = 0;
    while i <= lines.len() {
        let is_blank = i == lines.len() || lines[i].trim().is_empty();
        if is_blank && i > block_start {
            let block: String = lines[block_start..i]
                .iter()
                .map(|l| l.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if !block.trim().is_empty()
                && let Err(e) = Parser::parse_sql(dialect.as_ref(), &block)
            {
                    let msg = e.to_string();
                    let (err_line, err_col) = parse_syntax_error_position(&msg);
                    // Map back to original file coordinates
                    let file_row = block_start + err_line.saturating_sub(1);
                    let file_col = if err_col > 0 { err_col - 1 } else { 0 };
                    // Clean up the message (remove position suffix)
                    let clean_msg = msg
                        .split(" at Line:")
                        .next()
                        .unwrap_or(&msg)
                        .to_string();
                    let col_end = if file_row < lines.len() {
                        // Underline to end of line or a reasonable span
                        let line_len = lines[file_row].len();
                        if file_col < line_len {
                            line_len
                        } else {
                            file_col + 1
                        }
                    } else {
                        file_col + 1
                    };
                    diagnostics.push(Diagnostic {
                        row: file_row,
                        col_start: file_col,
                        col_end,
                        message: clean_msg,
                    });
            }
            block_start = i + 1;
        } else if is_blank {
            block_start = i + 1;
        }
        i += 1;
    }
}

/// Parse line/column from sqlparser error messages.
/// Format: "Expected ..., found: ... at Line: 5, Column: 10"
fn parse_syntax_error_position(msg: &str) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    if let Some(pos) = msg.find("Line: ")
        && let Some(num_str) = msg[pos + 6..].split(',').next()
    {
        line = num_str.trim().parse().unwrap_or(1);
    }
    if let Some(pos) = msg.find("Column: ")
        && let Some(num_str) = msg[pos + 8..].split(|c: char| !c.is_ascii_digit()).next()
    {
        col = num_str.trim().parse().unwrap_or(1);
    }
    (line, col)
}

/// Collect known table/view names from the tree under a specific connection.
/// Walks the tree positionally: only collects leaves below the matching Connection node.
fn collect_known_objects(state: &AppState, conn_name: &str) -> Vec<(String, String)> {
    let mut objects = Vec::new();
    let mut in_target_conn = false;

    for node in &state.tree {
        match node {
            TreeNode::Connection { name, .. } => {
                in_target_conn = name == conn_name;
            }
            TreeNode::Group { .. } => {
                // Groups are above connections; reset
                in_target_conn = false;
            }
            TreeNode::Leaf {
                name, schema, kind, ..
            } if in_target_conn => {
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
            _ => {}
        }
    }
    objects
}

/// Collect known schema names under a specific connection.
fn collect_known_schemas(state: &AppState, conn_name: &str) -> Vec<String> {
    let key = format!("{conn_name}::schemas");
    let mut schemas = Vec::new();
    let mut in_target_conn = false;

    for node in &state.tree {
        match node {
            TreeNode::Connection { name, .. } => {
                in_target_conn = name == conn_name;
            }
            TreeNode::Group { .. } => {
                in_target_conn = false;
            }
            TreeNode::Schema { name, .. } if in_target_conn => {
                if state.object_filter.is_enabled(&key, name) {
                    schemas.push(name.clone());
                }
            }
            _ => {}
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

/// Extract table references and aliases from SQL lines.
/// Returns (table_refs, aliases) where aliases are names defined via AS or implicit aliasing.
fn extract_table_refs(lines: &[&str]) -> (Vec<TableRef>, Vec<String>) {
    let mut refs = Vec::new();
    let mut aliases = Vec::new();

    let tokens = tokenize_sql(lines);

    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];

        if token.kind == TokenKind::Word {
            let upper = token.text.to_uppercase();

            if TABLE_CONTEXT_KEYWORDS.contains(&upper.as_str()) {
                let next_idx = if matches!(
                    upper.as_str(),
                    "INNER" | "LEFT" | "RIGHT" | "FULL" | "CROSS" | "NATURAL"
                ) {
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

                let mut j = next_idx;
                while j < tokens.len() && tokens[j].kind == TokenKind::Whitespace {
                    j += 1;
                }

                // Comma-separated table list
                while j < tokens.len() {
                    if tokens[j].kind != TokenKind::Word {
                        break;
                    }

                    let first = &tokens[j];
                    let mut k = j + 1;

                    if k < tokens.len()
                        && tokens[k].kind == TokenKind::Dot
                        && k + 1 < tokens.len()
                        && tokens[k + 1].kind == TokenKind::Word
                    {
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

                    // Capture optional alias (AS alias | implicit alias)
                    let mut m = k;
                    while m < tokens.len() && tokens[m].kind == TokenKind::Whitespace {
                        m += 1;
                    }
                    if m < tokens.len() && tokens[m].kind == TokenKind::Word {
                        let alias_upper = tokens[m].text.to_uppercase();
                        if alias_upper == "AS" {
                            // Skip AS, take next word as alias
                            m += 1;
                            while m < tokens.len() && tokens[m].kind == TokenKind::Whitespace {
                                m += 1;
                            }
                            if m < tokens.len()
                                && tokens[m].kind == TokenKind::Word
                                && !is_sql_keyword(&tokens[m].text.to_uppercase())
                            {
                                aliases.push(tokens[m].text.to_string());
                                m += 1;
                            }
                        } else if !is_sql_keyword(&alias_upper) {
                            // Implicit alias (not a keyword)
                            aliases.push(tokens[m].text.to_string());
                            m += 1;
                        }
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

    (refs, aliases)
}
