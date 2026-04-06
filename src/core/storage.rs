use std::fs;
use std::path::{Path, PathBuf};

use argon2::Argon2;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use directories::ProjectDirs;
use rand::RngCore;

use crate::core::error::AppError;
use crate::core::models::ConnectionConfig;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

fn data_dir() -> Result<PathBuf, AppError> {
    ProjectDirs::from("", "", "dbtui")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .ok_or_else(|| AppError::Storage("Cannot determine data directory".into()))
}

fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; KEY_LEN], AppError> {
    let mut key = [0u8; KEY_LEN];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| AppError::Storage(format!("Key derivation failed: {e}")))?;
    Ok(key)
}

#[allow(dead_code)]
fn encrypt(data: &[u8], password: &str) -> Result<Vec<u8>, AppError> {
    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let key = derive_key(password, &salt)?;
    let cipher = ChaCha20Poly1305::new_from_slice(&key)
        .map_err(|e| AppError::Storage(format!("Cipher init failed: {e}")))?;

    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| AppError::Storage(format!("Encryption failed: {e}")))?;

    // Format: salt (16) + nonce (12) + ciphertext
    let mut output = Vec::with_capacity(SALT_LEN + NONCE_LEN + ciphertext.len());
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

fn decrypt(data: &[u8], password: &str) -> Result<Vec<u8>, AppError> {
    if data.len() < SALT_LEN + NONCE_LEN {
        return Err(AppError::Storage("Encrypted data too short".into()));
    }

    let salt = &data[..SALT_LEN];
    let nonce_bytes = &data[SALT_LEN..SALT_LEN + NONCE_LEN];
    let ciphertext = &data[SALT_LEN + NONCE_LEN..];

    let nonce = Nonce::from_slice(nonce_bytes);
    let key = derive_key(password, salt)?;
    let cipher = ChaCha20Poly1305::new_from_slice(&key)
        .map_err(|e| AppError::Storage(format!("Cipher init failed: {e}")))?;

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| AppError::Storage("Decryption failed (wrong password?)".into()))
}

pub struct ConnectionStore {
    dir: PathBuf,
}

impl ConnectionStore {
    pub fn new() -> Result<Self, AppError> {
        let dir = data_dir()?;
        fs::create_dir_all(&dir)
            .map_err(|e| AppError::Storage(format!("Cannot create data dir: {e}")))?;
        Ok(Self { dir })
    }

    pub fn dir_path(&self) -> &Path {
        &self.dir
    }

    fn connections_path(&self) -> PathBuf {
        self.dir.join("connections.json")
    }

    fn encrypted_path(&self) -> PathBuf {
        self.dir.join("connections.enc")
    }

    pub fn load(&self, master_password: &str) -> Result<Vec<ConnectionConfig>, AppError> {
        // Try plain JSON first (simpler, always works)
        let json_path = self.connections_path();
        if json_path.exists() {
            let data = fs::read_to_string(&json_path)
                .map_err(|e| AppError::Storage(format!("Cannot read connections: {e}")))?;
            let configs: Vec<ConnectionConfig> = serde_json::from_str(&data)
                .map_err(|e| AppError::Storage(format!("Invalid connection data: {e}")))?;
            return Ok(configs);
        }

        // Try encrypted file (legacy/future)
        let enc_path = self.encrypted_path();
        if enc_path.exists() {
            let data = fs::read(&enc_path)
                .map_err(|e| AppError::Storage(format!("Cannot read connections: {e}")))?;
            let decrypted = decrypt(&data, master_password)?;
            let configs: Vec<ConnectionConfig> = serde_json::from_slice(&decrypted)
                .map_err(|e| AppError::Storage(format!("Invalid connection data: {e}")))?;
            return Ok(configs);
        }

        Ok(vec![])
    }

