//! Entity-relationship exports: turn the metadata a tab already holds into
//! a Mermaid diagram the user can take elsewhere.

use std::collections::HashMap;

use super::*;
use crate::core::er::mermaid;

impl App {
    /// Write the schema diagram of a table tab into a new script tab.
    ///
    /// Entities come from the loaded foreign keys plus the open table, whose
    /// columns are the only ones known here, so it is the one entity drawn
    /// with attributes.
    pub(super) fn export_table_diagram(&mut self, tab_id: TabId) {
        let Some(tab) = self.state.find_tab(tab_id) else {
            return;
        };
        let TabKind::Table {
            conn_name,
            schema,
            table,
        } = &tab.kind
        else {
            return;
        };
        let Some(fks) = &tab.relations else {
            self.state.status_message = "Relations are still loading".to_string();
            return;
        };
        let mut columns = HashMap::new();
        if !tab.columns.is_empty() {
            columns.insert(table.clone(), tab.columns.clone());
        }
        let text = mermaid::render(schema, std::slice::from_ref(table), &columns, fks);
        let title = format!("ER {schema}");
        self.open_text_script(conn_name.clone(), title, &text);
    }

    /// Open a script tab holding `text`, or focus it if one with that name
    /// is already open.
    pub(super) fn open_text_script(&mut self, conn_name: String, name: String, text: &str) {
        let tab_id = self.state.open_or_focus_tab(TabKind::Script {
            file_path: None,
            name: name.clone(),
            conn_name: Some(conn_name),
            schema: None,
        });
        if let Some(tab) = self.state.find_tab_mut(tab_id)
            && let Some(editor) = tab.editor.as_mut()
        {
            editor.set_content(text);
            editor.mode = vimltui::VimMode::Normal;
        }
        self.state.status_message = format!("Mermaid diagram written to {name}");
    }
}
