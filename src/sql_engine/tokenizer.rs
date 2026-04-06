//! Shared SQL tokenizer for completion and diagnostics engines.
//! Provides position-preserving tokenization of SQL text.
//!
//! Migrated from src/ui/sql_tokens.rs — that file now re-exports from here.

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TokenKind {
    Word,
    Whitespace,
    Dot,
    Comma,
    Other,
}

#[derive(Debug)]
pub struct Token<'a> {
    pub text: &'a str,
    pub kind: TokenKind,
    pub row: usize,
    pub col: usize,
}

/// Tokenize SQL lines, skipping line comments (`--`).
pub fn tokenize_sql<'a>(lines: &[&'a str]) -> Vec<Token<'a>> {
    let mut tokens = Vec::new();

    for (row, line) in lines.iter().enumerate() {
        // Skip line comments
        if let Some(pos) = line.find("--") {
            let effective = &line[..pos];
            tokenize_line(effective, row, &mut tokens);
            continue;
        }

        tokenize_line(line, row, &mut tokens);
    }

    tokens
}

fn tokenize_line<'a>(line: &'a str, row: usize, tokens: &mut Vec<Token<'a>>) {
    let bytes = line.as_bytes();
    let mut col = 0;

    while col < bytes.len() {
        let b = bytes[col];

        if b.is_ascii_whitespace() {
            let start = col;
            while col < bytes.len() && bytes[col].is_ascii_whitespace() {
                col += 1;
            }
            tokens.push(Token {
                text: &line[start..col],
                kind: TokenKind::Whitespace,
                row,
                col: start,
            });
        } else if b == b'.' {
            tokens.push(Token {
                text: ".",
                kind: TokenKind::Dot,
                row,
                col,
            });
            col += 1;
        } else if b == b',' {
            tokens.push(Token {
                text: ",",
                kind: TokenKind::Comma,
                row,
                col,
            });
            col += 1;
        } else if b == b'\'' {
            // Skip string literals
            let start = col;
            col += 1;
            while col < bytes.len() && bytes[col] != b'\'' {
                col += 1;
            }
            if col < bytes.len() {
                col += 1;
            }
            tokens.push(Token {
                text: &line[start..col],
                kind: TokenKind::Other,
                row,
                col: start,
            });
        } else if b.is_ascii_alphanumeric() || b == b'_' {
            let start = col;
            while col < bytes.len() && (bytes[col].is_ascii_alphanumeric() || bytes[col] == b'_') {
                col += 1;
            }
            tokens.push(Token {
                text: &line[start..col],
                kind: TokenKind::Word,
                row,
                col: start,
            });
        } else {
            tokens.push(Token {
                text: &line[col..col + 1],
                kind: TokenKind::Other,
                row,
                col,
            });
            col += 1;
        }
    }

    // Newline acts as whitespace between lines
    tokens.push(Token {
        text: " ",
        kind: TokenKind::Whitespace,
        row,
        col: line.len(),
    });
}

// ---------------------------------------------------------------------------
// Helpers for the SemanticAnalyzer
// ---------------------------------------------------------------------------

/// Extract the word prefix at cursor position. Returns (prefix, start_col).
pub fn word_prefix_at(lines: &[String], row: usize, col: usize) -> (&str, usize) {
    if row >= lines.len() {
        return ("", col);
    }
    let line = &lines[row];
    let bytes = line.as_bytes();
    let end = col.min(bytes.len());
    let mut start = end;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    (&line[start..end], start)
}

/// Extract the identifier immediately before a dot.
/// Given `"SELECT schema."`, returns `Some(("schema", 7))`.
/// Returns None if cursor is not in a `identifier.prefix` pattern.
pub fn identifier_before_dot(before_cursor: &str) -> Option<(&str, usize)> {
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

    Some((&before_cursor[id_start..id_end], id_start))
}

/// SQL keywords that precede table/view names.
const TABLE_CONTEXT_KEYWORDS: &[&str] = &[
    "FROM", "JOIN", "INTO", "UPDATE", "TABLE", "VIEW", "INNER", "LEFT", "RIGHT", "FULL", "CROSS",
    "NATURAL",
];

/// A raw table reference extracted from tokens.
pub struct RawTableRef {
    pub schema: Option<String>,
    pub name: String,
    pub alias: Option<String>,
    pub row: usize,
    pub col_start: usize,
    pub col_end: usize,
}