    pub fn save(
        &self,
        configs: &[ConnectionConfig],
        _master_password: &str,
    ) -> Result<(), AppError> {
        // Save as plain JSON (readable, debuggable)
        let json = serde_json::to_string_pretty(configs)
            .map_err(|e| AppError::Storage(format!("Serialization failed: {e}")))?;
        fs::write(self.connections_path(), json)
            .map_err(|e| AppError::Storage(format!("Cannot write connections: {e}")))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn save_encrypted(
        &self,
        configs: &[ConnectionConfig],
        master_password: &str,
    ) -> Result<(), AppError> {
        let json = serde_json::to_vec_pretty(configs)
            .map_err(|e| AppError::Storage(format!("Serialization failed: {e}")))?;
        let encrypted = encrypt(&json, master_password)?;
        fs::write(self.encrypted_path(), encrypted)
            .map_err(|e| AppError::Storage(format!("Cannot write connections: {e}")))?;
        Ok(())
    }

    fn groups_path(&self) -> PathBuf {
        self.dir.join("groups.json")
    }

    /// Load persisted group names (includes empty groups that have no connections)
    pub fn load_groups(&self) -> Result<Vec<String>, AppError> {
        let path = self.groups_path();
        if !path.exists() {
            return Ok(vec![]);
        }
        let data = fs::read_to_string(&path)
            .map_err(|e| AppError::Storage(format!("Cannot read groups: {e}")))?;
        let groups: Vec<String> = serde_json::from_str(&data)
            .map_err(|e| AppError::Storage(format!("Invalid groups data: {e}")))?;
        Ok(groups)
    }

    /// Persist group names (only saves non-"Default" groups that aren't implied by connections)
    pub fn save_groups(&self, groups: &[String]) -> Result<(), AppError> {
        let json = serde_json::to_string_pretty(groups)
            .map_err(|e| AppError::Storage(format!("Serialization failed: {e}")))?;
        fs::write(self.groups_path(), json)
            .map_err(|e| AppError::Storage(format!("Cannot write groups: {e}")))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn add(&self, config: ConnectionConfig, master_password: &str) -> Result<(), AppError> {
        let mut configs = self.load(master_password)?;
        // Replace if same name exists
        configs.retain(|c| c.name != config.name);
        configs.push(config);
        self.save(&configs, master_password)
    }

    #[allow(dead_code)]
    pub fn delete(&self, name: &str, master_password: &str) -> Result<(), AppError> {
        let mut configs = self.load(master_password)?;
        configs.retain(|c| c.name != name);
        self.save(&configs, master_password)
    }
}

pub struct ScriptCollection {
    pub name: String,
    pub scripts: Vec<String>,
}

pub struct ScriptTree {
    pub root_scripts: Vec<String>,
    pub collections: Vec<ScriptCollection>,
}

pub struct ScriptStore {
    dir: PathBuf,
}

impl ScriptStore {
    pub fn new() -> Result<Self, AppError> {
        let dir = data_dir()?.join("scripts");
        fs::create_dir_all(&dir)
            .map_err(|e| AppError::Storage(format!("Cannot create scripts dir: {e}")))?;
        Ok(Self { dir })
    }

    pub fn scripts_dir(&self) -> &Path {
        &self.dir
    }

    #[allow(dead_code)]
    pub fn list(&self) -> Result<Vec<String>, AppError> {
        let mut scripts = Vec::new();
        let entries = fs::read_dir(&self.dir)
            .map_err(|e| AppError::Storage(format!("Cannot read scripts dir: {e}")))?;

        for entry in entries {
            let entry = entry.map_err(|e| AppError::Storage(format!("Cannot read entry: {e}")))?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "sql")
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                scripts.push(name.to_string());
            }
        }
        scripts.sort();
        Ok(scripts)
    }

