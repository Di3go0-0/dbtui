use super::*;

impl App {
    pub(super) fn persist_connections(&self) {
        if let Ok(store) = crate::core::storage::ConnectionStore::new() {
            let _ = store.save(&self.state.dialogs.saved_connections, "");
        }
    }

    /// Persist the current list of group names (so empty groups survive restarts)
    pub(super) fn persist_groups(&self) {
        if let Ok(store) = crate::core::storage::ConnectionStore::new() {
            let groups: Vec<String> = self
                .state
                .sidebar
                .tree
                .iter()
                .filter_map(|n| {
                    if let TreeNode::Group { name, .. } = n {
                        return Some(name.clone());
                    }
                    None
                })
                .collect();
            let _ = store.save_groups(&groups);
        }
    }

    pub(super) fn save_current_connection(&mut self) {
        let config = self.state.dialogs.connection_form.to_connection_config();
        if config.name.is_empty() {
            self.state.dialogs.connection_form.error_message =
                "Name is required to save".to_string();
            return;
        }
        self.save_connection_config(&config);
        self.state.status_message = format!("Connection '{}' saved", config.name);
    }

    pub fn load_saved_connections(&mut self) {
        if let Ok(store) = crate::core::storage::ConnectionStore::new()
            && let Ok(configs) = store.load("")
        {
            self.state.dialogs.saved_connections = configs.clone();

            // Build grouped tree: collect unique groups from persisted + connections
            let mut seen = std::collections::HashSet::new();
            let mut groups_order: Vec<String> = Vec::new();
            // Include persisted groups (preserves order, includes "Default" if it was saved)
            if let Ok(persisted_groups) = store.load_groups() {
                for g in persisted_groups {
                    if seen.insert(g.clone()) {
                        groups_order.push(g);
                    }
                }
            }
            // Include groups from connections
            for config in &configs {
                if seen.insert(config.group.clone()) {
                    groups_order.push(config.group.clone());
                }
            }
            // If no groups at all, add "Default" as fallback
            if groups_order.is_empty() && !configs.is_empty() {
                groups_order.push("Default".to_string());
            }

            for group in &groups_order {
                let group_conns: Vec<_> = configs.iter().filter(|c| &c.group == group).collect();
                if group_conns.is_empty() && !seen.contains(group) {
                    continue;
                }
                self.state.sidebar.tree.push(TreeNode::Group {
                    name: group.clone(),
                    expanded: false,
                });
                for config in group_conns {
                    self.state.sidebar.tree.push(TreeNode::Connection {
                        name: config.name.clone(),
                        expanded: false,
                        status: crate::ui::state::ConnStatus::Disconnected,
                    });
                }
            }
            if !configs.is_empty() {
                self.state.status_message =
                    format!("{} connection(s) loaded - expand to connect", configs.len());
            }
        }
        self.load_object_filter();
        self.refresh_scripts_list();
    }

    pub(super) fn load_object_filter(&mut self) {
        if let Ok(dir) = crate::core::storage::ConnectionStore::new()
            && let Ok(data) = std::fs::read_to_string(dir.dir_path().join("object_filters.json"))
            && let Ok(filters) = serde_json::from_str::<HashMap<String, Vec<String>>>(&data)
        {
            for (key, names) in filters {
                let set: HashSet<String> = names.into_iter().collect();
                if !set.is_empty() {
                    self.state.sidebar.object_filter.filters.insert(key, set);
                }
            }
        }
    }

    /// Load `~/.config/dbtui/keybindings.toml` and merge it on top of the
    /// defaults already in `state.bindings`. Errors are surfaced via the
    /// status bar so the user can fix them — defaults stay in place.
    pub fn load_keybindings(&mut self) {
        let (bindings, error) = crate::keybindings::KeyBindings::load_from_default_path();
        self.state.bindings = bindings;
        if let Some(e) = error {
            self.state.status_message = format!("keybindings.toml {e}");
        }
    }

