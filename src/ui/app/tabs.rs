use crate::ui::state::{Focus, Overlay, TabGroup};
use crate::ui::tabs::{SubFocus, TabKind};

use super::{App, load_script_connection};

impl App {
    pub(super) fn handle_close_tab(&mut self) {
        // Context-aware close:
        //   - if focus is on a result sub-pane (the data grid, the error
        //     editor, or the failed-query editor that sits next to the error
        //     editor in the error split view) → close that result tab
        //   - if a query is currently streaming (even from Editor focus) →
        //     cancel it and clear the loading placeholder / partial result tab
        //   - otherwise fall through to closing the workspace tab
        let (on_results, is_streaming) = self
            .state
            .active_tab()
            .map(|t| {
                (
                    matches!(t.sub_focus, SubFocus::Results | SubFocus::QueryView)
                        && !t.result_tabs.is_empty(),
                    t.streaming,
                )
            })
            .unwrap_or((false, false));
        let close_result = on_results || is_streaming;

        if close_result {
            self.abort_active_streaming();
            if let Some(tab) = self.state.active_tab_mut() {
                if !tab.result_tabs.is_empty() {
                    let idx = tab.active_result_idx;
                    tab.result_tabs.remove(idx);
                    if tab.result_tabs.is_empty() {
                        tab.active_result_idx = 0;
                        tab.query_result = None;
                        tab.grid_focused = false;
                        tab.sub_focus = SubFocus::Editor;
                    } else if idx >= tab.result_tabs.len() {
                        tab.active_result_idx = tab.result_tabs.len() - 1;
                    }
                } else {
                    // Pure loading placeholder (streaming with no batches yet).
                    tab.query_result = None;
                    tab.grid_focused = false;
                    tab.sub_focus = SubFocus::Editor;
                }
            }
            self.state.status_message = "Query cancelled".to_string();
            return;
        }

        // Check if active tab has a modified editor (script)
        let is_modified = if let Some(tab) = self.state.active_tab() {
            match &tab.kind {
                TabKind::Script { .. } => tab.editor.as_ref().map(|e| e.modified).unwrap_or(false),
                _ => false, // Non-script tabs close without confirmation
            }
        } else {
            false
        };

        if is_modified {
            self.state.overlay = Some(Overlay::ConfirmClose);
        } else {
            self.abort_active_streaming();
            self.state.close_active_tab();
        }
    }

    /// Create a vertical split. Clones the active tab as an independent instance
    /// (new TabId, separate state) into the new (right) group.
    pub(super) fn handle_create_split(&mut self) {
        if self.state.groups.is_some() {
            return; // Already split — max 2 groups
        }
        if self.state.tabs.is_empty() {
            return;
        }

        let all_ids: Vec<_> = self.state.tabs.iter().map(|t| t.id).collect();
        let active_idx = self.state.active_tab_idx;

        // Clone the active tab as an independent instance with a new TabId
        let new_id = self.state.alloc_tab_id();
        let cloned_tab = self.state.tabs[active_idx].clone_for_split(new_id);
        self.state.tabs.push(cloned_tab);

        let g0 = TabGroup::new(all_ids, active_idx);
        let g1 = TabGroup::new(vec![new_id], 0);

        self.state.groups = Some([g0, g1]);
        self.state.active_group = 1;
        self.state.sync_active_tab_idx();
        self.state.status_message = "Split created".to_string();
    }

    /// Close the focused group: kill the active tab in the focused group and move
    /// all OTHER tabs from the focused group into the surviving group. Then exit
    /// split mode. When there's no split, falls back to closing the active tab.
    pub(super) fn handle_close_group(&mut self) {
        let groups = match self.state.groups.take() {
            Some(g) => g,
            None => {
                // No split — just close the active tab
                self.handle_close_tab();
                return;
            }
        };

        let closed = self.state.active_group;
        let mut surviving = groups[1 - closed].clone();
        let closed_group = &groups[closed];
        let active_id = closed_group.active_tab_id();

        // Move all non-active tabs from the closed group into the surviving group
        // (skip ones already in surviving to avoid duplicates).
        for id in &closed_group.tab_ids {
            if Some(*id) == active_id {
                continue; // skip the active tab — it gets killed
            }
            if !surviving.tab_ids.contains(id) {
                surviving.tab_ids.push(*id);
            }
        }

        // Kill the active tab from state.tabs (only if no other group still references it)
        if let Some(id) = active_id
            && !surviving.tab_ids.contains(&id)
            && let Some(idx) = self.state.tabs.iter().position(|t| t.id == id)
        {
            self.state.tabs.remove(idx);
        }

        // Reorder state.tabs to match surviving group order
        let mut new_tabs = Vec::with_capacity(surviving.tab_ids.len());
        for id in &surviving.tab_ids {
            if let Some(pos) = self.state.tabs.iter().position(|t| t.id == *id) {
                new_tabs.push(self.state.tabs.remove(pos));
            }
        }
        new_tabs.append(&mut self.state.tabs);
        self.state.tabs = new_tabs;
        self.state.active_tab_idx = surviving
            .active_idx
            .min(self.state.tabs.len().saturating_sub(1));
        self.state.active_group = 0;

        if self.state.tabs.is_empty() {
            self.state.focus = Focus::Sidebar;
        }
        self.state.status_message = "Group closed".to_string();
    }