    pub fn list_tree(&self) -> Result<ScriptTree, AppError> {
        let mut root_scripts = Vec::new();
        let mut collections = Vec::new();

        let entries = fs::read_dir(&self.dir)
            .map_err(|e| AppError::Storage(format!("Cannot read scripts dir: {e}")))?;

        for entry in entries {
            let entry = entry.map_err(|e| AppError::Storage(format!("Cannot read entry: {e}")))?;
            let path = entry.path();

            if path.is_dir() {
                if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                    let mut scripts = Vec::new();
                    let sub_entries = fs::read_dir(&path).map_err(|e| {
                        AppError::Storage(format!("Cannot read collection '{dir_name}': {e}"))
                    })?;
                    for sub_entry in sub_entries {
                        let sub_entry = sub_entry
                            .map_err(|e| AppError::Storage(format!("Cannot read entry: {e}")))?;
                        let sub_path = sub_entry.path();
                        if sub_path.extension().is_some_and(|ext| ext == "sql")
                            && let Some(name) = sub_path.file_name().and_then(|n| n.to_str())
                        {
                            scripts.push(name.to_string());
                        }
                    }
                    scripts.sort();
                    collections.push(ScriptCollection {
                        name: dir_name.to_string(),
                        scripts,
                    });
                }
            } else if path.extension().is_some_and(|ext| ext == "sql")
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                root_scripts.push(name.to_string());
            }
        }

        root_scripts.sort();
        collections.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(ScriptTree {
            root_scripts,
            collections,
        })
    }

    pub fn read(&self, name: &str) -> Result<String, AppError> {
        let path = self.dir.join(name);
        fs::read_to_string(&path)
            .map_err(|e| AppError::Storage(format!("Cannot read script '{name}': {e}")))
    }

    pub fn save(&self, name: &str, content: &str) -> Result<(), AppError> {
        let name = if name.ends_with(".sql") {
            name.to_string()
        } else {
            format!("{name}.sql")
        };
        let path = self.dir.join(&name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                AppError::Storage(format!("Cannot create directory for '{name}': {e}"))
            })?;
        }
        fs::write(&path, content)
            .map_err(|e| AppError::Storage(format!("Cannot write script '{name}': {e}")))
    }

    pub fn delete(&self, name: &str) -> Result<(), AppError> {
        let path = self.dir.join(name);
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| AppError::Storage(format!("Cannot delete script '{name}': {e}")))?;
        }
        Ok(())
    }

    pub fn create_collection(&self, name: &str) -> Result<(), AppError> {
        let path = self.dir.join(name);
        fs::create_dir(&path)
            .map_err(|e| AppError::Storage(format!("Cannot create collection '{name}': {e}")))
    }

    pub fn rename_collection(&self, old_name: &str, new_name: &str) -> Result<(), AppError> {
        let old_path = self.dir.join(old_name);
        let new_path = self.dir.join(new_name);
        fs::rename(&old_path, &new_path).map_err(|e| {
            AppError::Storage(format!(
                "Cannot rename collection '{old_name}' to '{new_name}': {e}"
            ))
        })
    }

    pub fn delete_collection(&self, name: &str) -> Result<(), AppError> {
        let path = self.dir.join(name);
        fs::remove_dir(&path)
            .map_err(|e| AppError::Storage(format!("Cannot delete collection '{name}': {e}")))
    }

    pub fn move_script(&self, from: &str, to: &str) -> Result<(), AppError> {
        let from_path = self.dir.join(from);
        let to_path = self.dir.join(to);
        if let Some(parent) = to_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AppError::Storage(format!("Cannot create directory: {e}")))?;
        }
        fs::rename(&from_path, &to_path)
            .map_err(|e| AppError::Storage(format!("Cannot move script: {e}")))
    }

    #[allow(dead_code)]
    pub fn copy_script(&self, from: &str, to: &str) -> Result<(), AppError> {
        let from_path = self.dir.join(from);
        let to_path = self.dir.join(to);
        if let Some(parent) = to_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AppError::Storage(format!("Cannot create directory: {e}")))?;
        }
        fs::copy(&from_path, &to_path)
            .map_err(|e| AppError::Storage(format!("Cannot copy script: {e}")))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn list_collections(&self) -> Result<Vec<String>, AppError> {
        let mut collections = Vec::new();
        let entries = fs::read_dir(&self.dir)
            .map_err(|e| AppError::Storage(format!("Cannot read scripts dir: {e}")))?;
        for entry in entries {
            let entry = entry.map_err(|e| AppError::Storage(format!("Cannot read entry: {e}")))?;
            let path = entry.path();
            if path.is_dir()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                collections.push(name.to_string());
            }
        }
        collections.sort();
        Ok(collections)
    }
}

// --- Cache Store (VFS local persistence) ---

#[allow(dead_code)]
pub struct CacheStore {
    dir: PathBuf,
}

#[allow(dead_code)]
impl CacheStore {
    pub fn new(connection_id: &str) -> Result<Self, AppError> {
        let dir = data_dir()?.join("cache").join(connection_id);
        fs::create_dir_all(&dir)
            .map_err(|e| AppError::Storage(format!("Cannot create cache dir: {e}")))?;
        Ok(Self { dir })
    }

    /// Get the base cache directory path (without connection ID)
    pub fn base_cache_dir() -> Option<PathBuf> {
        data_dir().ok().map(|d| d.join("cache"))
    }

