//! Choosing which connection's adapter a piece of work runs on.

use super::*;

impl App {
    /// Get the adapter for a connection name
    pub(super) fn adapter_for(&self, conn_name: &str) -> Option<Arc<dyn DatabaseAdapter>> {
        self.adapters.get(conn_name).cloned()
    }

    /// Engine of a connection, read from its live adapter. `state.conn.db_type`
    /// only remembers the connection that connected last, so anything that
    /// generates SQL or builds a tree for a specific connection asks here.
    pub(super) fn db_type_of(&self, conn_name: &str) -> Option<DatabaseType> {
        self.adapters.get(conn_name).map(|a| a.db_type())
    }

    /// Adapter a tab's work must run on: the connection the tab belongs to.
    /// Only a script with no connection assigned follows the sidebar selection
    /// — falling back to it for a bound tab would run the tab's SQL against
    /// whichever connection the sidebar cursor happens to be in.
    pub(super) fn adapter_for_tab(&self, tab_id: TabId) -> Option<Arc<dyn DatabaseAdapter>> {
        match self.state.find_tab(tab_id).and_then(|t| t.kind.conn_name()) {
            Some(conn_name) => self.adapter_for(conn_name),
            None => self.active_adapter().map(|(_, a)| a),
        }
    }

    /// Connection the completion engine is working for: the active tab's own,
    /// or the sidebar selection for a script with none assigned. Metadata
    /// fetched on demand while typing must come from there, and be cached
    /// under that name.
    pub(super) fn completion_adapter(&self) -> Option<(String, Arc<dyn DatabaseAdapter>)> {
        match self.state.active_tab().and_then(|t| t.kind.conn_name()) {
            Some(name) => Some((name.to_string(), self.adapter_for(name)?)),
            None => self.active_adapter(),
        }
    }

    /// `adapter_for_tab`, reporting the failure: when the tab's connection is
    /// not live, its loading indicators are cleared and the status bar says
    /// why, instead of leaving a spinner that never resolves.
    pub(super) fn require_tab_adapter(
        &mut self,
        tab_id: TabId,
    ) -> Option<Arc<dyn DatabaseAdapter>> {
        if let Some(adapter) = self.adapter_for_tab(tab_id) {
            return Some(adapter);
        }
        let conn_name = self
            .state
            .find_tab(tab_id)
            .and_then(|t| t.kind.conn_name().map(str::to_string));
        if let Some(tab) = self.state.find_tab_mut(tab_id) {
            tab.streaming = false;
            tab.streaming_since = None;
            tab.first_batch_pending = false;
            tab.pending_query = None;
        }
        self.finish_loading();
        self.state.status_message = match conn_name {
            Some(name) => format!("Not connected to '{name}' — connect it and try again"),
            None => "No active connection".to_string(),
        };
        None
    }

    /// Get the adapter for the currently active connection (from tree selection)
    pub(super) fn active_adapter(&self) -> Option<(String, Arc<dyn DatabaseAdapter>)> {
        // Walk up from selected node to find its Connection parent
        let selected = self.state.selected_tree_index()?;
        let mut idx = selected;
        loop {
            match &self.state.sidebar.tree[idx] {
                TreeNode::Connection { name, .. } => {
                    let adapter = self.adapters.get(name)?;
                    return Some((name.clone(), Arc::clone(adapter)));
                }
                _ => {
                    if idx == 0 {
                        break;
                    }
                    idx -= 1;
                }
            }
        }
        // Fallback: first adapter
        self.adapters
            .iter()
            .next()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
    }
}
