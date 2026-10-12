//! Entity-relationship diagrams: opening a diagram tab, loading what it
//! shows, and exporting Mermaid from a diagram or a table's relations.

use super::*;
use crate::core::er::mermaid;
use crate::ui::diagram::DiagramState;

impl App {
    /// Open (or focus) the diagram of `schema`, centred on `focus` when
    /// given, and load it if it is not loaded yet.
    pub(super) fn open_diagram(
        &mut self,
        conn_name: String,
        schema: String,
        focus: Option<String>,
    ) {
        let tab_id = self.state.open_or_focus_tab(TabKind::Diagram {
            conn_name,
            schema: schema.clone(),
        });
        let mut needs_load = false;
        let mut jumped = false;
        if let Some(tab) = self.state.find_tab_mut(tab_id) {
            let diagram = tab
                .diagram
                .get_or_insert_with(|| Box::new(DiagramState::new(schema.clone(), focus.clone())));
            if diagram.loaded {
                if let Some(table) = &focus {
                    jumped = diagram.jump_to(table) || jumped;
                    diagram.scroll_request = Some(crate::ui::diagram::ScrollRequest::Center);
                }
            } else {
                // Not loaded yet (a tab created by open_or_focus_tab has no
                // focus): the table asked for becomes the opening view.
                if focus.is_some() {
                    diagram.focus = focus.clone();
                    diagram.selected = focus.clone();
                }
                if diagram.loading_since.is_none() {
                    needs_load = true;
                }
            }
        }
        if needs_load {
            self.load_diagram(tab_id, &schema);
        } else if jumped {
            self.request_diagram_columns(tab_id);
        }
    }

    /// Fetch the tables and foreign keys of the schema for a diagram tab.
    pub(super) fn load_diagram(&mut self, tab_id: TabId, schema: &str) {
        let Some(adapter) = self.require_tab_adapter(tab_id) else {
            return;
        };
        if let Some(diagram) = self
            .state
            .find_tab_mut(tab_id)
            .and_then(|t| t.diagram.as_mut())
        {
            diagram.reset();
            diagram.loading_since = Some(std::time::Instant::now());
        }
        self.state.status_message = format!("Loading diagram of {schema}...");
        let tx = self.msg_tx.clone();
        let schema = schema.to_string();
        tokio::spawn(async move {
            let loaded = async {
                let tables = adapter.get_tables(&schema).await?;
                let fks = adapter.get_schema_foreign_keys(&schema).await?;
                Ok::<_, crate::core::error::DbError>((tables, fks))
            }
            .await;
            let message = match loaded {
                Ok((tables, fks)) => AppMessage::DiagramLoaded {
                    tab_id,
                    tables: tables.into_iter().map(|t| t.name).collect(),
                    fks,
                },
                Err(e) => AppMessage::DiagramFailed {
                    tab_id,
                    error: e.to_string(),
                },
            };
            let _ = tx.send(message).await;
        });
    }

    pub(super) fn handle_diagram_loaded(
        &mut self,
        tab_id: TabId,
        tables: Vec<String>,
        fks: Vec<ForeignKeyInfo>,
    ) {
        if let Some(diagram) = self
            .state
            .find_tab_mut(tab_id)
            .and_then(|t| t.diagram.as_mut())
        {
            diagram.set_data(tables, fks);
            self.state.status_message = format!("Diagram: {}", diagram.summary());
        }
        self.request_diagram_columns(tab_id);
    }

    pub(super) fn handle_diagram_failed(&mut self, tab_id: TabId, error: String) {
        if let Some(diagram) = self
            .state
            .find_tab_mut(tab_id)
            .and_then(|t| t.diagram.as_mut())
        {
            diagram.loading_since = None;
            diagram.error = Some(error.clone());
        }
        self.state.status_message = error;
    }

    pub(super) fn handle_diagram_columns_loaded(
        &mut self,
        tab_id: TabId,
        table: String,
        columns: Vec<Column>,
    ) {
        if let Some(diagram) = self
            .state
            .find_tab_mut(tab_id)
            .and_then(|t| t.diagram.as_mut())
        {
            diagram.set_columns(table, columns);
        }
    }

    /// Fetch the columns of every table the diagram shows and has no
    /// columns for yet, one task walking them in order.
    pub(super) fn request_diagram_columns(&mut self, tab_id: TabId) {
        let needed = match self.state.find_tab(tab_id).and_then(|t| t.diagram.as_ref()) {
            Some(diagram) => diagram.tables_needing_columns(),
            None => return,
        };
        if needed.is_empty() {
            return;
        }
        let Some(adapter) = self.require_tab_adapter(tab_id) else {
            return;
        };
        let Some(schema) = self
            .state
            .find_tab(tab_id)
            .and_then(|t| t.diagram_context())
            .map(|(_, schema)| schema.to_string())
        else {
            return;
        };
        // Marked only now: a request that could not be sent (no adapter)
        // must be retried the next time the view changes.
        if let Some(diagram) = self
            .state
            .find_tab_mut(tab_id)
            .and_then(|t| t.diagram.as_mut())
        {
            diagram.pending_columns.extend(needed.iter().cloned());
        }
        let tx = self.msg_tx.clone();
        tokio::spawn(async move {
            for table in needed {
                // A table whose columns cannot be read is drawn without
                // them; the error is not worth interrupting the diagram.
                let columns = adapter
                    .get_columns(&schema, &table)
                    .await
                    .unwrap_or_default();
                if tx
                    .send(AppMessage::DiagramColumnsLoaded {
                        tab_id,
                        table,
                        columns,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
            }
        });
    }

    /// Write the whole schema of the tab's diagram as Mermaid into a new
    /// script tab, with the columns loaded so far.
    pub(super) fn export_diagram(&mut self, tab_id: TabId) {
        let Some(tab) = self.state.find_tab(tab_id) else {
            return;
        };
        let Some((conn_name, schema)) = tab
            .diagram_context()
            .map(|(c, s)| (c.to_string(), s.to_string()))
        else {
            return;
        };
        let Some(diagram) = tab.diagram.as_ref().filter(|d| d.loaded) else {
            self.state.status_message = "The diagram is still loading".to_string();
            return;
        };
        let text = mermaid::render(&schema, &diagram.tables, &diagram.columns, &diagram.fks);
        self.open_text_script(conn_name, format!("ER {schema}"), &text);
    }

    /// Open a script tab holding `text`, or focus it if one with that name
    /// is already open.
    fn open_text_script(&mut self, conn_name: String, name: String, text: &str) {
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
