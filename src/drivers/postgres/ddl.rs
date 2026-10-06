//! Catalog queries and DDL text for PostgreSQL tables.
//!
//! Everything here reads `pg_catalog` rather than `information_schema`: the
//! ANSI views hide columns of types they cannot describe (`ARRAY`,
//! `USER-DEFINED`), report integer "precision" that is not valid DDL, and
//! cannot pair the two sides of a composite or cross-schema foreign key.

/// Columns of one table, in definition order: name, type as written in DDL
/// (`format_type`), NOT NULL flag and default expression.
pub(super) const DDL_COLUMNS_SQL: &str = "\
    SELECT a.attname::text, \
           pg_catalog.format_type(a.atttypid, a.atttypmod), \
           a.attnotnull, \
           pg_catalog.pg_get_expr(d.adbin, d.adrelid) \
    FROM pg_catalog.pg_attribute a \
    JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
    LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
    WHERE n.nspname = $1 AND c.relname = $2 \
      AND a.attnum > 0 AND NOT a.attisdropped \
    ORDER BY a.attnum";

/// Primary-key column names of one table, in key order.
pub(super) const PRIMARY_KEY_SQL: &str = "\
    SELECT a.attname::text \
    FROM pg_catalog.pg_constraint con \
    JOIN pg_catalog.pg_class c ON c.oid = con.conrelid \
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
    CROSS JOIN LATERAL unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord) \
    JOIN pg_catalog.pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum \
    WHERE con.contype = 'p' AND n.nspname = $1 AND c.relname = $2 \
    ORDER BY k.ord";

/// One row per column, in definition order. The primary-key flag is an
/// `EXISTS`, so a column that also belongs to a foreign key or a unique
/// constraint is still returned exactly once — the grid addresses columns by
/// position when it builds `WHERE pk = ...`.
pub(super) const COLUMNS_SQL: &str = "\
    SELECT c.column_name::text AS column_name, \
           c.data_type::text AS data_type, \
           c.is_nullable::text AS is_nullable, \
           EXISTS ( \
               SELECT 1 \
               FROM pg_catalog.pg_constraint con \
               JOIN pg_catalog.pg_class t ON t.oid = con.conrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace \
               JOIN pg_catalog.pg_attribute a \
                 ON a.attrelid = con.conrelid AND a.attnum = ANY (con.conkey) \
               WHERE con.contype = 'p' \
                 AND n.nspname = c.table_schema \
                 AND t.relname = c.table_name \
                 AND a.attname = c.column_name \
           ) AS is_pk \
    FROM information_schema.columns c \
    WHERE c.table_schema = $1 AND c.table_name = $2 \
    ORDER BY c.ordinal_position";

/// One row per foreign-key column pair. `conkey` and `confkey` are unnested
/// side by side, so the n-th referencing column is matched with the n-th
/// referenced one, and the referenced table is resolved by oid, whatever
/// schema it lives in.
pub(super) const FOREIGN_KEYS_SQL: &str = "\
    SELECT con.conname::text AS constraint_name, \
           att.attname::text AS column_name, \
           rns.nspname::text AS ref_schema, \
           rcl.relname::text AS ref_table, \
           ratt.attname::text AS ref_column \
    FROM pg_catalog.pg_constraint con \
    JOIN pg_catalog.pg_class cl ON cl.oid = con.conrelid \
    JOIN pg_catalog.pg_namespace ns ON ns.oid = cl.relnamespace \
    JOIN pg_catalog.pg_class rcl ON rcl.oid = con.confrelid \
    JOIN pg_catalog.pg_namespace rns ON rns.oid = rcl.relnamespace \
    CROSS JOIN LATERAL unnest(con.conkey, con.confkey) \
         WITH ORDINALITY AS k(attnum, ref_attnum, ord) \
    JOIN pg_catalog.pg_attribute att \
      ON att.attrelid = con.conrelid AND att.attnum = k.attnum \
    JOIN pg_catalog.pg_attribute ratt \
      ON ratt.attrelid = con.confrelid AND ratt.attnum = k.ref_attnum \
    WHERE con.contype = 'f' AND ns.nspname = $1 AND cl.relname = $2 \
    ORDER BY con.conname, k.ord";

/// A column as `get_table_ddl` reads it from the catalog.
pub(super) struct DdlColumn {
    pub name: String,
    /// Already valid DDL: `integer`, `numeric(10,2)`, `text[]`, `public.mood`.
    pub data_type: String,
    pub not_null: bool,
    pub default: Option<String>,
}

/// Assemble a `CREATE TABLE` statement.
pub(super) fn build_table_ddl(
    schema: &str,
    table: &str,
    columns: &[DdlColumn],
    primary_key: &[String],
) -> String {
    let mut lines: Vec<String> = columns
        .iter()
        .map(|c| {
            let not_null = if c.not_null { " NOT NULL" } else { "" };
            let default = c
                .default
                .as_deref()
                .map(|d| format!(" DEFAULT {d}"))
                .unwrap_or_default();
            format!(
                "    {} {}{not_null}{default}",
                quote_ident(&c.name),
                c.data_type
            )
        })
        .collect();

    if !primary_key.is_empty() {
        let cols: Vec<String> = primary_key.iter().map(|c| quote_ident(c)).collect();
        lines.push(format!("    PRIMARY KEY ({})", cols.join(", ")));
    }

    format!(
        "CREATE TABLE {}.{} (\n{}\n);",
        quote_ident(schema),
        quote_ident(table),
        lines.join(",\n")
    )
}

