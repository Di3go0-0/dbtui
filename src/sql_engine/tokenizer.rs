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

/// Detect a `qualifier1.qualifier2.` chain immediately before the cursor.
/// Returns `(qualifier1, qualifier2)` when the text ends with two identifiers
/// separated by a single dot, e.g. `schema1.emp_pkg.<cursor>`.
///
/// Whitespace and word characters after the trailing dot are ignored — i.e.
/// we still match `schema.pkg.get` because the user is in the middle of
/// typing the third identifier.
pub fn two_identifiers_before_dot(before_cursor: &str) -> Option<(&str, &str)> {
    let bytes = before_cursor.as_bytes();
    let mut pos = bytes.len();
    // Skip the in-progress identifier the user is currently typing
    while pos > 0 && (bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'_') {
        pos -= 1;
    }
    if pos == 0 || bytes[pos - 1] != b'.' {
        return None;
    }
    let dot2 = pos - 1;
    // Second identifier (immediately before dot2)
    let id2_end = dot2;
    let mut id2_start = id2_end;
    while id2_start > 0
        && (bytes[id2_start - 1].is_ascii_alphanumeric() || bytes[id2_start - 1] == b'_')
    {
        id2_start -= 1;
    }
    if id2_start >= id2_end {
        return None;
    }
    // Now expect another dot before id2_start
    if id2_start == 0 || bytes[id2_start - 1] != b'.' {
        return None;
    }
    let dot1 = id2_start - 1;
    // First identifier (immediately before dot1)
    let id1_end = dot1;
    let mut id1_start = id1_end;
    while id1_start > 0
        && (bytes[id1_start - 1].is_ascii_alphanumeric() || bytes[id1_start - 1] == b'_')
    {
        id1_start -= 1;
    }
    if id1_start >= id1_end {
        return None;
    }
    Some((
        &before_cursor[id1_start..id1_end],
        &before_cursor[id2_start..id2_end],
    ))
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

    let mut words = Vec::new();

    // Current line up to cursor
    if row < lines.len() {
        let before = &lines[row][..col.min(lines[row].len())];
        extract_words_reverse(before, &mut words);

        // If cursor is after a space (or at start of line), there is no partial
        // prefix word. Insert an empty sentinel so words[0] (which gets skipped
        // as the "prefix") is always present and the first real token is at i=1.
        let at_word_boundary = before.is_empty()
            || before
                .as_bytes()
                .last()
                .is_some_and(|&b| !b.is_ascii_alphanumeric() && b != b'_');
        if at_word_boundary {
            words.insert(0, String::new());
        }
    }

    // Previous lines in the block
    for r in (0..row).rev() {
        if r < lines.len() {
            extract_words_reverse(&lines[r], &mut words);
        }
    }

    let mut idents_before_keyword = 0;
    // Track parenthesis depth: when scanning backwards, `)` increases depth
    // (entering a paren group), `(` decreases it (leaving).
    // At depth > 0 we're inside parentheses — skip keywords to avoid
    // confusing `ORDER BY` inside `OVER()` with a top-level `ORDER BY`.
    let mut paren_depth: i32 = 0;
    let mut skip_next_over = false;

    for (i, word) in words.iter().enumerate() {
        if i == 0 {
            continue;
        }

        // Track parens
        if word == ")" {
            paren_depth += 1;
            continue;
        }
        if word == "(" {
            if paren_depth > 0 {
                paren_depth -= 1;
                if paren_depth == 0 {
                    skip_next_over = true;
                }
            }
            continue;
        }

        let upper = word.to_uppercase();

        // After exiting a paren group, skip `OVER` so we don't re-enter
        // window function context from the outside.
        if skip_next_over {
            skip_next_over = false;
            if upper == "OVER" {
                continue;
            }
        }

        // Inside parentheses: skip everything (keywords and idents)
        if paren_depth > 0 {
            continue;
        }

        if !is_sql_keyword(&upper) {
            idents_before_keyword += 1;
            continue;
        }

        match upper.as_str() {
            "SELECT" => return CursorContext::SelectList,
            "FROM" | "JOIN" => {
                // Check if FROM is preceded by DELETE → DML target context
                let is_delete_from = upper == "FROM"
                    && words
                        .get(i + 1)
                        .is_some_and(|w| w.to_uppercase() == "DELETE");
                if is_delete_from {
                    if idents_before_keyword == 0 {
                        return CursorContext::TableTarget;
                    }
                    return CursorContext::AfterDeleteTable;
                }
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
                // After UPDATE table_name — suggest SET
                return CursorContext::AfterUpdateTable;
            }
            "DELETE" => {
                // DELETE with no idents after → suggest FROM keyword
                if idents_before_keyword == 0 {
                    return CursorContext::General; // will suggest FROM via general keywords
                }
                // DELETE FROM table — suggest WHERE
                return CursorContext::AfterDeleteTable;
            }
            "SET" => {
                if let Some(qn) = find_dml_target_table(&words, i, "UPDATE") {
                    return CursorContext::SetClause { target_table: qn };
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
            "ORDER" | "GROUP" | "OVER" | "PARTITION" => return CursorContext::OrderGroupBy,
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

/// Extract words and parentheses from a line in reverse order.
/// Parentheses are emitted as `"("` / `")"` tokens so the scanner can track depth.
fn extract_words_reverse(line: &str, words: &mut Vec<String>) {
    let bytes = line.as_bytes();
    let mut pos = bytes.len();

    while pos > 0 {
        // Skip non-word, non-paren characters
        while pos > 0
            && !bytes[pos - 1].is_ascii_alphanumeric()
            && bytes[pos - 1] != b'_'
            && bytes[pos - 1] != b'('
            && bytes[pos - 1] != b')'
        {
            pos -= 1;
        }
        if pos == 0 {
            break;
        }
        // Emit parentheses as single-char tokens
        if bytes[pos - 1] == b'(' || bytes[pos - 1] == b')' {
            words.push(String::from(bytes[pos - 1] as char));
            pos -= 1;
            continue;
        }
        let end = pos;
        while pos > 0 && (bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'_') {
            pos -= 1;
        }
        words.push(line[pos..end].to_string());
    }
}

/// In a reverse word list, scan backward from `start_idx` to find the target
/// table of a DML statement (UPDATE or DELETE). Returns a QualifiedName with
/// optional schema if the table was schema-qualified (e.g. `schema.table`).
///
/// `anchor_kw` is the keyword to stop at (e.g. "UPDATE" or "DELETE").
fn find_dml_target_table(
    words: &[String],
    start_idx: usize,
    anchor_kw: &str,
) -> Option<crate::sql_engine::models::QualifiedName> {
    use crate::sql_engine::models::QualifiedName;

    // Collect identifiers between start_idx and the anchor keyword
    let mut idents = Vec::new();
    for word in words.iter().skip(start_idx + 1) {
        let upper = word.to_uppercase();
        if upper == anchor_kw {
            break;
        }
        // Skip FROM (for DELETE FROM table)
        if upper == "FROM" {
            continue;
        }
        if is_sql_keyword(&upper) {
            break;
        }
        idents.push(word.clone());
    }

    // idents are in reverse order (closest to SET first)
    // For "UPDATE schema.table SET" → idents = ["table", "schema"]
    // For "UPDATE table SET" → idents = ["table"]
    match idents.len() {
        0 => None,
        1 => Some(QualifiedName {
            schema: None,
            name: idents[0].clone(),
        }),
        _ => Some(QualifiedName {
            schema: Some(idents.last()?.clone()),
            name: idents[0].clone(),
        }),
    }
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
            | "OVER"
            | "PARTITION"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql_engine::context::CursorContext;

    fn ctx(sql: &str) -> CursorContext {
        let lines: Vec<String> = sql.lines().map(String::from).collect();
        let row = lines.len() - 1;
        let col = lines[row].len();
        find_keyword_context(&lines, row, col)
    }

    // --- UPDATE ---

    #[test]
    fn update_suggests_table() {
        assert!(matches!(ctx("UPDATE "), CursorContext::TableTarget));
    }

    #[test]
    fn update_table_suggests_set() {
        assert!(matches!(
            ctx("UPDATE orders "),
            CursorContext::AfterUpdateTable
        ));
    }

    #[test]
    fn update_schema_table_suggests_set() {
        assert!(matches!(
            ctx("UPDATE mydb.orders "),
            CursorContext::AfterUpdateTable
        ));
    }

    #[test]
    fn update_set_suggests_columns() {
        match ctx("UPDATE orders SET ") {
            CursorContext::SetClause { target_table } => {
                assert_eq!(target_table.name, "orders");
            }
            other => panic!("expected SetClause, got {other:?}"),
        }
    }

    #[test]
    fn update_set_schema_qualified() {
        match ctx("UPDATE mydb.orders SET ") {
            CursorContext::SetClause { target_table } => {
                assert_eq!(target_table.name, "orders");
                assert_eq!(target_table.schema.as_deref(), Some("mydb"));
            }
            other => panic!("expected SetClause, got {other:?}"),
        }
    }

    #[test]
    fn update_set_value_suggests_where() {
        // After SET col = val, typing next word → still in SetClause (columns + WHERE)
        match ctx("UPDATE orders\nSET total_amount = 2000\n") {
            CursorContext::SetClause { .. } => {}
            other => panic!("expected SetClause, got {other:?}"),
        }
    }

    #[test]
    fn update_where_suggests_predicates() {
        assert!(matches!(
            ctx("UPDATE orders SET total = 1 WHERE "),
            CursorContext::Predicate
        ));
    }

    #[test]
    fn over_clause_suggests_order_partition() {
        // Inside OVER( → should suggest ORDER, PARTITION, BY
        assert!(matches!(ctx("SUM(x) OVER( "), CursorContext::OrderGroupBy));
        assert!(matches!(
            ctx("SUM(x) OVER(\n    "),
            CursorContext::OrderGroupBy
        ));
    }

    #[test]
    fn partition_by_suggests_columns() {
        assert!(matches!(
            ctx("SUM(x) OVER(\n    PARTITION BY "),
            CursorContext::OrderGroupBy
        ));
    }

    #[test]
    fn over_order_by_does_not_escape_to_select() {
        // ORDER BY inside OVER() should NOT resolve to outer SELECT context
        let lines = vec![
            "SELECT".to_string(),
            "    RANK() OVER(".to_string(),
            "        ORDER BY ".to_string(),
        ];
        let context = find_keyword_context(&lines, 2, 17);
        assert!(
            matches!(context, CursorContext::OrderGroupBy),
            "got {context:?}"
        );
    }

    #[test]
    fn after_over_close_paren_is_select_list() {
        // After closing ) of OVER(), should be back in SelectList
        // SELECT RANK() OVER(ORDER BY x) |
        let lines = vec!["SELECT RANK() OVER(ORDER BY x) ".to_string()];
        let context = find_keyword_context(&lines, 0, 31);
        assert!(
            matches!(context, CursorContext::SelectList),
            "got {context:?}"
        );
    }

    #[test]
    fn update_set_value_multiline_where_prefix() {
        // User is typing "w" after value assignment → should still be SetClause
        let lines = vec![
            "UPDATE orders".to_string(),
            "SET total_amount = 2000".to_string(),
            "w".to_string(),
        ];
        let context = find_keyword_context(&lines, 2, 1);
        assert!(
            matches!(context, CursorContext::SetClause { .. }),
            "got {context:?}"
        );
    }

    // --- DELETE ---

    #[test]
    fn delete_from_suggests_table() {
        assert!(matches!(ctx("DELETE FROM "), CursorContext::TableTarget));
    }

    #[test]
    fn delete_from_table_suggests_where() {
        assert!(matches!(
            ctx("DELETE FROM orders "),
            CursorContext::AfterDeleteTable
        ));
    }

    #[test]
    fn delete_where_suggests_predicates() {
        assert!(matches!(
            ctx("DELETE FROM orders WHERE "),
            CursorContext::Predicate
        ));
    }

    // --- SELECT (regression) ---

    #[test]
    fn select_from_still_works() {
        assert!(matches!(ctx("SELECT * FROM "), CursorContext::TableRef));
    }

    #[test]
    fn select_where_still_works() {
        assert!(matches!(
            ctx("SELECT * FROM t WHERE "),
            CursorContext::Predicate
        ));
    }

    #[test]
    fn cursor_on_star_in_select() {
        // Cursor after * on second line → should be SelectList
        let lines = vec!["SELECT ".to_string(), "    *".to_string()];
        let context = find_keyword_context(&lines, 1, 5);
        assert!(
            matches!(context, CursorContext::SelectList),
            "got {context:?}"
        );
    }

    #[test]
    fn cursor_after_star_multiline() {
        // SELECT\n    *\nFROM orders — cursor after * before FROM
        let lines = vec![
            "SELECT".to_string(),
            "    *".to_string(),
            "FROM orders".to_string(),
        ];
        let context = find_keyword_context(&lines, 1, 5);
        assert!(
            matches!(context, CursorContext::SelectList),
            "got {context:?}"
        );
    }
}
