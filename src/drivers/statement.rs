//! Dialect-aware statement classification.
//!
//! Drivers ask the server whether a statement returns rows wherever the wire
//! protocol can answer (PostgreSQL statement metadata, Oracle's statement
//! type, MySQL's result packets). This module covers what is left: engines
//! whose API needs the answer before the statement is sent, and the fallback
//! used to label an empty result.

use crate::core::adapter::{is_row_producing_query, leading_keyword, skip_leading_noise};
use crate::core::models::DatabaseType;

/// Whether `sql` is expected to return a result set on `db`.
pub(crate) fn is_row_producing(db: DatabaseType, sql: &str) -> bool {
    if is_row_producing_query(sql) {
        return true;
    }
    let keyword = leading_keyword(sql);
    match db {
        // `EXPLAIN PLAN FOR` only fills PLAN_TABLE and `RETURNING ... INTO`
        // writes to bind variables, so neither yields rows on Oracle.
        DatabaseType::Oracle => false,
        DatabaseType::PostgreSQL => match keyword.as_str() {
            "VALUES" | "TABLE" | "SHOW" | "EXPLAIN" | "FETCH" => true,
            "INSERT" | "UPDATE" | "DELETE" | "MERGE" => contains_keyword(db, sql, &["RETURNING"]),
            _ => false,
        },
        DatabaseType::MySQL => matches!(
            mysql_leading_keyword(sql).as_str(),
            "SELECT"
                | "WITH"
                | "SHOW"
                | "DESCRIBE"
                | "DESC"
                | "EXPLAIN"
                | "ANALYZE"
                | "CALL"
                | "VALUES"
                | "TABLE"
                | "CHECK"
                | "CHECKSUM"
                | "OPTIMIZE"
                | "REPAIR"
                | "HELP"
        ),
        DatabaseType::SqlServer => match keyword.as_str() {
            "EXEC" | "EXECUTE" => true,
            "INSERT" | "UPDATE" | "DELETE" | "MERGE" => contains_keyword(db, sql, &["OUTPUT"]),
            // A batch that opens with procedural T-SQL returns rows whenever
            // a later statement in it does. `SET` and `USE` are left out on
            // purpose: sent as a raw batch they would change the options or
            // the database of a pooled session for whoever uses it next.
            "DECLARE" | "IF" | "BEGIN" | "WHILE" => {
                contains_keyword(db, sql, &["SELECT", "EXEC", "EXECUTE"])
            }
            _ => false,
        },
    }
}

/// `leading_keyword` for MySQL, which also has `# ...` line comments.
fn mysql_leading_keyword(sql: &str) -> String {
    let mut rest = sql;
    loop {
        rest = &rest[skip_leading_noise(rest)..];
        match rest.strip_prefix('#') {
            Some(comment) => rest = comment.find('\n').map_or("", |nl| &comment[nl..]),
            None => return leading_keyword(rest),
        }
    }
}

/// Whether any of `keywords` (upper-case) appears in `sql` as a bare word.
pub(crate) fn contains_keyword(db: DatabaseType, sql: &str, keywords: &[&str]) -> bool {
    bare_words(db, sql)
        .iter()
        .any(|w| keywords.contains(&w.as_str()))
}

/// Upper-cased words of `sql` that sit outside string literals, quoted
/// identifiers and comments, in order of appearance.
pub(crate) fn bare_words(db: DatabaseType, sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut words = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        i = match c {
            '-' if next == Some('-') => skip_line(&chars, i),
            '#' if db == DatabaseType::MySQL => skip_line(&chars, i),
            '/' if next == Some('*') => skip_block_comment(&chars, i),
            '\'' | '"' => skip_quoted(&chars, i, c),
            '`' if db == DatabaseType::MySQL => skip_quoted(&chars, i, '`'),
            '[' if db == DatabaseType::SqlServer => skip_quoted(&chars, i, ']'),
            '$' if db == DatabaseType::PostgreSQL => skip_dollar_quoted(&chars, i),
            c if is_word_char(c) => {
                let end = word_end(&chars, i);
                words.push(
                    chars[i..end]
                        .iter()
                        .map(|c| c.to_ascii_uppercase())
                        .collect(),
                );
                end
            }
            _ => i + 1,
        };
    }
    words
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '@'
}

fn word_end(chars: &[char], start: usize) -> usize {
    let mut i = start;
    while i < chars.len() && (is_word_char(chars[i]) || chars[i] == '$') {
        i += 1;
    }
    i
}

/// Index just past the end of the line starting at `start`.
fn skip_line(chars: &[char], start: usize) -> usize {
    let mut i = start;
    while i < chars.len() && chars[i] != '\n' {
        i += 1;
    }
    i
}

/// Index just past the `/* ... */` comment opening at `start`.
fn skip_block_comment(chars: &[char], start: usize) -> usize {
    let mut i = start + 2;
    while i < chars.len() {
        if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
            return i + 2;
        }
        i += 1;
    }
    chars.len()
}

