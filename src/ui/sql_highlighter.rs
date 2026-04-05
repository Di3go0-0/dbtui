use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use vimltui::SyntaxHighlighter;

const SQL_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "INSERT",
    "INTO",
    "UPDATE",
    "DELETE",
    "SET",
    "JOIN",
    "LEFT",
    "RIGHT",
    "INNER",
    "OUTER",
    "FULL",
    "CROSS",
    "ON",
    "AND",
    "OR",
    "NOT",
    "IN",
    "IS",
    "NULL",
    "LIKE",
    "BETWEEN",
    "EXISTS",
    "AS",
    "ORDER",
    "BY",
    "GROUP",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "DISTINCT",
    "UNION",
    "ALL",
    "CREATE",
    "ALTER",
    "DROP",
    "TABLE",
    "INDEX",
    "VIEW",
    "BEGIN",
    "END",
    "COMMIT",
    "ROLLBACK",
    "DECLARE",
    "CURSOR",
    "FETCH",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "ASC",
    "DESC",
    "COUNT",
    "SUM",
    "AVG",
    "MAX",
    "MIN",
    "PROCEDURE",
    "FUNCTION",
    "PACKAGE",
    "BODY",
    "REPLACE",
    "VALUES",
    "WITH",
    "RECURSIVE",
    "TRIGGER",
    "GRANT",
    "REVOKE",
    "TYPE",
    "RETURN",
    "IF",
    "ELSIF",
    "LOOP",
    "FOR",
    "WHILE",
    "EXIT",
    "EXCEPTION",
    "RAISE",
    "PRAGMA",
    "EXECUTE",
    "IMMEDIATE",
    "BULK",
    "COLLECT",
    "FORALL",
    "OPEN",
    "CLOSE",
    "DBMS_OUTPUT",
    "PUT_LINE",
];

pub struct SqlHighlighter {
    pub keyword: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub operator: Color,
    pub bind_var: Color,
}

impl SqlHighlighter {
    pub fn from_theme(theme: &crate::ui::theme::Theme) -> Self {
        Self {
            keyword: theme.sql_keyword,
            string: theme.sql_string,
            number: theme.sql_number,
            comment: theme.sql_comment,
            operator: theme.sql_operator,
            bind_var: theme.sql_bind_var,
        }
    }
}

impl SyntaxHighlighter for SqlHighlighter {
    fn highlight_line<'a>(&self, line: &'a str, spans: &mut Vec<Span<'a>>) {
        if line.is_empty() {
            return;
        }

        // Check for line comment
        if let Some(comment_pos) = line.find("--") {
            if comment_pos > 0 {
                self.highlight_tokens(&line[..comment_pos], spans);
            }
            spans.push(Span::styled(
                &line[comment_pos..],
                Style::default()
                    .fg(self.comment)
                    .add_modifier(Modifier::ITALIC),
            ));
            return;
        }

        self.highlight_tokens(line, spans);
    }

    fn highlight_segment<'a>(&self, text: &'a str, spans: &mut Vec<Span<'a>>) {
        self.highlight_line(text, spans);
    }
}

impl SqlHighlighter {
    fn highlight_tokens<'a>(&self, text: &'a str, spans: &mut Vec<Span<'a>>) {
        let mut remaining = text;

        while !remaining.is_empty() {
            // Skip leading whitespace
            if remaining.starts_with(|c: char| c.is_whitespace()) {
                let ws_end = remaining
                    .find(|c: char| !c.is_whitespace())
                    .unwrap_or(remaining.len());
                spans.push(Span::raw(&remaining[..ws_end]));
                remaining = &remaining[ws_end..];
                continue;
            }

            // String literal
            if remaining.starts_with('\'') {
                let end = remaining[1..]
                    .find('\'')
                    .map(|p| p + 2)
                    .unwrap_or(remaining.len());
                spans.push(Span::styled(
                    &remaining[..end],
                    Style::default().fg(self.string),
                ));
                remaining = &remaining[end..];
                continue;
            }

            // Bind variable :name (Oracle/MySQL) or $name/$1 (PostgreSQL)
            if remaining.starts_with(':')
                && remaining.len() > 1
                && remaining.as_bytes()[1].is_ascii_alphanumeric()
            {
                let end = remaining[1..]
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .map(|p| p + 1)
                    .unwrap_or(remaining.len());
                spans.push(Span::styled(
                    &remaining[..end],
                    Style::default().fg(self.bind_var),
                ));
                remaining = &remaining[end..];
                continue;
            }
            if remaining.starts_with('$')
                && remaining.len() > 1
                && remaining.as_bytes()[1].is_ascii_alphanumeric()
            {
                let end = remaining[1..]
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .map(|p| p + 1)
                    .unwrap_or(remaining.len());
                spans.push(Span::styled(
                    &remaining[..end],
                    Style::default().fg(self.bind_var),
                ));
                remaining = &remaining[end..];
                continue;
            }

            // Number
            if remaining.starts_with(|c: char| c.is_ascii_digit()) {
                let end = remaining
                    .find(|c: char| !c.is_ascii_digit() && c != '.')
                    .unwrap_or(remaining.len());
                spans.push(Span::styled(
                    &remaining[..end],
                    Style::default().fg(self.number),
                ));
                remaining = &remaining[end..];
                continue;
            }

            // Word (potential keyword or identifier)
            if remaining.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                let end = remaining
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .unwrap_or(remaining.len());
                let word = &remaining[..end];
                let upper = word.to_uppercase();

                if SQL_KEYWORDS.contains(&upper.as_str()) {
                    spans.push(Span::styled(
                        word,
                        Style::default()
                            .fg(self.keyword)
                            .add_modifier(Modifier::BOLD),
                    ));
                } else {
                    spans.push(Span::raw(word));
                }
                remaining = &remaining[end..];
                continue;
            }

            // Operators and punctuation
            let end = remaining
                .find(|c: char| c.is_alphanumeric() || c == '_' || c == '\'' || c.is_whitespace())
                .unwrap_or(remaining.len())
                .max(1);
            spans.push(Span::styled(
                &remaining[..end],
                Style::default().fg(self.operator),
            ));
            remaining = &remaining[end..];
        }
    }
}