/// Extract table references from tokens (used as fallback when sqlparser fails).
pub fn extract_table_refs_from_tokens(tokens: &[Token<'_>]) -> Vec<RawTableRef> {
    let mut refs = Vec::new();

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

                    let (schema, name, row, col_start, col_end) = if k < tokens.len()
                        && tokens[k].kind == TokenKind::Dot
                        && k + 1 < tokens.len()
                        && tokens[k + 1].kind == TokenKind::Word
                    {
                        let second = &tokens[k + 1];
                        k += 2;
                        (
                            Some(first.text.to_string()),
                            second.text.to_string(),
                            first.row,
                            first.col,
                            second.col + second.text.len(),
                        )
                    } else {
                        let upper_name = first.text.to_uppercase();
                        if is_sql_keyword(&upper_name) {
                            break;
                        }
                        k = j + 1;
                        (
                            None,
                            first.text.to_string(),
                            first.row,
                            first.col,
                            first.col + first.text.len(),
                        )
                    };

                    // Capture optional alias
                    let mut m = k;
                    while m < tokens.len() && tokens[m].kind == TokenKind::Whitespace {
                        m += 1;
                    }
                    let alias = if m < tokens.len() && tokens[m].kind == TokenKind::Word {
                        let alias_upper = tokens[m].text.to_uppercase();
                        if alias_upper == "AS" {
                            m += 1;
                            while m < tokens.len() && tokens[m].kind == TokenKind::Whitespace {
                                m += 1;
                            }
                            if m < tokens.len()
                                && tokens[m].kind == TokenKind::Word
                                && !is_sql_keyword(&tokens[m].text.to_uppercase())
                            {
                                let a = tokens[m].text.to_string();
                                m += 1;
                                Some(a)
                            } else {
                                None
                            }
                        } else if !is_sql_keyword(&alias_upper) {
                            let a = tokens[m].text.to_string();
                            m += 1;
                            Some(a)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    refs.push(RawTableRef {
                        schema,
                        name,
                        alias,
                        row,
                        col_start,
                        col_end,
                    });

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

/// Backward keyword scanning to determine cursor context.
/// Migrated from ui/completion.rs find_keyword_context.
pub fn find_keyword_context(
    lines: &[String],
    row: usize,
    col: usize,
) -> crate::sql_engine::context::CursorContext {
    use crate::sql_engine::context::CursorContext;
    use crate::sql_engine::models::QualifiedName;

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

    let mut idents_before_keyword = 0;

    for (i, word) in words.iter().enumerate() {
        let upper = word.to_uppercase();
        if i == 0 {
            continue;
        }
        if !is_sql_keyword(&upper) {
            idents_before_keyword += 1;
            continue;
        }

        match upper.as_str() {
            "SELECT" => return CursorContext::SelectList,
            "FROM" | "JOIN" => {
                if idents_before_keyword == 0 {
                    return CursorContext::TableRef;
                }
                return CursorContext::AfterTableRef;
            }
            "INNER" | "LEFT" | "RIGHT" | "FULL" | "CROSS" | "NATURAL" => {
                if idents_before_keyword == 0 {
                    return CursorContext::TableRef;
                }
                return CursorContext::AfterTableRef;
            }
            "WHERE" | "AND" | "OR" | "ON" | "HAVING" => return CursorContext::Predicate,
            "INTO" => {
                if words
                    .get(i + 1)
                    .is_some_and(|w| w.to_uppercase() == "INSERT")
                {
                    if idents_before_keyword == 0 {
                        return CursorContext::TableTarget;
                    }
                    return CursorContext::General;
                }
                if idents_before_keyword == 0 {
                    return CursorContext::TableRef;
                }
                return CursorContext::SelectList;
            }
            "UPDATE" => {
                if idents_before_keyword == 0 {
                    return CursorContext::TableTarget;
                }
                return CursorContext::General;
            }
            "SET" => {
                if let Some(table) = find_update_table(&words, i) {
                    return CursorContext::SetClause {
                        target_table: QualifiedName {
                            schema: None,
                            name: table,
                        },
                    };
                }
                return CursorContext::Predicate;
            }
            "BY" => {
                if words.get(i + 1).is_some_and(|w| {
                    let u = w.to_uppercase();
                    u == "ORDER" || u == "GROUP"
                }) {
                    return CursorContext::OrderGroupBy;
                }
            }
            "ORDER" | "GROUP" => return CursorContext::OrderGroupBy,
            "EXEC" | "EXECUTE" | "CALL" => {
                if idents_before_keyword == 0 {
                    return CursorContext::ExecCall;
                }
                return CursorContext::General;
            }
            "CREATE" | "ALTER" | "DROP" => return CursorContext::DdlObject,
            // PL/SQL block keywords: these mark a context boundary.
            // Return General so the scanner stops here instead of
            // looking further back and hitting a stale SELECT/FROM.
            "IF" | "ELSIF" | "THEN" | "ELSE" | "LOOP" | "WHILE" | "FOR" | "BEGIN" | "DECLARE"
            | "EXCEPTION" | "RETURN" | "END" | "CURSOR" | "OPEN" | "CLOSE" | "FETCH" | "EXIT"
            | "CONTINUE" | "RAISE" | "PIPE" | "PRAGMA" | "FUNCTION" | "PROCEDURE" | "PACKAGE"
            | "BODY" | "TYPE" | "RECORD" | "CONSTANT" | "SUBTYPE" | "REPLACE" | "TRIGGER" => {
                return CursorContext::General;
            }
            _ => {}
        }
    }

    CursorContext::General
}

/// Extract words from a line in reverse order (for backward keyword scanning).
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

/// In a reverse word list, find the table name after UPDATE.
fn find_update_table(words: &[String], set_idx: usize) -> Option<String> {
    for j in (set_idx + 1)..words.len() {
        let upper = words[j].to_uppercase();
        if upper == "UPDATE" {
            if j > set_idx + 1 {
                return Some(words[set_idx + 1].clone());
            }
            return None;
        }
        if is_sql_keyword(&upper) {
            break;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Keyword detection
// ---------------------------------------------------------------------------

/// Check if a word (already UPPERCASE) is a common SQL/PL/SQL keyword.
/// Used by the context detector to distinguish identifiers from keywords
/// when scanning backwards. Adding a word here means it will NOT be treated
/// as a table name or alias during context detection.
pub fn is_sql_keyword(upper: &str) -> bool {
    matches!(
        upper,
        // DML
        "SELECT"
            | "FROM"
            | "WHERE"
            | "INSERT"
            | "INTO"
            | "VALUES"
            | "UPDATE"
            | "SET"
            | "DELETE"
            | "MERGE"
            | "USING"
            | "TRUNCATE"
            // Joins
            | "JOIN"
            | "INNER"
            | "LEFT"
            | "RIGHT"
            | "FULL"
            | "OUTER"
            | "CROSS"
            | "NATURAL"
            | "ON"
            // Logical / predicates
            | "AND"
            | "OR"
            | "NOT"
            | "IN"
            | "EXISTS"
            | "BETWEEN"
            | "LIKE"
            | "IS"
            | "NULL"
            | "ANY"
            // Clauses
            | "ORDER"
            | "BY"
            | "GROUP"
            | "HAVING"
            | "LIMIT"
            | "OFFSET"
            | "ASC"
            | "DESC"
            | "DISTINCT"
            | "ALL"
            | "AS"
            | "FETCH"
            | "NEXT"
            | "ROWS"
            | "ONLY"
            // CASE
            | "CASE"
            | "WHEN"
            | "THEN"
            | "ELSE"
            | "END"
            // Set operations
            | "UNION"
            | "INTERSECT"
            | "EXCEPT"
            | "MINUS"
            // CTE
            | "WITH"
            | "RECURSIVE"
            // DDL
            | "CREATE"
            | "ALTER"
            | "DROP"
            | "TABLE"
            | "VIEW"
            | "INDEX"
            | "SEQUENCE"
            | "CONSTRAINT"
            | "PRIMARY"
            | "KEY"
            | "FOREIGN"
            | "REFERENCES"
            | "UNIQUE"
            | "DEFAULT"
            | "CASCADE"
            | "RENAME"
            | "TO"
            // Transaction
            | "BEGIN"
            | "COMMIT"
            | "ROLLBACK"
            | "SAVEPOINT"
            // PL/SQL — structure
            | "DECLARE"
            | "BODY"
            | "REPLACE"
            | "PACKAGE"
            | "PROCEDURE"
            | "FUNCTION"
            | "TRIGGER"
            | "TYPE"
            | "SUBTYPE"
            | "RECORD"
            | "OBJECT"
            // PL/SQL — control flow
            | "IF"
            | "ELSIF"
            | "LOOP"
            | "FOR"
            | "WHILE"
            | "EXIT"
            | "CONTINUE"
            | "RETURN"
            | "EXCEPTION"
            | "RAISE"
            // PL/SQL — cursors & bulk
            | "CURSOR"
            | "OPEN"
            | "CLOSE"
            | "BULK"
            | "COLLECT"
            | "FORALL"
            | "PIPE"
            | "ROW"
            | "PIPELINED"
            // PL/SQL — execution
            | "EXECUTE"
            | "IMMEDIATE"
            | "PRAGMA"
            | "EXEC"
            | "CALL"
            // PL/SQL — type modifiers
            | "OF"
            | "CONSTANT"
            | "NOCOPY"
            | "DETERMINISTIC"
            // Data types (common ones that shouldn't be treated as identifiers)
            | "NUMBER"
            | "VARCHAR2"
            | "VARCHAR"
            | "CHAR"
            | "CLOB"
            | "BLOB"
            | "DATE"
            | "TIMESTAMP"
            | "BOOLEAN"
            | "INTEGER"
            | "INT"
            | "FLOAT"
            | "BINARY_INTEGER"
            | "PLS_INTEGER"
    )
}
