use super::*;

impl App {
    // ─── VFS Methods ───

    /// Get or create VFS for a connection
    fn vfs_for(&mut self, conn_name: &str) -> &mut VirtualFileSystem {
        let cache_dir = self.cache_dir.as_ref().map(|d| d.join(conn_name));
        self.vfs
            .entry(conn_name.to_string())
            .or_insert_with(|| VirtualFileSystem::new(conn_name.to_string(), cache_dir))
    }

    /// Register content in VFS when package/source content is loaded from DB
    pub(super) fn register_in_vfs(&mut self, tab_id: TabId, conn_name: &str) {
        let tab = match self.state.find_tab(tab_id) {
            Some(t) => t,
            None => return,
        };

        match &tab.kind {
            TabKind::Package { schema, name, .. } => {
                let decl = tab
                    .decl_editor
                    .as_ref()
                    .map(|e| e.content())
                    .unwrap_or_default();
                let body = tab
                    .body_editor
                    .as_ref()
                    .map(|e| e.content())
                    .unwrap_or_default();
                let schema = schema.clone();
                let name = name.clone();
                let conn = conn_name.to_string();

                let vfs = self.vfs_for(&conn);
                vfs.get_or_create(
                    FileType::PackageDeclaration {
                        schema: schema.clone(),
                        package: name.clone(),
                    },
                    decl,
                );
                vfs.get_or_create(
                    FileType::PackageBody {
                        schema,
                        package: name,
                    },
                    body,
                );
            }
            TabKind::Function { schema, name, .. } => {
                let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
                let schema = schema.clone();
                let name = name.clone();
                let conn = conn_name.to_string();

                let vfs = self.vfs_for(&conn);
                vfs.get_or_create(FileType::Function { schema, name }, content);
            }
            TabKind::Procedure { schema, name, .. } => {
                let content = tab.editor.as_ref().map(|e| e.content()).unwrap_or_default();
                let schema = schema.clone();
                let name = name.clone();
                let conn = conn_name.to_string();

                let vfs = self.vfs_for(&conn);
                vfs.get_or_create(FileType::Procedure { schema, name }, content);
            }
            _ => {}
        }
    }

    /// Get VFS path for a tab
    fn vfs_path_for_tab(&self, tab_id: TabId) -> Option<(String, String)> {
        let tab = self.state.find_tab(tab_id)?;
        match &tab.kind {
            TabKind::Package {
                conn_name,
                schema,
                name,
            } => {
                let sub = tab.active_sub_view.as_ref();
                let path = match sub {
                    Some(SubView::PackageBody) => {
                        VirtualFileSystem::path_for_package_body(schema, name)
                    }
                    _ => VirtualFileSystem::path_for_package_decl(schema, name),
                };
                Some((conn_name.clone(), path))
            }
            TabKind::Function {
                conn_name,
                schema,
                name,
            } => Some((
                conn_name.clone(),
                VirtualFileSystem::path_for_function(schema, name),
            )),
            TabKind::Procedure {
                conn_name,
                schema,
                name,
            } => Some((
                conn_name.clone(),
                VirtualFileSystem::path_for_procedure(schema, name),
            )),
            _ => None,
        }
    }

    /// Sync tab content to VFS and mark as locally saved
    pub(super) fn sync_tab_to_vfs(&mut self, tab_id: TabId, mark_saved: bool) {
        let (conn_name, vfs_path) = match self.vfs_path_for_tab(tab_id) {
            Some(p) => p,
            None => return,
        };

        // Get current content from editor
        let content = if let Some(tab) = self.state.find_tab(tab_id) {
            tab.active_editor().map(|e| e.content()).unwrap_or_default()
        } else {
            return;
        };

        let vfs = self.vfs_for(&conn_name);
        if let Some(file) = vfs.get_mut(&vfs_path) {
            file.update_content(content);
            if mark_saved {
                file.mark_local_saved();
                // Save to disk cache
                if let Some(ref cache_path) = file.cache_path {
                    if let Some(parent) = cache_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(cache_path, &file.local_saved);
                }
            }
        }

        // For packages, also sync the paired file (Declaration <-> Body)
        if let Some(tab) = self.state.find_tab(tab_id)
            && let TabKind::Package { schema, name, .. } = &tab.kind
        {
            let other_path = match tab.active_sub_view.as_ref() {
                Some(SubView::PackageBody) => {
                    VirtualFileSystem::path_for_package_decl(schema, name)
                }
                _ => VirtualFileSystem::path_for_package_body(schema, name),
            };

            let other_content = match tab.active_sub_view.as_ref() {
                Some(SubView::PackageBody) => tab
                    .decl_editor
                    .as_ref()
                    .map(|e| e.content())
                    .unwrap_or_default(),
                _ => tab
                    .body_editor
                    .as_ref()
                    .map(|e| e.content())
                    .unwrap_or_default(),
            };

            let conn_name = conn_name.clone();
            let vfs = self.vfs_for(&conn_name);
            if let Some(file) = vfs.get_mut(&other_path) {
                file.update_content(other_content);
                if mark_saved {
                    file.mark_local_saved();
                    if let Some(ref cache_path) = file.cache_path {
                        if let Some(parent) = cache_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::write(cache_path, &file.local_saved);
                    }
                }
            }
        }
        self.update_tab_sync_state(tab_id);
    }