/// Quote an identifier only when PostgreSQL would not read it back as
/// written: anything but lower-case letters, digits, `_` and `$`, a leading
/// digit or `$`, or a reserved word. Embedded quotes are doubled.
pub(super) fn quote_ident(name: &str) -> String {
    let mut chars = name.chars();
    let plain_start = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_');
    let plain_rest =
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '$');
    if plain_start && plain_rest && !RESERVED_WORDS.contains(&name) {
        return name.to_string();
    }
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Words PostgreSQL does not accept as a bare column or table name.
const RESERVED_WORDS: &[&str] = &[
    "all",
    "analyse",
    "analyze",
    "and",
    "any",
    "array",
    "as",
    "asc",
    "asymmetric",
    "authorization",
    "binary",
    "both",
    "case",
    "cast",
    "check",
    "collate",
    "collation",
    "column",
    "concurrently",
    "constraint",
    "create",
    "cross",
    "current_catalog",
    "current_date",
    "current_role",
    "current_schema",
    "current_time",
    "current_timestamp",
    "current_user",
    "default",
    "deferrable",
    "desc",
    "distinct",
    "do",
    "else",
    "end",
    "except",
    "false",
    "fetch",
    "for",
    "foreign",
    "freeze",
    "from",
    "full",
    "grant",
    "group",
    "having",
    "ilike",
    "in",
    "initially",
    "inner",
    "intersect",
    "into",
    "is",
    "isnull",
    "join",
    "lateral",
    "leading",
    "left",
    "like",
    "limit",
    "localtime",
    "localtimestamp",
    "natural",
    "not",
    "notnull",
    "null",
    "offset",
    "on",
    "only",
    "or",
    "order",
    "outer",
    "overlaps",
    "placing",
    "primary",
    "references",
    "returning",
    "right",
    "select",
    "session_user",
    "similar",
    "some",
    "symmetric",
    "system_user",
    "table",
    "tablesample",
    "then",
    "to",
    "trailing",
    "true",
    "union",
    "unique",
    "user",
    "using",
    "variadic",
    "verbose",
    "when",
    "where",
    "window",
    "with",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, data_type: &str, not_null: bool, default: Option<&str>) -> DdlColumn {
        DdlColumn {
            name: name.to_string(),
            data_type: data_type.to_string(),
            not_null,
            default: default.map(str::to_string),
        }
    }

    #[test]
    fn plain_identifiers_are_left_alone() {
        assert_eq!(quote_ident("users"), "users");
        assert_eq!(quote_ident("_tmp$1"), "_tmp$1");
        assert_eq!(quote_ident("order_items"), "order_items");
    }

    #[test]
    fn identifiers_that_need_quoting() {
        assert_eq!(quote_ident("Users"), "\"Users\"");
        assert_eq!(quote_ident("first name"), "\"first name\"");
        assert_eq!(quote_ident("1st"), "\"1st\"");
        assert_eq!(quote_ident("$x"), "\"$x\"");
        assert_eq!(quote_ident("order"), "\"order\"");
        assert_eq!(quote_ident("user"), "\"user\"");
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
        assert_eq!(quote_ident("año"), "\"año\"");
        assert_eq!(quote_ident(""), "\"\"");
    }

    #[test]
    fn ddl_uses_catalog_types_verbatim() {
        let ddl = build_table_ddl(
            "public",
            "orders",
            &[
                col(
                    "id",
                    "integer",
                    true,
                    Some("nextval('orders_id_seq'::regclass)"),
                ),
                col("total", "numeric(10,2)", false, None),
                col("tags", "text[]", false, None),
                col("status", "mood", true, Some("'ok'::mood")),
            ],
            &["id".to_string()],
        );
        assert_eq!(
            ddl,
            "CREATE TABLE public.orders (\n\
             \x20   id integer NOT NULL DEFAULT nextval('orders_id_seq'::regclass),\n\
             \x20   total numeric(10,2),\n\
             \x20   tags text[],\n\
             \x20   status mood NOT NULL DEFAULT 'ok'::mood,\n\
             \x20   PRIMARY KEY (id)\n\
             );"
        );
    }

    #[test]
    fn ddl_quotes_names_and_handles_composite_keys() {
        let ddl = build_table_ddl(
            "Sales",
            "order",
            &[
                col("Order Id", "bigint", true, None),
                col("line", "integer", true, None),
            ],
            &["Order Id".to_string(), "line".to_string()],
        );
        assert_eq!(
            ddl,
            "CREATE TABLE \"Sales\".\"order\" (\n\
             \x20   \"Order Id\" bigint NOT NULL,\n\
             \x20   line integer NOT NULL,\n\
             \x20   PRIMARY KEY (\"Order Id\", line)\n\
             );"
        );
    }

    #[test]
    fn ddl_without_a_primary_key_has_no_trailing_comma() {
        let ddl = build_table_ddl("public", "log", &[col("msg", "text", false, None)], &[]);
        assert_eq!(ddl, "CREATE TABLE public.log (\n    msg text\n);");
    }
}
