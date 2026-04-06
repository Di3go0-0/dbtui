use super::*;

impl App {
    pub(super) fn rename_connection(&mut self, old_name: &str, new_name: &str) {
        // Check for name collision
        if self
            .state
            .saved_connections
            .iter()
            .any(|c| c.name == new_name)
        {
            self.state.status_message = format!("Connection '{new_name}' already exists");
            return;
        }

        // Update saved config
        if let Some(config) = self
            .state
            .saved_connections
            .iter_mut()
            .find(|c| c.name == old_name)
        {
            config.name = new_name.to_string();
        }

        // Update tree node
        for node in &mut self.state.tree {
            if let TreeNode::Connection { name, .. } = node
                && *name == old_name
            {
                *name = new_name.to_string();
                break;
            }
        }

        // Update adapter key
        if let Some(adapter) = self.adapters.remove(old_name) {
            self.adapters.insert(new_name.to_string(), adapter);
        }

        // Update active connection name
        if self.state.connection_name.as_deref() == Some(old_name) {
            self.state.connection_name = Some(new_name.to_string());
        }

        // Update tab connection references
        for tab in &mut self.state.tabs {
            match &mut tab.kind {
                TabKind::Table { conn_name, .. }
                | TabKind::Package { conn_name, .. }
                | TabKind::Function { conn_name, .. }
                | TabKind::Procedure { conn_name, .. }
                | TabKind::DbType { conn_name, .. }
                | TabKind::Trigger { conn_name, .. } => {
                    if *conn_name == old_name {
                        *conn_name = new_name.to_string();
                    }
                }
                TabKind::Script { conn_name, .. } => {
                    if conn_name.as_deref() == Some(old_name) {
                        *conn_name = Some(new_name.to_string());
                    }
                }
            }
        }

        // Migrate object filter keys from old connection name to new
        let old_prefix = format!("{old_name}::");
        let new_prefix = format!("{new_name}::");
        let keys_to_migrate: Vec<String> = self
            .state
            .object_filter
            .filters
            .keys()
            .filter(|k| k.starts_with(&old_prefix))
            .cloned()
            .collect();
        for old_key in keys_to_migrate {
            if let Some(value) = self.state.object_filter.filters.remove(&old_key) {
                let new_key = format!("{new_prefix}{}", &old_key[old_prefix.len()..]);
                self.state.object_filter.filters.insert(new_key, value);
            }
        }

        self.persist_connections();
        self.save_object_filter();
        self.state.status_message = format!("Connection renamed: {old_name} → {new_name}");
    }

    pub(super) fn duplicate_connection(&mut self, source_name: &str, target_group: &str) {
        if let Some(config) = self
            .state
            .saved_connections
            .iter()
            .find(|c| c.name == source_name)
            .cloned()
        {
            let mut new_config = config;
            new_config.name = format!("{source_name} (copy)");
            new_config.group = target_group.to_string();
            // Avoid name collisions
            let mut n = 1;
            while self
                .state
                .saved_connections
                .iter()
                .any(|c| c.name == new_config.name)
            {
                n += 1;
                new_config.name = format!("{source_name} (copy {n})");
            }
            self.save_connection_config(&new_config);
            let insert_idx = self.find_or_create_group_insert_idx(&new_config.group);
            self.state.tree.insert(
                insert_idx,
                TreeNode::Connection {
                    name: new_config.name.clone(),
                    expanded: false,
                    status: crate::ui::state::ConnStatus::Disconnected,
                },
            );
            self.state.status_message = format!("Connection duplicated: {}", new_config.name);
        }
    }

    pub(super) fn set_conn_status(
        &mut self,
        conn_name: &str,
        status: crate::ui::state::ConnStatus,
    ) {
        for node in &mut self.state.tree {
            if let TreeNode::Connection {
                name, status: s, ..
            } = node
                && name == conn_name
            {
                *s = status;
                break;
            }
        }
    }