    pub fn dir_path(&self) -> &Path {
        &self.dir
    }

    pub fn save_file(&self, filename: &str, content: &str) -> Result<(), AppError> {
        let path = self.dir.join(filename);
        fs::write(&path, content)
            .map_err(|e| AppError::Storage(format!("Cannot write cache file '{filename}': {e}")))
    }

    pub fn load_file(&self, filename: &str) -> Result<Option<String>, AppError> {
        let path = self.dir.join(filename);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)
            .map_err(|e| AppError::Storage(format!("Cannot read cache file '{filename}': {e}")))?;
        Ok(Some(content))
    }

    pub fn delete_file(&self, filename: &str) -> Result<(), AppError> {
        let path = self.dir.join(filename);
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| AppError::Storage(format!("Cannot delete cache file: {e}")))?;
        }
        Ok(())
    }

    /// Remove cache files older than max_age_days
    pub fn cleanup_stale(&self, max_age_days: u64) -> Result<usize, AppError> {
        let max_age = std::time::Duration::from_secs(max_age_days * 24 * 60 * 60);
        let now = std::time::SystemTime::now();
        let mut removed = 0;

        let entries = fs::read_dir(&self.dir)
            .map_err(|e| AppError::Storage(format!("Cannot read cache dir: {e}")))?;

        for entry in entries {
            let entry = entry.map_err(|e| AppError::Storage(format!("Cannot read entry: {e}")))?;
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "sql") {
                continue;
            }
            if let Ok(metadata) = fs::metadata(&path) {
                let modified = metadata.modified().unwrap_or(now);
                if let Ok(age) = now.duration_since(modified)
                    && age > max_age
                    && fs::remove_file(&path).is_ok()
                {
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    /// Enforce LRU: keep at most max_files, remove oldest by modification time
    pub fn enforce_lru(&self, max_files: usize) -> Result<usize, AppError> {
        let entries = fs::read_dir(&self.dir)
            .map_err(|e| AppError::Storage(format!("Cannot read cache dir: {e}")))?;

        let mut files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| AppError::Storage(format!("Cannot read entry: {e}")))?;
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "sql") {
                continue;
            }
            let modified = fs::metadata(&path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            files.push((path, modified));
        }

        if files.len() <= max_files {
            return Ok(0);
        }

        // Sort oldest first
        files.sort_by_key(|(_, t)| *t);
        let to_remove = files.len() - max_files;
        let mut removed = 0;
        for (path, _) in files.iter().take(to_remove) {
            if fs::remove_file(path).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// List all cached SQL files with their modification times
    pub fn list_files(&self) -> Result<Vec<(String, std::time::SystemTime)>, AppError> {
        let entries = fs::read_dir(&self.dir)
            .map_err(|e| AppError::Storage(format!("Cannot read cache dir: {e}")))?;

        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| AppError::Storage(format!("Cannot read entry: {e}")))?;
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "sql") {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                let modified = fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                files.push((name.to_string(), modified));
            }
        }
        Ok(files)
    }
}

// ---------------------------------------------------------------------------
// Export / Import (.dbx format)
// ---------------------------------------------------------------------------

const EXPORT_MAGIC: &[u8; 4] = b"DBTX";
const EXPORT_VERSION: u8 = 1;
// Flags: bit 0 = credentials included
const FLAG_CREDENTIALS: u8 = 0x01;

/// Options for export_bundle.
pub struct ExportOptions {
    pub include_credentials: bool,
    pub password: String,
}

/// Metadata about an export (written as manifest.json inside the archive).
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ExportManifest {
    pub version: u8,
    pub exported_at: String,
    pub dbtui_version: String,
    pub includes_credentials: bool,
    pub connection_count: usize,
    pub script_count: usize,
}

/// Result of import_bundle: all data parsed and ready for the UI to merge.
pub struct ImportResult {
    pub manifest: ExportManifest,
    pub connections: Vec<ConnectionConfig>,
    pub groups: Vec<String>,
    pub scripts: Vec<(String, String)>, // (relative_path, content)
    pub object_filters: std::collections::HashMap<String, Vec<String>>,
    pub script_connections: std::collections::HashMap<String, String>,
    pub bind_variables: std::collections::HashMap<String, String>,
}

