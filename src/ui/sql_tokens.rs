/// Shared SQL tokenizer for completion and diagnostics engines.
/// Provides position-preserving tokenization of SQL text.

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

/// Check if a word (already UPPERCASE) is a common SQL keyword.
pub fn is_sql_keyword(upper: &str) -> bool {
    matches!(
        upper,
        "SELECT"
            | "FROM"
            | "WHERE"
            | "INSERT"
            | "INTO"
            | "VALUES"
            | "UPDATE"
            | "SET"
            | "DELETE"
            | "CREATE"
            | "ALTER"
            | "DROP"
            | "TABLE"
            | "VIEW"
            | "INDEX"
            | "JOIN"
            | "INNER"
            | "LEFT"
            | "RIGHT"
            | "FULL"
            | "OUTER"
            | "CROSS"
            | "NATURAL"
            | "ON"
            | "USING"
            | "AND"
            | "OR"
            | "NOT"
            | "IN"
            | "EXISTS"
            | "BETWEEN"
            | "LIKE"
            | "IS"
            | "NULL"
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
            | "CASE"
            | "WHEN"
            | "THEN"
            | "ELSE"
            | "END"
            | "UNION"
            | "INTERSECT"
            | "EXCEPT"
            | "WITH"
            | "RECURSIVE"
            | "BEGIN"
            | "COMMIT"
            | "ROLLBACK"
            | "DECLARE"
            | "RETURN"
            | "IF"
            | "LOOP"
            | "FOR"
            | "WHILE"
            | "EXCEPTION"
            | "RAISE"
            | "PRAGMA"
            | "EXEC"
            | "EXECUTE"
            | "CALL"
    )
}
