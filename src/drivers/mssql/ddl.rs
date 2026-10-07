//! `CREATE TABLE` text for SQL Server tables.

use crate::core::models::Column;

/// Bracket-quote an identifier, doubling embedded `]`.
pub(super) fn bracket(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

/// Assemble a `CREATE TABLE` statement.
///
/// `catalog` is the database when the sidebar qualified the schema with one;
/// each part of the name is quoted on its own.
pub(super) fn build_table_ddl(
    catalog: Option<&str>,
    schema: &str,
    table: &str,
    columns: &[Column],
) -> String {
    let mut lines: Vec<String> = columns
        .iter()
        .map(|c| {
            let null = if c.nullable { "NULL" } else { "NOT NULL" };
            format!("    {} {} {null}", bracket(&c.name), c.data_type)
        })
        .collect();

    let primary_key: Vec<String> = columns
        .iter()
        .filter(|c| c.is_primary_key)
        .map(|c| bracket(&c.name))
        .collect();
    if !primary_key.is_empty() {
        lines.push(format!("    PRIMARY KEY ({})", primary_key.join(", ")));
    }

    let mut name: Vec<String> = catalog.iter().map(|c| bracket(c)).collect();
    name.push(bracket(schema));
    name.push(bracket(table));

    format!(
        "CREATE TABLE {} (\n{}\n);",
        name.join("."),
        lines.join(",\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, data_type: &str, nullable: bool, pk: bool) -> Column {
        Column {
            name: name.to_string(),
            data_type: data_type.to_string(),
            nullable,
            is_primary_key: pk,
        }
    }

    #[test]
    fn bracket_doubles_closing_brackets() {
        assert_eq!(bracket("orders"), "[orders]");
        assert_eq!(bracket("we]rd"), "[we]]rd]");
        assert_eq!(bracket("a.b"), "[a.b]");
    }

    #[test]
    fn each_name_part_is_quoted_separately() {
        let ddl = build_table_ddl(
            Some("Company.Sales"),
            "dbo",
            "Orders",
            &[
                col("Id", "int", false, true),
                col("Note]s", "nvarchar(50)", true, false),
            ],
        );
        assert_eq!(
            ddl,
            "CREATE TABLE [Company.Sales].[dbo].[Orders] (\n\
             \x20   [Id] int NOT NULL,\n\
             \x20   [Note]]s] nvarchar(50) NULL,\n\
             \x20   PRIMARY KEY ([Id])\n\
             );"
        );
    }

    #[test]
    fn bare_schema_and_no_primary_key() {
        let ddl = build_table_ddl(
            None,
            "dbo",
            "Log",
            &[col("Msg", "varchar(max)", true, false)],
        );
        assert_eq!(
            ddl,
            "CREATE TABLE [dbo].[Log] (\n    [Msg] varchar(max) NULL\n);"
        );
    }

    #[test]
    fn composite_primary_key_keeps_column_order() {
        let ddl = build_table_ddl(
            None,
            "dbo",
            "Lines",
            &[
                col("OrderId", "int", false, true),
                col("Line", "int", false, true),
            ],
        );
        assert!(ddl.ends_with("    PRIMARY KEY ([OrderId], [Line])\n);"));
    }
}