/// Export all connections, scripts, and settings to an encrypted .dbx file.
///
/// Format: `[DBTX magic 4B][version 1B][flags 1B][encrypted payload...]`
/// The encrypted payload is a tar.gz containing manifest.json + data files.
pub fn export_bundle(dest: &Path, options: &ExportOptions) -> Result<ExportManifest, AppError> {
    let dir = data_dir()?;

    // --- Gather data ---
    let store = ConnectionStore::new()?;
    let mut connections = store.load("")?;
    if !options.include_credentials {
        for conn in &mut connections {
            conn.password = String::new();
        }
    }
    let connections_json = serde_json::to_string_pretty(&connections)
        .map_err(|e| AppError::Storage(format!("Serialize connections: {e}")))?;

    let groups_path = dir.join("groups.json");
    let groups_json = fs::read_to_string(&groups_path).unwrap_or_else(|_| "[]".to_string());

    let filters_path = dir.join("object_filters.json");
    let filters_json = fs::read_to_string(&filters_path).unwrap_or_else(|_| "{}".to_string());

    let script_conns_path = dir.join("script_connections.json");
    let script_conns_json =
        fs::read_to_string(&script_conns_path).unwrap_or_else(|_| "{}".to_string());

    let bind_vars_path = dir.join("bind_variables.json");
    let bind_vars_json = fs::read_to_string(&bind_vars_path).unwrap_or_else(|_| "{}".to_string());

    // Count scripts
    let scripts_dir = dir.join("scripts");
    let script_count = count_sql_files(&scripts_dir);

    // Build manifest
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let manifest = ExportManifest {
        version: EXPORT_VERSION,
        exported_at: format!("{now}"),
        dbtui_version: env!("CARGO_PKG_VERSION").to_string(),
        includes_credentials: options.include_credentials,
        connection_count: connections.len(),
        script_count,
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| AppError::Storage(format!("Serialize manifest: {e}")))?;

    // --- Build tar.gz in memory ---
    let mut archive_data = Vec::new();
    {
        let enc = flate2::write::GzEncoder::new(&mut archive_data, flate2::Compression::default());
        let mut tar = tar::Builder::new(enc);

        append_bytes(
            &mut tar,
            "dbtui_export/manifest.json",
            manifest_json.as_bytes(),
        )?;
        append_bytes(
            &mut tar,
            "dbtui_export/connections.json",
            connections_json.as_bytes(),
        )?;
        append_bytes(&mut tar, "dbtui_export/groups.json", groups_json.as_bytes())?;
        append_bytes(
            &mut tar,
            "dbtui_export/object_filters.json",
            filters_json.as_bytes(),
        )?;
        append_bytes(
            &mut tar,
            "dbtui_export/script_connections.json",
            script_conns_json.as_bytes(),
        )?;
        append_bytes(
            &mut tar,
            "dbtui_export/bind_variables.json",
            bind_vars_json.as_bytes(),
        )?;

        // Add scripts directory recursively
        if scripts_dir.exists() {
            add_scripts_to_tar(&mut tar, &scripts_dir, "dbtui_export/scripts")?;
        }

        tar.finish()
            .map_err(|e| AppError::Storage(format!("Archive finalize: {e}")))?;
    }

    // --- Encrypt ---
    let encrypted = encrypt(&archive_data, &options.password)?;

    // --- Write file: magic + version + flags + encrypted ---
    let flags = if options.include_credentials {
        FLAG_CREDENTIALS
    } else {
        0
    };
    let mut output = Vec::with_capacity(6 + encrypted.len());
    output.extend_from_slice(EXPORT_MAGIC);
    output.push(EXPORT_VERSION);
    output.push(flags);
    output.extend_from_slice(&encrypted);

    fs::write(dest, output)
        .map_err(|e| AppError::Storage(format!("Cannot write export file: {e}")))?;

    Ok(manifest)
}

