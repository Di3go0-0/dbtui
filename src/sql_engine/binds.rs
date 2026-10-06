//! Bind variables (`:name`) in SQL text: finding them and substituting values.

use std::ops::Range;

/// Every `:name` placeholder in `query`, in order, as the byte range of the
/// whole placeholder (colon included) and its name.
///
/// String literals and line comments are skipped, and `::` (a PostgreSQL
/// cast) and `:=` (PL/SQL assignment) are not placeholders.
pub fn bind_occurrences(query: &str) -> Vec<(Range<usize>, &str)> {
    let bytes = query.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        // Skip string literals
        if bytes[i] == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                i += 1;
            }
            i += 1;
            continue;
        }
        // Skip line comments
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Detect :name (but not ::)
        if bytes[i] == b':'
            && i + 1 < bytes.len()
            && bytes[i + 1].is_ascii_alphabetic()
            && (i == 0 || bytes[i - 1] != b':')
        {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            found.push((i..end, &query[start..end]));
            i = end;
            continue;
        }
        i += 1;
    }

    found
}

/// Unique bind variable names in `query`, in order of first appearance.
pub fn bind_names(query: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (_, name) in bind_occurrences(query) {
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    names
}

/// Replace each placeholder with its value from `values`; an empty value
/// becomes `NULL`, and a placeholder with no entry is left as written.
///
/// The text is rebuilt from the scanned placeholders in a single pass. A
/// find-and-replace per variable is wrong twice over: `:id` also matches the
/// start of `:id2`, and it reaches inside string literals.
pub fn substitute_binds(query: &str, values: &[(String, String)]) -> String {
    let mut out = String::with_capacity(query.len());
    let mut copied = 0;
    for (range, name) in bind_occurrences(query) {
        let Some((_, value)) = values.iter().find(|(n, _)| n == name) else {
            continue;
        };
        out.push_str(&query[copied..range.start]);
        out.push_str(if value.is_empty() { "NULL" } else { value });
        copied = range.end;
    }
    out.push_str(&query[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn names_are_unique_and_ordered() {
        assert_eq!(
            bind_names("SELECT :b, :a, :b FROM t WHERE x = :a"),
            vec!["b", "a"]
        );
    }

    #[test]
    fn casts_assignments_literals_and_comments_are_not_binds() {
        assert!(bind_names("SELECT x::text, '12:30:00' -- :nope\nFROM t").is_empty());
        assert!(bind_names("BEGIN v := 1; END;").is_empty());
    }

    #[test]
    fn a_name_that_prefixes_another_is_not_corrupted() {
        let sql = "WHERE a = :id AND b = :id2";
        assert_eq!(
            substitute_binds(sql, &values(&[("id", "5"), ("id2", "9")])),
            "WHERE a = 5 AND b = 9"
        );
    }

    #[test]
    fn literals_are_left_alone_and_empty_means_null() {
        let sql = "SELECT ':id' FROM t WHERE a = :id AND b = :missing";
        assert_eq!(
            substitute_binds(sql, &values(&[("id", "")])),
            "SELECT ':id' FROM t WHERE a = NULL AND b = :missing"
        );
    }
}
