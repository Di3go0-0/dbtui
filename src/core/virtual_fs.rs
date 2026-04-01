use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

/// Synchronization state between editor content, local cache, and database
#[derive(Debug, Clone)]
pub enum SyncState {
    /// Content matches database (no local changes)
    Clean,
    /// Editor content differs from last local save
    Dirty,
    /// Saved locally but not yet compiled to database
    LocalSaved,
    /// Validation or compilation error
    ValidationError(String),
}

/// What type of database object this file represents
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FileType {
    PackageDeclaration { schema: String, package: String },
    PackageBody { schema: String, package: String },
    Function { schema: String, name: String },
    Procedure { schema: String, name: String },
}

impl FileType {
    /// Generate a cache filename like "HR_PKG_VENTAS_DECLARATION.sql"
    pub fn cache_filename(&self) -> String {
        match self {
            FileType::PackageDeclaration { schema, package } => {
                format!("{schema}_{package}_DECLARATION.sql")
            }
            FileType::PackageBody { schema, package } => {
                format!("{schema}_{package}_BODY.sql")
            }
            FileType::Function { schema, name } => {
                format!("{schema}_FUNCTION_{name}.sql")
            }
            FileType::Procedure { schema, name } => {
                format!("{schema}_PROCEDURE_{name}.sql")
            }
        }
    }

    /// Virtual path used as key: "SCHEMA.OBJECT" or "SCHEMA.OBJECT.DECLARATION"
    pub fn vfs_path(&self) -> String {
        match self {
            FileType::PackageDeclaration { schema, package } => {
                format!("{schema}.{package}.DECLARATION")
            }
            FileType::PackageBody { schema, package } => {
                format!("{schema}.{package}.BODY")
            }
            FileType::Function { schema, name } => {
                format!("{schema}.{name}")
            }
            FileType::Procedure { schema, name } => {
                format!("{schema}.{name}")
            }
        }
    }

    /// Get the paired path for packages (Declaration <-> Body)
    pub fn paired_path(&self) -> Option<String> {
        match self {
            FileType::PackageDeclaration { schema, package } => {
                Some(format!("{schema}.{package}.BODY"))
            }
            FileType::PackageBody { schema, package } => {
                Some(format!("{schema}.{package}.DECLARATION"))
            }
            _ => None,
        }
    }

    /// Schema this file belongs to
    pub fn schema(&self) -> &str {
        match self {
            FileType::PackageDeclaration { schema, .. }
            | FileType::PackageBody { schema, .. }
            | FileType::Function { schema, .. }
            | FileType::Procedure { schema, .. } => schema,
        }
    }
}

/// A virtual file tracking editor state and sync with database
pub struct VirtualFile {
    /// VFS path key (e.g. "HR.PKG_VENTAS.DECLARATION")
    pub path: String,
    /// Current editor content
    pub content: String,
    /// Last content saved locally via Ctrl+S
    pub local_saved: String,
    /// Content as fetched from database
    pub db_content: String,
    /// Current sync state
    pub sync_state: SyncState,
    /// Type of database object
    pub file_type: FileType,
    /// Path to local cache file (~/.dbtui/cache/...)
    pub cache_path: Option<PathBuf>,
    /// Last time this file was accessed (for LRU eviction)
    pub last_accessed: SystemTime,
}

impl VirtualFile {
    pub fn new(file_type: FileType, db_content: String, cache_dir: Option<&PathBuf>) -> Self {
        let path = file_type.vfs_path();
        let cache_path = cache_dir.map(|dir| dir.join(file_type.cache_filename()));
        Self {
            path,
            content: db_content.clone(),
            local_saved: String::new(),
            db_content,
            sync_state: SyncState::Clean,
            file_type,
            cache_path,
            last_accessed: SystemTime::now(),
        }
    }

