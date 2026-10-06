//! Turning the pending edits of a table grid into SQL and applying them.

use std::collections::HashMap;

use super::*;
use crate::sql_engine::quoting::{quote_ident, quote_qualified};
use crate::ui::tabs::{CellEdit, RowChange};

/// Cell text that stands for SQL NULL in the grid — what an emptied cell is
/// stored as, and what the drivers render a NULL as.
const NULL_CELL: &str = "NULL";

/// What the statement builder needs to know about the table being edited.
struct GridChangeContext {
    db_type: Option<DatabaseType>,
    schema: String,
    table: String,
    /// Primary key columns as (index into a grid row, column name).
    pk_cols: Vec<(usize, String)>,
    col_names: Vec<String>,
}

/// One generated statement and the grid row it applies.
#[derive(Debug, PartialEq, Eq)]
struct RowStatement {
    row: usize,
    sql: String,
}

impl GridChangeContext {
    fn from_tab(tab: &WorkspaceTab, db_type: Option<DatabaseType>) -> Option<Self> {
        let TabKind::Table { schema, table, .. } = &tab.kind else {
            return None;
        };
        let col_names = tab
            .query_result
            .as_ref()
            .map(|r| r.columns.clone())
            .unwrap_or_default();
        // Locate each key column in the grid by name; the metadata list and
        // the result set are not guaranteed to share an ordering.
        let pk_cols = tab
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_primary_key)
            .map(|(i, c)| {
                let idx = col_names
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(&c.name))
                    .unwrap_or(i);
                (idx, c.name.clone())
            })
            .collect();
        Some(Self {
            db_type,
            schema: schema.clone(),
            table: table.clone(),
            pk_cols,
            col_names,
        })
    }

    fn target(&self) -> String {
        quote_qualified(self.db_type, &self.schema, &self.table)
    }

    fn column(&self, idx: usize) -> String {
        quote_ident(
            self.db_type,
            self.col_names.get(idx).map(String::as_str).unwrap_or(""),
        )
    }

    /// `WHERE` clause identifying one row by its primary key values.
    fn pk_where(&self, row: &[String]) -> String {
        self.pk_cols
            .iter()
            .map(|(idx, name)| {
                let column = quote_ident(self.db_type, name);
                match row.get(*idx).map(String::as_str) {
                    Some(NULL_CELL) | None => format!("{column} IS NULL"),
                    Some(value) => format!("{column} = {}", sql_literal(value)),
                }
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }
}

/// Render a grid cell as a SQL literal.
fn sql_literal(value: &str) -> String {
    if value == NULL_CELL {
        NULL_CELL.to_string()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

/// The row as it was before its pending edits. The grid shows edited values,
/// but the database row is still identified by the old ones — which matters
/// as soon as a key column is among the edits.
fn row_before_edits(row: &[String], edits: &[CellEdit]) -> Vec<String> {
    let mut original = row.to_vec();
    for edit in edits {
        if let Some(cell) = original.get_mut(edit.col) {
            *cell = edit.original.clone();
        }
    }
    original
}

/// Build the statements for every pending change, in row order.
fn build_statements(
    ctx: &GridChangeContext,
    rows: &[Vec<String>],
    changes: &HashMap<usize, RowChange>,
) -> Result<Vec<RowStatement>, String> {
    let mut ordered: Vec<(&usize, &RowChange)> = changes.iter().collect();
    ordered.sort_by_key(|(row, _)| **row);

    let mut statements = Vec::new();
    for (&row, change) in ordered {
        let sql = match change {
            RowChange::Modified { edits } => {
                if ctx.pk_cols.is_empty() {
                    return Err("Cannot UPDATE: table has no primary key".to_string());
                }
                let Some(row_data) = rows.get(row) else {
                    continue;
                };
                let assignments = edits
                    .iter()
                    .map(|e| format!("{} = {}", ctx.column(e.col), sql_literal(&e.value)))
                    .collect::<Vec<_>>()
                    .join(",\n       ");
                format!(
                    "UPDATE {}\n  SET {assignments}\n  WHERE {}",
                    ctx.target(),
                    ctx.pk_where(&row_before_edits(row_data, edits))
                )
            }
            RowChange::New { values } => {
                let columns = (0..ctx.col_names.len())
                    .map(|i| ctx.column(i))
                    .collect::<Vec<_>>()
                    .join(", ");
                let values = values
                    .iter()
                    .map(|v| sql_literal(v))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "INSERT INTO {}\n  ({columns})\n  VALUES ({values})",
                    ctx.target()
                )
            }
            RowChange::Deleted => {
                if ctx.pk_cols.is_empty() {
                    return Err("Cannot DELETE: table has no primary key".to_string());
                }
                let Some(row_data) = rows.get(row) else {
                    continue;
                };
                format!(
                    "DELETE FROM {}\n  WHERE {}",
                    ctx.target(),
                    ctx.pk_where(row_data)
                )
            }
        };
        statements.push(RowStatement { row, sql });
    }
    Ok(statements)
}

/// Forget the changes that were applied, so a retry after a partial failure
/// only sends what is still pending. Rows whose delete went through leave the
/// grid, and the remaining changes are re-keyed to the shifted row numbers.
pub(super) fn drop_applied_changes(tab: &mut WorkspaceTab, applied_rows: &[usize]) {
    let mut deleted: Vec<usize> = Vec::new();
    for row in applied_rows {
        if let Some(RowChange::Deleted) = tab.grid_changes.remove(row) {
            deleted.push(*row);
        }
    }
    if deleted.is_empty() {
        return;
    }
    deleted.sort_unstable();

    if let Some(result) = tab.query_result.as_mut() {
        for row in deleted.iter().rev() {
            if *row < result.rows.len() {
                result.rows.remove(*row);
            }
        }
        tab.grid_selected_row = tab
            .grid_selected_row
            .min(result.rows.len().saturating_sub(1));
    }
    let pending = std::mem::take(&mut tab.grid_changes);
    tab.grid_changes = pending
        .into_iter()
        .map(|(row, change)| (row - deleted.partition_point(|d| *d < row), change))
        .collect();
}

impl App {
    /// Execute pending grid changes: build SQL, spawn async execution.
    pub(super) fn execute_grid_changes(&mut self) {
        let Some(tab) = self.state.tabs.get(self.state.active_tab_idx) else {
            return;
        };
        let tab_id = tab.id;
        // The table's own connection, whatever the sidebar cursor is on.
        let Some(adapter) = self.adapter_for_tab(tab_id) else {
            self.state.status_message = match tab.kind.conn_name() {
                Some(name) => format!("Not connected to '{name}' — changes not saved"),
                None => "No active connection".to_string(),
            };
            return;
        };

        let Some(ctx) = GridChangeContext::from_tab(tab, Some(adapter.db_type())) else {
            self.state.status_message = "No changes to save".to_string();
            return;
        };
        let rows = tab
            .query_result
            .as_ref()
            .map(|r| r.rows.as_slice())
            .unwrap_or(&[]);
        let statements = match build_statements(&ctx, rows, &tab.grid_changes) {
            Ok(statements) if statements.is_empty() => {
                self.state.status_message = "No changes to save".to_string();
                return;
            }
            Ok(statements) => statements,
            Err(message) => {
                self.state.status_message = message;
                return;
            }
        };

        let tx = self.msg_tx.clone();
        let total = statements.len();
        tokio::spawn(async move {
            let mut applied_rows = Vec::new();
            let mut failed_sql = Vec::new();
            let mut errors = Vec::new();
            for statement in statements {
                match adapter.execute(&statement.sql).await {
                    Ok(_) => applied_rows.push(statement.row),
                    Err(e) => {
                        errors.push(e.to_string());
                        failed_sql.push(statement.sql);
                    }
                }
            }
            let message = if errors.is_empty() {
                AppMessage::GridChangesSaved {
                    tab_id,
                    count: applied_rows.len(),
                }
            } else {
                AppMessage::GridChangesError {
                    tab_id,
                    applied_rows,
                    error_text: errors.join("\n\n"),
                    sql_text: failed_sql.join(";\n\n"),
                }
            };
            let _ = tx.send(message).await;
        });

        self.state.status_message = format!("Executing {total} statements...");
        self.state.loading = true;
        self.state.loading_since = Some(std::time::Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(db_type: DatabaseType) -> GridChangeContext {
        GridChangeContext {
            db_type: Some(db_type),
            schema: "public".to_string(),
            table: "users".to_string(),
            pk_cols: vec![(0, "id".to_string())],
            col_names: vec!["id".to_string(), "name".to_string()],
        }
    }

    fn row(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    fn edit(col: usize, original: &str, value: &str) -> CellEdit {
        CellEdit {
            col,
            original: original.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn update_targets_the_row_by_its_key_before_the_edit() {
        // The grid row already shows the edited key (7); the database row is
        // still 5, and that is the one the WHERE has to name.
        let rows = vec![row(&["7", "ana"])];
        let changes = HashMap::from([(
            0,
            RowChange::Modified {
                edits: vec![edit(0, "5", "7"), edit(1, "bob", "ana")],
            },
        )]);
        let statements = build_statements(&ctx(DatabaseType::PostgreSQL), &rows, &changes)
            .expect("table has a key");
        assert_eq!(
            statements[0].sql,
            "UPDATE public.users\n  SET id = '7',\n       name = 'ana'\n  WHERE id = '5'"
        );
    }

    #[test]
    fn emptied_cell_is_written_as_sql_null() {
        let rows = vec![row(&["1", "NULL"])];
        let changes = HashMap::from([(
            0,
            RowChange::Modified {
                edits: vec![edit(1, "bob", "NULL")],
            },
        )]);
        let statements = build_statements(&ctx(DatabaseType::PostgreSQL), &rows, &changes)
            .expect("table has a key");
        assert!(
            statements[0].sql.contains("SET name = NULL"),
            "{statements:?}"
        );
    }

    #[test]
    fn identifiers_are_quoted_for_the_engine() {
        let mut ctx = ctx(DatabaseType::PostgreSQL);
        ctx.table = "UserAccounts".to_string();
        let rows = vec![row(&["1", "it's"])];
        let changes = HashMap::from([(0, RowChange::Deleted)]);
        let statements = build_statements(&ctx, &rows, &changes).expect("table has a key");
        assert_eq!(
            statements[0].sql,
            "DELETE FROM public.\"UserAccounts\"\n  WHERE id = '1'"
        );

        let insert = HashMap::from([(
            0,
            RowChange::New {
                values: row(&["2", "it's"]),
            },
        )]);
        let statements =
            build_statements(&self::ctx(DatabaseType::MySQL), &[], &insert).expect("insert");
        assert_eq!(
            statements[0].sql,
            "INSERT INTO `public`.`users`\n  (`id`, `name`)\n  VALUES ('2', 'it''s')"
        );
    }

    #[test]
    fn changes_without_a_primary_key_are_refused() {
        let mut ctx = ctx(DatabaseType::PostgreSQL);
        ctx.pk_cols.clear();
        let rows = vec![row(&["1", "a"])];
        let changes = HashMap::from([(0, RowChange::Deleted)]);
        assert!(build_statements(&ctx, &rows, &changes).is_err());
    }

    #[test]
    fn statements_come_out_in_row_order() {
        let rows = vec![row(&["1", "a"]), row(&["2", "b"]), row(&["3", "c"])];
        let changes = HashMap::from([(2, RowChange::Deleted), (0, RowChange::Deleted)]);
        let statements = build_statements(&ctx(DatabaseType::PostgreSQL), &rows, &changes)
            .expect("table has a key");
        assert_eq!(
            statements.iter().map(|s| s.row).collect::<Vec<_>>(),
            vec![0, 2]
        );
    }
}
