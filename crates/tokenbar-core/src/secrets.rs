//! Credential store.
//!
//! macOS keeps pasted tokens in the login keychain through the `keyring` crate.
//! Every other platform uses a `0600` JSON file. `TOKENBAR_SECRETS_PATH` forces
//! the file backend, including on macOS, which is how tests stay hermetic.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SecretKind {
    CursorSession,
    ClaudeOauth,
    OpenaiKey,
    DeepseekKey,
}

impl SecretKind {
    fn key(self) -> &'static str {
        match self {
            Self::CursorSession => "cursor_session",
            Self::ClaudeOauth => "claude_oauth",
            Self::OpenaiKey => "openai_key",
            Self::DeepseekKey => "deepseek_key",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::CursorSession => "Cursor session",
            Self::ClaudeOauth => "Claude OAuth",
            Self::OpenaiKey => "OpenAI key",
            Self::DeepseekKey => "DeepSeek key",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SecretPresence {
    pub cursor: bool,
    pub claude: bool,
    pub openai: bool,
    pub deepseek: bool,
}

pub(crate) trait SecretStore {
    fn get(&self, kind: SecretKind) -> Option<String>;
    fn set(&self, kind: SecretKind, value: &str) -> Result<(), String>;
    fn delete(&self, kind: SecretKind) -> Result<(), String>;
}

pub fn save_secret(kind: SecretKind, raw: &str) -> Result<(), String> {
    let value = validate_secret(kind, raw)?;
    open_store()?.set(kind, &value)
}

pub fn delete_secret(kind: SecretKind) -> Result<(), String> {
    open_store()?.delete(kind)
}

pub fn secret_presence() -> SecretPresence {
    match open_store() {
        Ok(store) => presence_of(store.as_ref()),
        Err(_) => SecretPresence::default(),
    }
}

pub(crate) fn presence_of(store: &dyn SecretStore) -> SecretPresence {
    SecretPresence {
        cursor: store.get(SecretKind::CursorSession).is_some(),
        claude: store.get(SecretKind::ClaudeOauth).is_some(),
        openai: store.get(SecretKind::OpenaiKey).is_some(),
        deepseek: store.get(SecretKind::DeepseekKey).is_some(),
    }
}

pub(crate) fn validate_secret(kind: SecretKind, raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("剪贴板是空的".to_string());
    }
    if value.len() > 16 * 1024 {
        return Err("内容太长".to_string());
    }
    if value
        .chars()
        .any(|ch| ch == '\n' || ch == '\r' || ch == '\0')
    {
        return Err("请粘贴单行凭据".to_string());
    }
    match kind {
        SecretKind::CursorSession if value.len() < 8 => Err("Cursor 登录内容太短".to_string()),
        SecretKind::ClaudeOauth if value.len() < 20 => Err("Claude 凭据太短".to_string()),
        SecretKind::OpenaiKey if !value.starts_with("sk-") => {
            Err("OpenAI 密钥格式不对".to_string())
        }
        SecretKind::DeepseekKey if value.len() < 8 => Err("DeepSeek 密钥太短".to_string()),
        _ => Ok(value.to_string()),
    }
}

pub(crate) fn open_store() -> Result<Box<dyn SecretStore>, String> {
    if let Some(path) = std::env::var_os("TOKENBAR_SECRETS_PATH") {
        return Ok(Box::new(FileSecretStore::new(PathBuf::from(path))));
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(MacSecretStore {
            file: FileSecretStore::new(default_secrets_path()),
        }))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(Box::new(FileSecretStore::new(default_secrets_path())))
    }
}

pub(crate) fn default_secrets_path() -> PathBuf {
    dirs::config_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tokenbar")
        .join("secrets.json")
}

pub(crate) struct MemoryStore {
    values: std::sync::Mutex<BTreeMap<SecretKind, String>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            values: std::sync::Mutex::new(BTreeMap::new()),
        }
    }

    pub fn insert(&self, kind: SecretKind, value: impl Into<String>) {
        self.values
            .lock()
            .expect("memory store")
            .insert(kind, value.into());
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, kind: SecretKind) -> Option<String> {
        self.values
            .lock()
            .expect("memory store")
            .get(&kind)
            .cloned()
            .filter(|value| !value.is_empty())
    }

    fn set(&self, kind: SecretKind, value: &str) -> Result<(), String> {
        self.insert(kind, value.to_string());
        Ok(())
    }

    fn delete(&self, kind: SecretKind) -> Result<(), String> {
        self.values.lock().expect("memory store").remove(&kind);
        Ok(())
    }
}

struct FileSecretStore {
    path: PathBuf,
}

