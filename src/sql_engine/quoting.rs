//! Identifier quoting for SQL the app generates itself (table browsing, grid
//! edits, drop/rename).
//!
//! Names reach these functions exactly as the catalog lists them, so the
//! quoted form always resolves. PostgreSQL and Oracle are only quoted when the
//! bare name would fold to something else, which keeps generated SQL readable
//! for the common case.

use crate::core::models::DatabaseType;

/// Quote a single identifier for `db_type`.
pub fn quote_ident(db_type: Option<DatabaseType>, ident: &str) -> String {
    match db_type {
        Some(DatabaseType::MySQL) => format!("`{}`", ident.replace('`', "``")),
        Some(DatabaseType::SqlServer) => format!("[{}]", ident.replace(']', "]]")),
        Some(DatabaseType::PostgreSQL) => {
            if is_plain(ident, |c| c.is_ascii_lowercase()) {
                ident.to_string()
            } else {
                double_quoted(ident)
            }
        }
        Some(DatabaseType::Oracle) => {
            if is_plain(ident, |c| c.is_ascii_uppercase()) {
                ident.to_string()
            } else {
                double_quoted(ident)
            }
        }
        None => ident.to_string(),
    }
}

/// Quote `schema.name`. On SQL Server the sidebar qualifies schemas as
/// `database.schema`; the schema part is split on its last dot so each level
/// gets its own brackets.
pub fn quote_qualified(db_type: Option<DatabaseType>, schema: &str, name: &str) -> String {
    let name = quote_ident(db_type, name);
    if schema.is_empty() {
        return name;
    }
    if db_type == Some(DatabaseType::SqlServer)
        && let Some((database, schema)) = schema.rsplit_once('.')
    {
        return format!(
            "{}.{}.{name}",
            quote_ident(db_type, database),
            quote_ident(db_type, schema)
        );
    }
    format!("{}.{name}", quote_ident(db_type, schema))
}

/// True when `ident` survives unquoted: it starts with a letter of the case
/// the engine folds to (or `_`) and continues with that case, digits, `_`, `$`
/// or `#`.
fn is_plain(ident: &str, is_folded_letter: impl Fn(char) -> bool) -> bool {
    let mut chars = ident.chars();
    match chars.next() {
        Some(c) if is_folded_letter(c) || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| is_folded_letter(c) || c.is_ascii_digit() || matches!(c, '_' | '$' | '#'))
}

fn double_quoted(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_quotes_only_when_folding_would_change_the_name() {
        let pg = Some(DatabaseType::PostgreSQL);
        assert_eq!(quote_ident(pg, "users"), "users");
        assert_eq!(quote_ident(pg, "UserAccounts"), "\"UserAccounts\"");
        assert_eq!(quote_ident(pg, "año"), "\"año\"");
        assert_eq!(quote_ident(pg, "we\"ird"), "\"we\"\"ird\"");
    }

    #[test]
    fn oracle_keeps_uppercase_bare() {
        let ora = Some(DatabaseType::Oracle);
        assert_eq!(quote_ident(ora, "EMPLOYEES"), "EMPLOYEES");
        assert_eq!(quote_ident(ora, "SYS$LOG"), "SYS$LOG");
        assert_eq!(quote_ident(ora, "MyTable"), "\"MyTable\"");
    }

    #[test]
    fn mysql_and_sqlserver_always_quote() {
        assert_eq!(quote_ident(Some(DatabaseType::MySQL), "a`b"), "`a``b`");
        assert_eq!(quote_ident(Some(DatabaseType::SqlServer), "a]b"), "[a]]b]");
    }

    #[test]
    fn qualified_names() {
        assert_eq!(
            quote_qualified(Some(DatabaseType::PostgreSQL), "public", "Order"),
            "public.\"Order\""
        );
        assert_eq!(
            quote_qualified(Some(DatabaseType::SqlServer), "Company.Sales.dbo", "t"),
            "[Company.Sales].[dbo].[t]"
        );
        assert_eq!(
            quote_qualified(Some(DatabaseType::MySQL), "shop", "items"),
            "`shop`.`items`"
        );
        assert_eq!(quote_qualified(None, "s", "t"), "s.t");
    }
}