    pub(super) fn connect_by_name(&mut self, name: &str) {
        self.adapters.remove(name);
        self.state.metadata_ready = false;
        self.state.metadata_index.clear();
        self.set_conn_status(name, crate::ui::state::ConnStatus::Connecting);

        let config = self
            .state
            .saved_connections
            .iter()
            .find(|c| c.name == name)
            .cloned();

        if let Some(config) = config {
            let tx = self.msg_tx.clone();
            let conn_name = name.to_string();
            self.state.status_message = format!("Connecting to {conn_name}...");
            self.state.loading = true;
            self.state.loading_since = Some(std::time::Instant::now());

            tokio::spawn(async move {
                match crate::drivers::create_adapter(&config).await {
                    Ok(adapter) => {
                        let adapter: Arc<dyn crate::core::DatabaseAdapter> = adapter.into();
                        let _ = tx
                            .send(AppMessage::Connected {
                                adapter,
                                name: conn_name,
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppMessage::Error(e.to_string())).await;
                    }
                }
            });
        } else {
            self.state.status_message = format!("No saved config for '{name}'");
        }
    }

    pub(super) fn disconnect_by_name(&mut self, name: &str) {
        self.adapters.remove(name);
        self.state.metadata_ready = false;
        self.state.metadata_index.clear();
        self.set_conn_status(name, crate::ui::state::ConnStatus::Disconnected);

        if let Some(conn_idx) = self
            .state
            .tree
            .iter()
            .position(|n| matches!(n, TreeNode::Connection { name: n, .. } if n == name))
        {
            if let TreeNode::Connection { expanded, .. } = &mut self.state.tree[conn_idx] {
                *expanded = false;
            }
            let d = self.state.tree[conn_idx].depth();
            let mut end = conn_idx + 1;
            while end < self.state.tree.len() && self.state.tree[end].depth() > d {
                end += 1;
            }
            self.state.tree.drain(conn_idx + 1..end);
        }

        if self.state.connection_name.as_deref() == Some(name) {
            self.state.connected = false;
            self.state.connection_name = None;
        }

        self.state.status_message = format!("Disconnected from '{name}'");
    }

    pub(super) fn delete_connection(&mut self, name: &str) {
        self.adapters.remove(name);

        if let Some(conn_idx) = self
            .state
            .tree
            .iter()
            .position(|n| matches!(n, TreeNode::Connection { name: n, .. } if n == name))
        {
            let d = self.state.tree[conn_idx].depth();
            let mut end = conn_idx + 1;
            while end < self.state.tree.len() && self.state.tree[end].depth() > d {
                end += 1;
            }
            self.state.tree.drain(conn_idx..end);
        }

        self.state.saved_connections.retain(|c| c.name != name);
        self.persist_connections();
        self.remove_empty_groups();
        self.persist_groups();

        if self.adapters.is_empty() {
            self.state.connected = false;
            self.state.connection_name = None;
            self.state.db_type = None;
        }

        self.state.tree_state.cursor = 0;
        self.state.tree_state.offset = 0;
        self.state.status_message = format!("Connection '{name}' deleted");
    }

    pub(super) fn save_connection_config(&mut self, config: &ConnectionConfig) {
        // Track old group for potential tree move
        let old_group = self
            .state
            .connection_form
            .editing_name
            .as_ref()
            .and_then(|old| {
                self.state
                    .saved_connections
                    .iter()
                    .find(|c| c.name == *old)
                    .map(|c| c.group.clone())
            });

        if let Some(old_name) = self.state.connection_form.editing_name.take() {
            self.state.saved_connections.retain(|c| c.name != old_name);
            if old_name != config.name {
                if let Some(adapter) = self.adapters.remove(&old_name) {
                    self.adapters.insert(config.name.clone(), adapter);
                }
                for node in &mut self.state.tree {
                    if let TreeNode::Connection { name, .. } = node
                        && *name == old_name
                    {
                        *name = config.name.clone();
                    }
                }
                if self.state.connection_name.as_deref() == Some(&old_name) {
                    self.state.connection_name = Some(config.name.clone());
                }
            }

            // If group changed, move the connection node in the tree
            if old_group.as_deref() != Some(&config.group) {
                // Remove connection + its children from old position
                if let Some(conn_idx) = self.state.tree.iter().position(
                    |n| matches!(n, TreeNode::Connection { name, .. } if name == &config.name),
                ) {
                    let d = self.state.tree[conn_idx].depth();
                    let mut end = conn_idx + 1;
                    while end < self.state.tree.len() && self.state.tree[end].depth() > d {
                        end += 1;
                    }
                    let nodes: Vec<_> = self.state.tree.drain(conn_idx..end).collect();
                    // Insert into new group
                    let insert_idx = self.find_or_create_group_insert_idx(&config.group);
                    for (i, node) in nodes.into_iter().enumerate() {
                        self.state.tree.insert(insert_idx + i, node);
                    }
                }
                // Remove old group if now empty
                self.remove_empty_groups();
            }
        }

        self.state
            .saved_connections
            .retain(|c| c.name != config.name);
        self.state.saved_connections.push(config.clone());
        self.persist_connections();
        self.persist_groups();
    }

    /// Remove group nodes that have no children
    pub(super) fn remove_empty_groups(&mut self) {
        let mut i = 0;
        while i < self.state.tree.len() {
            if let TreeNode::Group { .. } = &self.state.tree[i] {
                let next_is_child = i + 1 < self.state.tree.len()
                    && self.state.tree[i + 1].depth() > self.state.tree[i].depth();
                if !next_is_child {
                    self.state.tree.remove(i);
                    continue;
                }
            }
            i += 1;
        }
    }

    /// Find the insert index for a connection within a group.
    /// If the group doesn't exist yet, creates it and returns the index after it.
    pub(super) fn find_or_create_group_insert_idx(&mut self, group_name: &str) -> usize {
        // Find existing group node
        for i in 0..self.state.tree.len() {
            if let TreeNode::Group { name, .. } = &self.state.tree[i]
                && name == group_name
            {
                let d = self.state.tree[i].depth();
                let mut end = i + 1;
                while end < self.state.tree.len() && self.state.tree[end].depth() > d {
                    end += 1;
                }
                return end;
            }
        }
        // Group doesn't exist — create it at the end
        self.state.tree.push(TreeNode::Group {
            name: group_name.to_string(),
            expanded: true,
        });
        self.state.tree.len()
    }
}