impl FileSecretStore {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn load(&self) -> Result<Map<String, Value>, String> {
        if !self.path.exists() {
            return Ok(Map::new());
        }
        let text = fs::read_to_string(&self.path).map_err(|err| format!("read secrets: {err}"))?;
        if text.trim().is_empty() {
            return Ok(Map::new());
        }
        serde_json::from_str(&text).map_err(|err| format!("secrets file is not valid JSON: {err}"))
    }

    fn store(&self, map: &Map<String, Value>) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("create secrets dir: {err}"))?;
            set_dir_mode(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(map).map_err(|err| err.to_string())?;
        atomic_write(&self.path, &bytes)
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, kind: SecretKind) -> Option<String> {
        let map = self.load().ok()?;
        match map.get(kind.key())? {
            Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
            _ => None,
        }
    }

    fn set(&self, kind: SecretKind, value: &str) -> Result<(), String> {
        let mut map = self.load()?;
        map.insert(kind.key().to_string(), Value::String(value.to_string()));
        self.store(&map)
    }

    fn delete(&self, kind: SecretKind) -> Result<(), String> {
        let mut map = self.load()?;
        map.remove(kind.key());
        if map.is_empty() {
            if self.path.exists() {
                fs::remove_file(&self.path).map_err(|err| format!("remove secrets: {err}"))?;
            }
            return Ok(());
        }
        self.store(&map)
    }
}

#[cfg(target_os = "macos")]
struct MacSecretStore {
    file: FileSecretStore,
}

#[cfg(target_os = "macos")]
impl MacSecretStore {
    fn entry(kind: SecretKind) -> Result<keyring::Entry, String> {
        keyring::Entry::new("TokenBar", kind.key()).map_err(|err| err.to_string())
    }
}

#[cfg(target_os = "macos")]
impl SecretStore for MacSecretStore {
    fn get(&self, kind: SecretKind) -> Option<String> {
        if let Ok(entry) = Self::entry(kind) {
            if let Ok(value) = entry.get_password() {
                let value = value.trim().to_string();
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
        self.file.get(kind)
    }

    fn set(&self, kind: SecretKind, value: &str) -> Result<(), String> {
        match Self::entry(kind)
            .and_then(|entry| entry.set_password(value).map_err(|err| err.to_string()))
        {
            Ok(()) => Ok(()),
            Err(keychain_err) => self
                .file
                .set(kind, value)
                .map_err(|file_err| format!("keychain: {keychain_err}; file fallback: {file_err}")),
        }
    }

    fn delete(&self, kind: SecretKind) -> Result<(), String> {
        if let Ok(entry) = Self::entry(kind) {
            match entry.delete_credential() {
                Ok(()) => {}
                Err(err) => {
                    let text = err.to_string().to_ascii_lowercase();
                    if !text.contains("no entry") && !text.contains("not found") {
                        return Err(format!("keychain: {err}"));
                    }
                }
            }
        }
        self.file.delete(kind)
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut options = OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .map_err(|err| format!("write secrets: {err}"))?;
        file.write_all(bytes)
            .map_err(|err| format!("write secrets: {err}"))?;
        file.sync_all()
            .map_err(|err| format!("sync secrets: {err}"))?;
    }
    set_file_mode(&tmp)?;
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(path);
        fs::rename(&tmp, path).map_err(|err| format!("replace secrets: {err}"))?;
    }
    set_file_mode(path)?;
    Ok(())
}

fn set_dir_mode(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|err| format!("chmod secrets dir: {err}"))?;
    }
    let _ = path;
    Ok(())
}

fn set_file_mode(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|err| format!("chmod secrets: {err}"))?;
    }
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_pastes() {
        assert!(validate_secret(SecretKind::OpenaiKey, "sk-live").is_ok());
        assert!(validate_secret(SecretKind::OpenaiKey, "not-a-key").is_err());
        assert!(validate_secret(SecretKind::CursorSession, "short").is_err());
        assert!(validate_secret(SecretKind::ClaudeOauth, "line\nbreak-token-value").is_err());
        assert!(validate_secret(SecretKind::ClaudeOauth, "").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn file_store_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "tokenbar-secrets-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let path = dir.join("tokenbar").join("secrets.json");
        let store = FileSecretStore::new(path.clone());
        store
            .set(SecretKind::OpenaiKey, "sk-test-value")
            .expect("set");
        let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let dir_mode = fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600);
        assert_eq!(dir_mode, 0o700);
        assert_eq!(
            store.get(SecretKind::OpenaiKey).as_deref(),
            Some("sk-test-value")
        );
        store.delete(SecretKind::OpenaiKey).unwrap();
        assert!(store.get(SecretKind::OpenaiKey).is_none());
        let _ = fs::remove_dir_all(dir);
    }

    fn unique_suffix() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
}