    pub fn save_object_filter(&mut self) {
        if let Ok(dir) = crate::core::storage::ConnectionStore::new() {
            let filter_path = dir.dir_path().join("object_filters.json");
            let serializable: HashMap<&String, Vec<&String>> = self
                .state
                .sidebar
                .object_filter
                .filters
                .iter()
                .filter(|(_, set)| !set.is_empty())
                .map(|(k, set)| (k, set.iter().collect()))
                .collect();
            match std::fs::write(
                &filter_path,
                serde_json::to_string_pretty(&serializable).unwrap_or_default(),
            ) {
                Ok(()) => {
                    let total: usize = self
                        .state
                        .sidebar
                        .object_filter
                        .filters
                        .values()
                        .map(|s| s.len())
                        .sum();
                    if total > 0 {
                        self.state.status_message = format!("Filters saved ({total} rules)");
                    }
                }
                Err(e) => {
                    self.state.status_message = format!("Error saving filter: {e}");
                }
            }
        }
    }

    pub(super) fn handle_export(&mut self) {
        let dialog = match self.state.dialogs.export_dialog.take() {
            Some(d) => d,
            None => return,
        };

        let options = crate::core::storage::ExportOptions {
            include_credentials: dialog.include_credentials,
            password: dialog.password,
        };

        let resolved = crate::ui::state::expand_user_path(&dialog.path);
        match crate::core::storage::export_bundle(&resolved, &options) {
            Ok(manifest) => {
                let cred = if manifest.includes_credentials {
                    "with credentials"
                } else {
                    "without credentials"
                };
                self.state.status_message = format!(
                    "Exported {} connections, {} scripts ({cred}) → {}",
                    manifest.connection_count, manifest.script_count, dialog.path
                );
            }
            Err(e) => {
                self.state.status_message = format!("Export failed: {e}");
            }
        }
    }

