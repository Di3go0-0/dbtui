//! Entity-relationship model shared by the relations view, the diagram tab
//! and the exporters. Pure data over the adapter's metadata: no UI, no
//! database access.

pub mod mermaid;

use crate::core::models::ForeignKeyInfo;

/// A foreign key constraint with all of its columns gathered together.
///
/// The adapter reports one row per constraint column; this is the
/// constraint-level view the diagram and the exporters work with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    pub constraint_name: String,
    /// Table the constraint is declared on (the "many" side).
    pub table: String,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    /// Table the constraint points at (the "one" side).
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
}

impl Relationship {
    /// `a, b` for a composite key, `a` for a plain one.
    pub fn column_list(&self) -> String {
        self.columns.join(", ")
    }
}

/// Group per-column foreign key rows into one relationship per constraint,
/// keeping the order in which the rows arrived.
pub fn group_constraints(fks: &[ForeignKeyInfo]) -> Vec<Relationship> {
    let mut out: Vec<Relationship> = Vec::new();
    for fk in fks {
        let existing = out
            .iter_mut()
            .find(|r| r.table == fk.table_name && r.constraint_name == fk.constraint_name);
        match existing {
            Some(rel) => {
                rel.columns.push(fk.column_name.clone());
                rel.referenced_columns.push(fk.referenced_column.clone());
            }
            None => out.push(Relationship {
                constraint_name: fk.constraint_name.clone(),
                table: fk.table_name.clone(),
                columns: vec![fk.column_name.clone()],
                referenced_schema: fk.referenced_schema.clone(),
                referenced_table: fk.referenced_table.clone(),
                referenced_columns: vec![fk.referenced_column.clone()],
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fk(
        table: &str,
        constraint: &str,
        col: &str,
        ref_table: &str,
        ref_col: &str,
    ) -> ForeignKeyInfo {
        ForeignKeyInfo {
            constraint_name: constraint.into(),
            table_name: table.into(),
            column_name: col.into(),
            referenced_schema: "app".into(),
            referenced_table: ref_table.into(),
            referenced_column: ref_col.into(),
        }
    }

    #[test]
    fn composite_keys_collapse_into_one_relationship() {
        let rows = vec![
            fk("order_items", "fk_order", "order_id", "orders", "id"),
            fk("order_items", "fk_order", "order_seq", "orders", "seq"),
            fk("order_items", "fk_product", "product_id", "products", "id"),
        ];
        let rels = group_constraints(&rows);
        assert_eq!(rels.len(), 2);
        assert_eq!(rels[0].columns, ["order_id", "order_seq"]);
        assert_eq!(rels[0].referenced_columns, ["id", "seq"]);
        assert_eq!(rels[0].column_list(), "order_id, order_seq");
        assert_eq!(rels[1].table, "order_items");
        assert_eq!(rels[1].referenced_table, "products");
    }

    #[test]
    fn same_constraint_name_on_different_tables_stays_apart() {
        let rows = vec![
            fk("a", "fk_owner", "owner_id", "users", "id"),
            fk("b", "fk_owner", "owner_id", "users", "id"),
        ];
        assert_eq!(group_constraints(&rows).len(), 2);
    }
}
