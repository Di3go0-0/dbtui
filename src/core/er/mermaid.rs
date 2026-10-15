//! Mermaid `erDiagram` export.
//!
//! Attribute types and entity names follow Mermaid's grammar: a type is a
//! single token, so `character varying(100)` becomes
//! `character_varying(100)`; a name with characters outside `[A-Za-z0-9_-]`
//! is quoted.

use std::collections::{BTreeSet, HashMap};

use super::{Relationship, group_constraints};
use crate::core::models::{Column, ForeignKeyInfo};

/// Render the schema as a Mermaid entity-relationship diagram.
///
/// `tables` fixes the entity order; `columns` holds the attributes of the
/// tables whose columns have been loaded (the rest are declared bare).
/// Tables outside `schema` that are referenced by a foreign key appear
/// qualified as `schema.table`.
pub fn render(
    schema: &str,
    tables: &[String],
    columns: &HashMap<String, Vec<Column>>,
    fks: &[ForeignKeyInfo],
) -> String {
    let rels = group_constraints(fks);
    let mut out = String::from("erDiagram\n");
    out.push_str(&format!("    %% schema {schema}\n"));

    let mut declared: BTreeSet<String> = BTreeSet::new();
    let mut order: Vec<String> = tables.to_vec();
    for rel in &rels {
        if !order.contains(&rel.table) {
            order.push(rel.table.clone());
        }
        let target = entity_name(schema, &rel.referenced_schema, &rel.referenced_table);
        if !order.contains(&target) {
            order.push(target);
        }
    }

    for table in &order {
        if !declared.insert(table.clone()) {
            continue;
        }
        let fk_columns: BTreeSet<&str> = rels
            .iter()
            .filter(|r| r.table == *table)
            .flat_map(|r| r.columns.iter().map(String::as_str))
            .collect();
        match columns.get(table) {
            Some(cols) if !cols.is_empty() => {
                out.push_str(&format!("    {} {{\n", quote(table)));
                for col in cols {
                    let key =
                        key_marker(col.is_primary_key, fk_columns.contains(col.name.as_str()));
                    out.push_str(&format!(
                        "        {} {}{}\n",
                        type_token(&col.data_type),
                        attribute_name(&col.name),
                        key
                    ));
                }
                out.push_str("    }\n");
            }
            _ => out.push_str(&format!("    {}\n", quote(table))),
        }
    }

    for rel in &rels {
        out.push_str(&relationship_line(schema, rel, columns.get(&rel.table)));
    }
    out
}

fn relationship_line(
    schema: &str,
    rel: &Relationship,
    child_columns: Option<&Vec<Column>>,
) -> String {
    // The referencing side holds the foreign key, so it is the "many" end.
    // A nullable key makes the parent optional: `|o`; otherwise `||`.
    let optional_parent = child_columns.is_some_and(|cols| {
        rel.columns
            .iter()
            .all(|name| cols.iter().any(|c| &c.name == name && c.nullable))
    });
    let parent_end = if optional_parent { "|o" } else { "||" };
    let target = entity_name(schema, &rel.referenced_schema, &rel.referenced_table);
    format!(
        "    {} }}o--{} {} : \"{}\"\n",
        quote(&rel.table),
        parent_end,
        quote(&target),
        rel.column_list().replace('"', "'")
    )
}

fn entity_name(schema: &str, ref_schema: &str, ref_table: &str) -> String {
    if ref_schema.is_empty() || ref_schema == schema {
        ref_table.to_string()
    } else {
        format!("{ref_schema}.{ref_table}")
    }
}

fn is_plain(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn quote(name: &str) -> String {
    if is_plain(name) {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "'"))
    }
}

/// Attribute names cannot be quoted in Mermaid; anything unusual is
/// replaced by an underscore.
fn attribute_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn type_token(data_type: &str) -> String {
    let token: String = data_type
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '(' | ')' | '[' | ']' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if token.is_empty()
        || !token
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
    {
        format!("t_{token}")
    } else {
        token
    }
}

fn key_marker(pk: bool, fk: bool) -> &'static str {
    match (pk, fk) {
        (true, true) => " PK, FK",
        (true, false) => " PK",
        (false, true) => " FK",
        (false, false) => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, data_type: &str, nullable: bool, pk: bool) -> Column {
        Column {
            name: name.into(),
            data_type: data_type.into(),
            nullable,
            is_primary_key: pk,
        }
    }

    fn fk(table: &str, col: &str, ref_table: &str) -> ForeignKeyInfo {
        ForeignKeyInfo {
            constraint_name: format!("fk_{table}_{col}"),
            table_name: table.into(),
            column_name: col.into(),
            referenced_schema: "app".into(),
            referenced_table: ref_table.into(),
            referenced_column: "id".into(),
        }
    }

    #[test]
    fn entities_attributes_and_relationships() {
        let tables = vec!["users".to_string(), "orders".to_string()];
        let mut columns = HashMap::new();
        columns.insert(
            "orders".to_string(),
            vec![
                col("id", "integer", false, true),
                col("user_id", "integer", true, false),
                col("note", "character varying(100)", true, false),
            ],
        );
        let out = render(
            "app",
            &tables,
            &columns,
            &[fk("orders", "user_id", "users")],
        );
        let expected = "erDiagram\n\
            \x20   %% schema app\n\
            \x20   users\n\
            \x20   orders {\n\
            \x20       integer id PK\n\
            \x20       integer user_id FK\n\
            \x20       character_varying(100) note\n\
            \x20   }\n\
            \x20   orders }o--|o users : \"user_id\"\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn cross_schema_and_odd_names_are_quoted() {
        let mut other = fk("order items", "owner", "people");
        other.referenced_schema = "hr".into();
        let out = render("app", &[], &HashMap::new(), &[other]);
        assert!(out.contains("    \"order items\"\n"), "{out}");
        assert!(out.contains("    \"hr.people\"\n"), "{out}");
        assert!(
            out.contains("\"order items\" }o--|| \"hr.people\" : \"owner\""),
            "{out}"
        );
    }

    #[test]
    fn type_tokens_never_start_with_a_digit() {
        assert_eq!(type_token("NUMBER(10,2)"), "NUMBER(10_2)");
        assert_eq!(type_token("2d"), "t_2d");
        assert_eq!(type_token(""), "t_");
    }
}