    pub(super) fn handle_import(&mut self) {
        let dialog = match self.state.dialogs.import_dialog.take() {
            Some(d) => d,
            None => return,
        };

        // Expand `~`, then resolve to absolute + canonical so the user sees
        // exactly which path we tried to open when something goes wrong.
        let expanded = crate::ui::state::expand_user_path(dialog.path.trim());
        let resolved = std::fs::canonicalize(&expanded).unwrap_or_else(|_| expanded.clone());
        let exists = resolved.exists();
        let result = match crate::core::storage::import_bundle(&resolved, &dialog.password) {
            Ok(r) => r,
            Err(e) => {
                let shown = resolved.display();
                self.state.status_message = if !exists {
                    format!(
                        "Import failed: file not found at {shown} (typed: {})",
                        dialog.path
                    )
                } else {
                    format!("Import failed: {e} (path: {shown})")
                };
                return;
            }
        };

        let mut conn_added = 0usize;
        let mut script_added = 0usize;

        // Merge connections (skip existing by name)
        let existing_names: std::collections::HashSet<String> = self
            .state
            .dialogs
            .saved_connections
            .iter()
            .map(|c| c.name.clone())
            .collect();
        for conn in result.connections {
            if !existing_names.contains(&conn.name) {
                self.save_connection_config(&conn);
                let insert_idx = self.find_or_create_group_insert_idx(&conn.group);
                self.state.sidebar.tree.insert(
                    insert_idx,
                    TreeNode::Connection {
                        name: conn.name.clone(),
                        expanded: false,
                        status: crate::ui::state::ConnStatus::Disconnected,
                    },
                );
                conn_added += 1;
            }
        }

        // Merge groups
        let existing_groups: std::collections::HashSet<String> = self
            .state
            .sidebar
            .tree
            .iter()
            .filter_map(|n| {
                if let TreeNode::Group { name, .. } = n {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();
        for group in &result.groups {
            if !existing_groups.contains(group) {
                self.state.sidebar.tree.push(TreeNode::Group {
                    name: group.clone(),
                    expanded: false,
                });
            }
        }

        // Merge scripts
        if let Ok(script_store) = crate::core::storage::ScriptStore::new() {
            for (path, content) in &result.scripts {
                let full_path = script_store.scripts_dir().join(path);
                if !full_path.exists() {
                    // Create parent dirs if needed
                    if let Some(parent) = full_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&full_path, content);
                    script_added += 1;
                }
            }
        }

        // Merge object filters
        for (key, values) in &result.object_filters {
            if !self.state.sidebar.object_filter.filters.contains_key(key) {
                self.state
                    .sidebar
                    .object_filter
                    .filters
                    .insert(key.clone(), values.iter().cloned().collect());
            }
        }

        // Merge script connections
        for (script, conn) in &result.script_connections {
            // Only add if not already mapped
            if load_script_connection(script).is_none() {
                save_script_connection(script, conn);
            }
        }

        // Merge bind variables
        let existing_vars = load_bind_variable_values();
        let new_vars: Vec<(String, String)> = result
            .bind_variables
            .iter()
            .filter(|(k, _)| !existing_vars.contains_key(*k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if !new_vars.is_empty() {
            save_bind_variable_values(&new_vars);
        }

        // Refresh UI
        self.persist_connections();
        self.persist_groups();
        self.save_object_filter();
        self.refresh_scripts_list();

        let cred = if result.manifest.includes_credentials {
            "with credentials"
        } else {
            "without credentials"
        };
        self.state.status_message =
            format!("Imported {conn_added} connections, {script_added} scripts ({cred})");
    }

    pub(super) fn refresh_scripts_list(&mut self) {
        use crate::ui::state::ScriptNode;
        if let Ok(store) = crate::core::storage::ScriptStore::new()
            && let Ok(tree) = store.list_tree()
        {
            // Preserve expanded state across refresh by collection path.
            let prev_expanded: std::collections::HashSet<String> = self
                .state
                .scripts
                .tree
                .iter()
                .filter_map(|n| match n {
                    ScriptNode::Collection {
                        name,
                        expanded: true,
                    } => Some(name.clone()),
                    _ => None,
                })
                .collect();

            let mut nodes = Vec::new();

            // Build parent→children map so we can emit folders before
            // files at every level of the hierarchy.
            let mut children_of: std::collections::HashMap<
                &str,
                Vec<&crate::core::storage::ScriptCollection>,
            > = std::collections::HashMap::new();
            let mut root_collections: Vec<&crate::core::storage::ScriptCollection> = Vec::new();
            for coll in &tree.collections {
                if let Some(slash) = coll.name.rfind('/') {
                    children_of
                        .entry(&coll.name[..slash])
                        .or_default()
                        .push(coll);
                } else {
                    root_collections.push(coll);
                }
            }

            // Recursive helper: collection node → child folders → scripts.
            fn emit_collection(
                coll: &crate::core::storage::ScriptCollection,
                children_of: &std::collections::HashMap<
                    &str,
                    Vec<&crate::core::storage::ScriptCollection>,
                >,
                prev_expanded: &std::collections::HashSet<String>,
                nodes: &mut Vec<ScriptNode>,
            ) {
                nodes.push(ScriptNode::Collection {
                    name: coll.name.clone(),
                    expanded: prev_expanded.contains(&coll.name),
                });
                if let Some(children) = children_of.get(coll.name.as_str()) {
                    for child in children {
                        emit_collection(child, children_of, prev_expanded, nodes);
                    }
                }
                for script in &coll.scripts {
                    let base = script.strip_suffix(".sql").unwrap_or(script).to_string();
                    nodes.push(ScriptNode::Script {
                        name: base,
                        collection: Some(coll.name.clone()),
                        file_path: format!("{}/{script}", coll.name),
                    });
                }
            }

            // Root level: folders first, then root scripts.
            for coll in &root_collections {
                emit_collection(coll, &children_of, &prev_expanded, &mut nodes);
            }
            for script in &tree.root_scripts {
                let base = script.strip_suffix(".sql").unwrap_or(script).to_string();
                nodes.push(ScriptNode::Script {
                    name: base,
                    collection: None,
                    file_path: script.clone(),
                });
            }
            self.state.scripts.tree = nodes;
            let visible_count = self.state.scripts.visible_scripts().len();
            if self.state.scripts.cursor >= visible_count && visible_count > 0 {
                self.state.scripts.cursor = visible_count - 1;
            }
        }
    }

    pub(super) fn save_active_script(&mut self) {
        if let Some(tab) = self.state.active_tab()
            && let TabKind::Script {
                ref file_path,
                ref name,
                ..
            } = tab.kind
            && file_path.is_none()
        {
            // New script: prompt for name
            self.state.scripts.save_name = Some(name.clone());
            self.state.overlay = Some(Overlay::SaveScriptName);
            return;
        }
        self.do_save_script(None);
    }

    pub(super) fn do_save_script(&mut self, new_name: Option<&str>) {
        if let Some(tab) = self.state.active_tab_mut()
            && let TabKind::Script {
                ref mut name,
                ref mut file_path,
                ..
            } = tab.kind
        {
            // Derive the save path: use file_path (which includes collection prefix)
            // for existing scripts, or the new_name / display name for new scripts.
            let save_path = if let Some(new) = new_name {
                new.to_string()
            } else if let Some(fp) = file_path.as_ref() {
                // Strip .sql extension — store.save() adds it back
                fp.strip_suffix(".sql").unwrap_or(fp).to_string()
            } else {
                name.clone()
            };
            let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
            if let Ok(store) = crate::core::storage::ScriptStore::new() {
                match store.save(&save_path, &content) {
                    Ok(()) => {
                        if let Some(new) = new_name {
                            *name = new.to_string();
                        }
                        *file_path = Some(format!("{save_path}.sql"));
                        if let Some(editor) = tab.editor.as_mut() {
                            editor.modified = false;
                        }
                        self.state.status_message = format!("Script '{}' saved", name);
                    }
                    Err(e) => {
                        self.state.status_message = format!("Error saving script: {e}");
                    }
                }
            }
        }
        // Snapshot content hash after save so check_modified can detect reverts
        if let Some(tab) = self.state.active_tab_mut() {
            tab.mark_saved();
        }
        self.refresh_scripts_list();
    }
}

/// Load the saved connection name for a script
pub(super) fn load_script_connection(script_name: &str) -> Option<String> {
    let dir = crate::core::storage::ConnectionStore::new().ok()?;
    let path = dir.dir_path().join("script_connections.json");
    let data = std::fs::read_to_string(&path).ok()?;
    let map: std::collections::HashMap<String, String> = serde_json::from_str(&data).ok()?;
    map.get(script_name).cloned()
}

/// Save the connection name for a script
pub(super) fn save_script_connection(script_name: &str, conn_name: &str) {
    let dir = match crate::core::storage::ConnectionStore::new() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = dir.dir_path().join("script_connections.json");

    let mut map: std::collections::HashMap<String, String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default();

    map.insert(script_name.to_string(), conn_name.to_string());

    if let Ok(json) = serde_json::to_string_pretty(&map) {
        let _ = std::fs::write(&path, json);
    }
}

/// Load saved bind variable values (all variables across all scripts)
pub fn load_bind_variable_values() -> std::collections::HashMap<String, String> {
    let dir = match crate::core::storage::ConnectionStore::new() {
        Ok(d) => d,
        Err(_) => return std::collections::HashMap::new(),
    };
    let path = dir.dir_path().join("bind_variables.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

/// Save bind variable values to disk
pub fn save_bind_variable_values(vars: &[(String, String)]) {
    let dir = match crate::core::storage::ConnectionStore::new() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = dir.dir_path().join("bind_variables.json");

    let mut map: std::collections::HashMap<String, String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default();

    for (name, value) in vars {
        if !value.is_empty() {
            map.insert(name.clone(), value.clone());
        }
    }

    if let Ok(json) = serde_json::to_string_pretty(&map) {
        let _ = std::fs::write(&path, json);
    }
}