    /// Move the focused group's active tab to the other group.
    pub(super) fn handle_move_tab_to_other(&mut self) {
        let groups = match self.state.groups.as_mut() {
            Some(g) => g,
            None => return,
        };

        let from = self.state.active_group;
        let to = 1 - from;

        // Get tab to move
        let moving_id = match groups[from].active_tab_id() {
            Some(id) => id,
            None => return,
        };

        // Remove from source group
        if let Some(pos) = groups[from].tab_ids.iter().position(|id| *id == moving_id) {
            groups[from].tab_ids.remove(pos);
            if groups[from].active_idx >= groups[from].tab_ids.len()
                && !groups[from].tab_ids.is_empty()
            {
                groups[from].active_idx = groups[from].tab_ids.len() - 1;
            }
        }

        // Add to destination if not already there
        if !groups[to].tab_ids.contains(&moving_id) {
            groups[to].tab_ids.push(moving_id);
            groups[to].active_idx = groups[to].tab_ids.len() - 1;
        }

        // If source group is now empty, destroy split
        if groups[from].tab_ids.is_empty() {
            let surviving = groups[to].clone();
            self.state.groups = None;
            self.state.active_group = 0;
            // Reorder tabs to match surviving group order
            let mut new_tabs = Vec::with_capacity(surviving.tab_ids.len());
            for id in &surviving.tab_ids {
                if let Some(pos) = self.state.tabs.iter().position(|t| t.id == *id) {
                    new_tabs.push(self.state.tabs.remove(pos));
                }
            }
            new_tabs.append(&mut self.state.tabs);
            self.state.tabs = new_tabs;
            self.state.active_tab_idx = surviving
                .active_idx
                .min(self.state.tabs.len().saturating_sub(1));
            self.state.status_message = "Tab moved (split closed)".to_string();
        } else {
            // Switch focus to destination group
            self.state.active_group = to;
            self.state.sync_active_tab_idx();
            self.state.status_message = "Tab moved".to_string();
        }
    }

    pub(super) fn open_script(&mut self, name: &str) {
        if let Ok(store) = crate::core::storage::ScriptStore::new() {
            if let Ok(content) = store.read(&format!("{name}.sql")) {
                // Display name = last segment (without collection prefix)
                let display_name = name.rsplit('/').next().unwrap_or(name).to_string();

                // Load saved connection: try full path first, then base name (backward compat)
                let saved_conn =
                    load_script_connection(name).or_else(|| load_script_connection(&display_name));

                // Auto-connect if saved connection exists but isn't active
                let needs_connect = saved_conn
                    .as_ref()
                    .is_some_and(|cn| !self.adapters.contains_key(cn.as_str()));
                if let Some(ref cn) = saved_conn
                    && needs_connect
                {
                    self.connect_by_name(cn);
                }

                let tab_id = self.state.open_or_focus_tab(TabKind::Script {
                    file_path: Some(format!("{name}.sql")),
                    name: display_name,
                    conn_name: saved_conn,
                });
                if let Some(tab) = self.state.find_tab_mut(tab_id) {
                    if let Some(editor) = tab.editor.as_mut() {
                        editor.set_content(&content);
                    }
                    tab.mark_saved();
                }
                if needs_connect {
                    self.state.status_message = "Loading context...".to_string();
                    self.state.loading = true;
                    self.state.loading_since = Some(std::time::Instant::now());
                } else {
                    self.state.status_message = format!("Opened script '{name}'");
                }
            } else {
                self.state.status_message = format!("Error reading script '{name}'");
            }
        }
    }
}