/// Import a .dbx file: decrypt, extract, and return parsed data for merging.
pub fn import_bundle(src: &Path, password: &str) -> Result<ImportResult, AppError> {
    let data =
        fs::read(src).map_err(|e| AppError::Storage(format!("Cannot read import file: {e}")))?;

    // Validate header
    if data.len() < 6 {
        return Err(AppError::Storage(
            "File too small to be a .dbx export".into(),
        ));
    }
    if &data[0..4] != EXPORT_MAGIC {
        return Err(AppError::Storage(
            "Invalid file format (not a dbtui export)".into(),
        ));
    }
    let version = data[4];
    if version > EXPORT_VERSION {
        return Err(AppError::Storage(format!(
            "Export version {version} is newer than supported ({EXPORT_VERSION})"
        )));
    }
    // flags at data[5] — read but not strictly needed for import

    // Decrypt payload
    let encrypted = &data[6..];
    let archive_data = decrypt(encrypted, password)?;

    // Extract tar.gz in memory
    let decoder = flate2::read::GzDecoder::new(archive_data.as_slice());
    let mut archive = tar::Archive::new(decoder);

    let mut manifest: Option<ExportManifest> = None;
    let mut connections: Vec<ConnectionConfig> = vec![];
    let mut groups: Vec<String> = vec![];
    let mut scripts: Vec<(String, String)> = vec![];
    let mut object_filters: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut script_connections: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut bind_variables: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for entry_result in archive
        .entries()
        .map_err(|e| AppError::Storage(format!("Archive read: {e}")))?
    {
        let mut entry =
            entry_result.map_err(|e| AppError::Storage(format!("Archive entry: {e}")))?;
        let path_str = entry
            .path()
            .map_err(|e| AppError::Storage(format!("Entry path: {e}")))?
            .to_string_lossy()
            .to_string();

        // Read entry content
        let mut content = String::new();
        use std::io::Read;
        entry
            .read_to_string(&mut content)
            .map_err(|e| AppError::Storage(format!("Read entry {path_str}: {e}")))?;

        // Strip the "dbtui_export/" prefix
        let rel = path_str.strip_prefix("dbtui_export/").unwrap_or(&path_str);

        match rel {
            "manifest.json" => {
                manifest = serde_json::from_str(&content).ok();
            }
            "connections.json" => {
                connections = serde_json::from_str(&content).unwrap_or_default();
            }
            "groups.json" => {
                groups = serde_json::from_str(&content).unwrap_or_default();
            }
            "object_filters.json" => {
                object_filters = serde_json::from_str(&content).unwrap_or_default();
            }
            "script_connections.json" => {
                script_connections = serde_json::from_str(&content).unwrap_or_default();
            }
            "bind_variables.json" => {
                bind_variables = serde_json::from_str(&content).unwrap_or_default();
            }
            _ if rel.starts_with("scripts/") => {
                let script_path = rel.strip_prefix("scripts/").unwrap_or(rel);
                if !script_path.is_empty() && !content.is_empty() {
                    scripts.push((script_path.to_string(), content));
                }
            }
            _ => {} // ignore unknown entries
        }
    }

    let manifest = manifest.ok_or_else(|| AppError::Storage("Missing manifest.json".into()))?;

    Ok(ImportResult {
        manifest,
        connections,
        groups,
        scripts,
        object_filters,
        script_connections,
        bind_variables,
    })
}

// --- Tar helpers ---

fn append_bytes(
    tar: &mut tar::Builder<flate2::write::GzEncoder<&mut Vec<u8>>>,
    path: &str,
    data: &[u8],
) -> Result<(), AppError> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, path, data)
        .map_err(|e| AppError::Storage(format!("Tar append {path}: {e}")))
}

fn add_scripts_to_tar(
    tar: &mut tar::Builder<flate2::write::GzEncoder<&mut Vec<u8>>>,
    dir: &Path,
    tar_prefix: &str,
) -> Result<(), AppError> {
    if !dir.exists() {
        return Ok(());
    }
    let entries =
        fs::read_dir(dir).map_err(|e| AppError::Storage(format!("Read scripts dir: {e}")))?;

    for entry in entries {
        let entry = entry.map_err(|e| AppError::Storage(format!("Dir entry: {e}")))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let tar_path = format!("{tar_prefix}/{name}");

        if path.is_dir() {
            add_scripts_to_tar(tar, &path, &tar_path)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("sql") {
            let content = fs::read(&path)
                .map_err(|e| AppError::Storage(format!("Read script {name}: {e}")))?;
            append_bytes(tar, &tar_path, &content)?;
        }
    }
    Ok(())
}

fn count_sql_files(dir: &Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += count_sql_files(&path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("sql") {
                count += 1;
            }
        }
    }
    count
}