    /// Update content from editor keystrokes
    pub fn update_content(&mut self, content: String) {
        self.content = content;
        self.last_accessed = SystemTime::now();
        // If content differs from local_saved (or db_content if never saved locally)
        let reference = if self.local_saved.is_empty() {
            &self.db_content
        } else {
            &self.local_saved
        };
        if self.content != *reference {
            self.sync_state = SyncState::Dirty;
        } else if !self.local_saved.is_empty() && self.local_saved != self.db_content {
            self.sync_state = SyncState::LocalSaved;
        } else {
            self.sync_state = SyncState::Clean;
        }
    }

    /// Mark as locally saved (after successful validation)
    pub fn mark_local_saved(&mut self) {
        self.local_saved = self.content.clone();
        self.sync_state = SyncState::LocalSaved;
    }

    /// Mark as compiled to database (after successful DB compilation)
    pub fn mark_compiled(&mut self) {
        self.db_content = self.local_saved.clone();
        self.sync_state = SyncState::Clean;
    }

    /// Mark validation/compilation error
    pub fn mark_error(&mut self, msg: String) {
        self.sync_state = SyncState::ValidationError(msg);
    }

    /// Touch last_accessed timestamp
    pub fn touch(&mut self) {
        self.last_accessed = SystemTime::now();
    }

    /// Check if content has been modified from the database version
    pub fn is_modified(&self) -> bool {
        self.content != self.db_content
    }
}

/// Virtual file system per connection, tracks all open database objects
pub struct VirtualFileSystem {
    pub files: HashMap<String, VirtualFile>,
    pub connection_id: String,
    pub cache_dir: Option<PathBuf>,
    pub max_cache_files: usize,
}

impl VirtualFileSystem {
    pub fn new(connection_id: String, cache_dir: Option<PathBuf>) -> Self {
        Self {
            files: HashMap::new(),
            connection_id,
            cache_dir,
            max_cache_files: 50,
        }
    }

    /// Get or create a virtual file for a database object
    pub fn get_or_create(
        &mut self,
        file_type: FileType,
        db_content: String,
    ) -> &mut VirtualFile {
        let path = file_type.vfs_path();
        self.files
            .entry(path.clone())
            .or_insert_with(|| VirtualFile::new(file_type, db_content, self.cache_dir.as_ref()))
    }

    /// Get a file by its VFS path
    pub fn get(&self, path: &str) -> Option<&VirtualFile> {
        self.files.get(path)
    }

    /// Get a file mutably by its VFS path
    pub fn get_mut(&mut self, path: &str) -> Option<&mut VirtualFile> {
        self.files.get_mut(path)
    }

    /// Get sync state for a VFS path
    pub fn sync_state(&self, path: &str) -> Option<&SyncState> {
        self.files.get(path).map(|f| &f.sync_state)
    }

    /// Remove a file from VFS
    pub fn remove(&mut self, path: &str) -> Option<VirtualFile> {
        self.files.remove(path)
    }

    /// Evict least recently used files if cache exceeds max
    pub fn evict_lru(&mut self) -> Vec<String> {
        let mut evicted = Vec::new();
        while self.files.len() > self.max_cache_files {
            // Find the least recently accessed file
            let oldest = self
                .files
                .iter()
                .min_by_key(|(_, f)| f.last_accessed)
                .map(|(k, _)| k.clone());

            if let Some(key) = oldest {
                evicted.push(key.clone());
                self.files.remove(&key);
            } else {
                break;
            }
        }
        evicted
    }

    /// Build VFS path for a tab kind
    pub fn path_for_package_decl(schema: &str, name: &str) -> String {
        format!("{schema}.{name}.DECLARATION")
    }

    pub fn path_for_package_body(schema: &str, name: &str) -> String {
        format!("{schema}.{name}.BODY")
    }

    pub fn path_for_function(schema: &str, name: &str) -> String {
        format!("{schema}.{name}")
    }

    pub fn path_for_procedure(schema: &str, name: &str) -> String {
        format!("{schema}.{name}")
    }
}
