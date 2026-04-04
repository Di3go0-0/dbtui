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

#[allow(dead_code)]
pub fn export(dest: &Path, master_password: &str) -> Result<(), AppError> {
    let dir = data_dir()?;
    if !dir.exists() {
        return Err(AppError::Storage("No data to export".into()));
    }

    let mut archive_data = Vec::new();
    {
        let enc = flate2::write::GzEncoder::new(&mut archive_data, flate2::Compression::default());
        let mut tar_builder = tar::Builder::new(enc);
        tar_builder
            .append_dir_all("dbtui", &dir)
            .map_err(|e| AppError::Storage(format!("Archive creation failed: {e}")))?;
        tar_builder
            .finish()
            .map_err(|e| AppError::Storage(format!("Archive finalize failed: {e}")))?;
    }

    let encrypted = encrypt(&archive_data, master_password)?;
    fs::write(dest, encrypted)
        .map_err(|e| AppError::Storage(format!("Cannot write export file: {e}")))?;
    Ok(())
}

#[allow(dead_code)]
pub fn import(src: &Path, master_password: &str) -> Result<(), AppError> {
    let data =
        fs::read(src).map_err(|e| AppError::Storage(format!("Cannot read import file: {e}")))?;
    let decrypted = decrypt(&data, master_password)?;

    let dir = data_dir()?;
    let decoder = flate2::read::GzDecoder::new(decrypted.as_slice());
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(dir.parent().unwrap_or(&dir))
        .map_err(|e| AppError::Storage(format!("Import extraction failed: {e}")))?;
    Ok(())
}