    /// Mark VFS file as having a validation/compilation error
    pub(super) fn sync_tab_to_vfs_error(&mut self, tab_id: TabId, error: String) {
        let (conn_name, vfs_path) = match self.vfs_path_for_tab(tab_id) {
            Some(p) => p,
            None => return,
        };
        let vfs = self.vfs_for(&conn_name);
        if let Some(file) = vfs.get_mut(&vfs_path) {
            file.mark_error(error);
        }
        self.update_tab_sync_state(tab_id);
    }

    /// Mark VFS file as successfully compiled to DB
    pub(super) fn sync_tab_to_vfs_compiled(&mut self, tab_id: TabId) {
        let (conn_name, vfs_path) = match self.vfs_path_for_tab(tab_id) {
            Some(p) => p,
            None => return,
        };
        let vfs = self.vfs_for(&conn_name);
        if let Some(file) = vfs.get_mut(&vfs_path) {
            file.mark_compiled();
        }
        self.update_tab_sync_state(tab_id);
    }

    /// Copy VFS sync state to the tab for rendering
    fn update_tab_sync_state(&mut self, tab_id: TabId) {
        let sync = self.vfs_sync_state(tab_id).cloned();
        if let Some(tab) = self.state.find_tab_mut(tab_id) {
            tab.sync_state = sync;
        }
    }

    /// Get VFS sync state for a tab
    pub fn vfs_sync_state(&self, tab_id: TabId) -> Option<&SyncState> {
        let tab = self.state.find_tab(tab_id)?;
        let (conn_name, vfs_path) = match &tab.kind {
            TabKind::Package {
                conn_name,
                schema,
                name,
            } => {
                let path = match tab.active_sub_view.as_ref() {
                    Some(SubView::PackageBody) => {
                        VirtualFileSystem::path_for_package_body(schema, name)
                    }
                    _ => VirtualFileSystem::path_for_package_decl(schema, name),
                };
                (conn_name, path)
            }
            TabKind::Function {
                conn_name,
                schema,
                name,
            } => (
                conn_name,
                VirtualFileSystem::path_for_function(schema, name),
            ),
            TabKind::Procedure {
                conn_name,
                schema,
                name,
            } => (
                conn_name,
                VirtualFileSystem::path_for_procedure(schema, name),
            ),
            _ => return None,
        };
        let vfs = self.vfs.get(conn_name.as_str())?;
        vfs.sync_state(&vfs_path)
    }

    /// Handle Ctrl+S: thorough validation + local save
    pub(super) fn handle_validate_and_save(&mut self, tab_id: TabId) {
        let tab = match self.state.find_tab(tab_id) {
            Some(t) => t,
            None => return,
        };

        let (conn_name, schema, content, obj_type) = match extract_source_info(tab) {
            Some(info) => info,
            None => return,
        };

        let adapter = match self.adapter_for(&conn_name) {
            Some(a) => a,
            None => {
                self.state.status_message = "Not connected".to_string();
                return;
            }
        };

        let db_type = adapter.db_type();
        let tx = self.msg_tx.clone();
        self.state.status_message = format!("Validating {obj_type}...");
        self.state.loading = true;
        self.state.loading_since = Some(std::time::Instant::now());

        tokio::spawn(async move {
            let validator = SqlValidator::new(db_type);
            let report = validator
                .validate_thorough(&schema, &content, &adapter)
                .await;
            let _ = tx
                .send(AppMessage::ValidationResult { tab_id, report })
                .await;
        });
    }

    pub(super) fn save_theme_preference(&self, name: &str) {
        if let Ok(dir) = crate::core::storage::ConnectionStore::new() {
            let path = dir.dir_path().join("theme.txt");
            let _ = std::fs::write(path, name);
        }
    }

    pub fn load_theme_preference(&mut self) {
        if let Ok(dir) = crate::core::storage::ConnectionStore::new() {
            let path = dir.dir_path().join("theme.txt");
            if let Ok(name) = std::fs::read_to_string(path) {
                let name = name.trim();
                if !name.is_empty() {
                    self.theme = crate::ui::theme::Theme::by_name(name);
                }
            }
        }
    }
}