/// Index just past the literal or quoted identifier opening at `start` and
/// closed by `close`. A doubled closing character is an escaped one.
fn skip_quoted(chars: &[char], start: usize, close: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == close {
            if chars.get(i + 1) == Some(&close) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

/// Index just past a PostgreSQL `$tag$ ... $tag$` literal opening at `start`,
/// or past the lone `$` when it does not open one (`$1` parameters).
fn skip_dollar_quoted(chars: &[char], start: usize) -> usize {
    let mut tag_end = start + 1;
    while tag_end < chars.len() && (chars[tag_end].is_alphabetic() || chars[tag_end] == '_') {
        tag_end += 1;
    }
    if chars.get(tag_end) != Some(&'$') {
        return start + 1;
    }
    let tag = &chars[start..=tag_end];
    let mut i = tag_end + 1;
    while i + tag.len() <= chars.len() {
        if &chars[i..i + tag.len()] == tag {
            return i + tag.len();
        }
        i += 1;
    }
    chars.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use DatabaseType::{MySQL, Oracle, PostgreSQL, SqlServer};

    #[test]
    fn select_and_with_return_rows_everywhere() {
        for db in [Oracle, PostgreSQL, MySQL, SqlServer] {
            assert!(is_row_producing(db, "SELECT 1"));
            assert!(is_row_producing(
                db,
                "-- c\nWITH x AS (SELECT 1) SELECT * FROM x"
            ));
            assert!(is_row_producing(db, "(SELECT 1) UNION (SELECT 2)"));
            assert!(!is_row_producing(db, "UPDATE t SET a = 1"));
            assert!(!is_row_producing(db, "CREATE TABLE t (id INT)"));
        }
    }

    #[test]
    fn postgres_row_statements() {
        for sql in [
            "EXPLAIN SELECT 1",
            "explain analyze select 1",
            "SHOW search_path",
            "VALUES (1), (2)",
            "TABLE users",
            "INSERT INTO t (a) VALUES (1) RETURNING id",
            "update t set a = 1 returning *",
            "DELETE FROM t WHERE a = 1 RETURNING a",
        ] {
            assert!(is_row_producing(PostgreSQL, sql), "{sql}");
        }
    }

    #[test]
    fn postgres_returning_must_be_a_real_keyword() {
        assert!(!is_row_producing(
            PostgreSQL,
            "INSERT INTO t (note) VALUES ('returning soon')"
        ));
        assert!(!is_row_producing(
            PostgreSQL,
            "UPDATE t SET a = 1 -- returning\n"
        ));
        assert!(!is_row_producing(
            PostgreSQL,
            "UPDATE \"returning\" SET a = $$ returning $$"
        ));
        assert!(!is_row_producing(
            PostgreSQL,
            "UPDATE t SET returning_at = 1"
        ));
    }

    #[test]
    fn mysql_row_statements() {
        for sql in [
            "SHOW TABLES",
            "DESCRIBE users",
            "desc users",
            "EXPLAIN SELECT 1",
            "CALL report(1)",
            "# note\nSHOW DATABASES",
            "# a\n-- b\n/* c */ # d\nselect 1",
            "CHECK TABLE t",
        ] {
            assert!(is_row_producing(MySQL, sql), "{sql}");
        }
        assert!(!is_row_producing(MySQL, "# select 1\nUPDATE t SET a = 1"));
        assert!(!is_row_producing(MySQL, "USE shop"));
        assert!(!is_row_producing(MySQL, "#"));
    }

    #[test]
    fn sqlserver_row_statements() {
        for sql in [
            "EXEC sp_who",
            "execute dbo.report @id = 1",
            "INSERT INTO t (a) OUTPUT inserted.id VALUES (1)",
            "DELETE FROM [t] OUTPUT deleted.* WHERE a = 1",
            "DECLARE @x INT = 1; SELECT @x",
            "IF EXISTS (SELECT 1 FROM t) EXEC dbo.report",
        ] {
            assert!(is_row_producing(SqlServer, sql), "{sql}");
        }
        assert!(!is_row_producing(
            SqlServer,
            "DECLARE @x INT = 1; UPDATE t SET a = @x"
        ));
        assert!(!is_row_producing(
            SqlServer,
            "UPDATE [output] SET a = 'OUTPUT'"
        ));
        assert!(!is_row_producing(
            SqlServer,
            "SET ROWCOUNT 10; SELECT * FROM t"
        ));
        assert!(!is_row_producing(SqlServer, "USE other; SELECT * FROM t"));
    }

    #[test]
    fn oracle_stays_narrow() {
        assert!(!is_row_producing(
            Oracle,
            "EXPLAIN PLAN FOR SELECT 1 FROM dual"
        ));
        assert!(!is_row_producing(
            Oracle,
            "INSERT INTO t (a) VALUES (1) RETURNING id INTO :id"
        ));
        assert!(!is_row_producing(Oracle, "BEGIN NULL; END;"));
    }

    #[test]
    fn bare_words_skip_literals_identifiers_and_comments() {
        assert_eq!(
            bare_words(PostgreSQL, "select 'it''s' , \"Mixed\" /* x */ from t -- y"),
            vec!["SELECT", "FROM", "T"]
        );
        assert_eq!(
            bare_words(MySQL, "select `a``b` from t # tail"),
            vec!["SELECT", "FROM", "T"]
        );
        assert_eq!(
            bare_words(SqlServer, "select [a]]b] from @t"),
            vec!["SELECT", "FROM", "@T"]
        );
        assert_eq!(
            bare_words(
                PostgreSQL,
                "do $body$ begin null; end $body$ language plpgsql"
            ),
            vec!["DO", "LANGUAGE", "PLPGSQL"]
        );
        assert_eq!(bare_words(PostgreSQL, "select $1"), vec!["SELECT", "1"]);
    }

    #[test]
    fn bare_words_survive_unterminated_input() {
        assert_eq!(bare_words(PostgreSQL, "select 'abc"), vec!["SELECT"]);
        assert_eq!(bare_words(PostgreSQL, "select /* café"), vec!["SELECT"]);
        assert_eq!(bare_words(PostgreSQL, "select $q$ x"), vec!["SELECT"]);
        assert!(bare_words(MySQL, "").is_empty());
    }
}
