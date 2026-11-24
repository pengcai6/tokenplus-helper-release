//! Key + state storage — port of helper/store.py.
//! KeyStore: OS keychain via the `keyring` crate (macOS Keychain /
//! Windows Credential Manager / Secret Service).
//! StateStore: <data_root>/state.json (managed records + settings).

use anyhow::{bail, Result};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SERVICE: &str = "tokenplus-helper";

/// platformdirs equivalent: mac ~/Library/Application Support/<app>,
/// win %APPDATA%\<app>, linux $XDG_DATA_HOME/<app> or ~/.local/share/<app>.
pub fn data_root() -> PathBuf {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let base = dirs::data_dir();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = dirs::data_local_dir();
    let base = base
        .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("tokenplus-helper")
}

/// Secrets live as individual keychain entries (never one big blob —
/// Windows Credential Manager caps credentials at 2560 bytes).
pub struct KeyStore;
impl KeyStore {
    pub fn new() -> Self {
        KeyStore
    }

    fn entry(name: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, name).map_err(|_| {
            anyhow::anyhow!("系统安全存储不可用，未保存密钥。请检查系统钥匙串或凭据服务后重试")
        })
    }

    pub fn available() -> bool {
        match keyring::Entry::new(SERVICE, "api_key") {
            Ok(entry) => matches!(
                entry.get_password(),
                Ok(_) | Err(keyring::Error::NoEntry)
            ),
            Err(_) => false,
        }
    }

    fn assert_available() -> Result<()> {
        if !Self::available() {
            bail!("系统安全存储不可用，未保存密钥。请检查系统钥匙串或凭据服务后重试");
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<String> {
        Self::entry(name).ok()?.get_password().ok()
    }

    pub fn set(&self, name: &str, value: &str) -> Result<()> {
        Self::assert_available()?;
        Self::entry(name)?.set_password(value)?;
        Ok(())
    }

    pub fn get_json(&self, name: &str) -> Option<Value> {
        serde_json::from_str(&self.get(name)?).ok()
    }

    pub fn set_json(&self, name: &str, value: &Value) -> Result<()> {
        self.set(name, &serde_json::to_string(value)?)
    }

    pub fn delete(&self, name: &str) {
        if let Ok(entry) = Self::entry(name) {
            let _ = entry.delete_credential();
        }
    }
}

fn default_state() -> Value {
    json!({
        "settings": {"boot": true, "tray": true, "watch": true, "update": true},
        "managed": {},
    })
}

pub struct StateStore {
    file: PathBuf,
    state: Mutex<Value>,
}
impl StateStore {
    pub fn new(root: &Path) -> Self {
        let _ = fs::create_dir_all(root);
        let file = root.join("state.json");
        let mut state = default_state();
        if file.exists() {
            if let Ok(parsed) = serde_json::from_str::<Value>(
                &fs::read_to_string(&file).unwrap_or_default(),
            ) {
                let mut settings = default_state()["settings"].clone();
                if let (Some(base), Some(saved)) = (
                    settings.as_object_mut(),
                    parsed.get("settings").and_then(Value::as_object),
                ) {
                    for (key, value) in saved {
                        base.insert(key.clone(), value.clone());
                    }
                }
                state = json!({
                    "settings": settings,
                    "managed": match parsed.get("managed") {
                        Some(Value::Object(map)) => Value::Object(map.clone()),
                        _ => json!({}),
                    },
                });
            }
        }
        Self {
            file,
            state: Mutex::new(state),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Value> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn managed(&self) -> Map<String, Value> {
        self.lock()
            .get("managed")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }

    pub fn get_managed(&self, tool: &str) -> Option<Value> {
        self.managed().get(tool).cloned()
    }

    pub fn set_managed(&self, tool: &str, record: &Value) -> Result<()> {
        {
            let mut state = self.lock();
            state["managed"][tool] = record.clone();
        }
        self.save()
    }

    pub fn clear_managed(&self, tool: &str) -> Result<()> {
        {
            let mut state = self.lock();
            if let Some(map) = state["managed"].as_object_mut() {
                map.remove(tool);
            }
        }
        self.save()
    }

    pub fn settings(&self) -> Value {
        self.lock().get("settings").cloned().unwrap_or(json!({}))
    }

    pub fn set_settings(&self, settings: &Value) -> Result<()> {
        {
            let mut state = self.lock();
            state["settings"] = settings.clone();
        }
        self.save()
    }

    fn save(&self) -> Result<()> {
        let temp = self.file.with_file_name(format!(
            "{}.tmp",
            self.file.file_name().unwrap_or_default().to_string_lossy()
        ));
        fs::write(&temp, crate::fsutil::json_text(&self.lock().clone()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
        }
        fs::rename(&temp, &self.file)?;
        Ok(())
    }
}
