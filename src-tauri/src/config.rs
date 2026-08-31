use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Application config — single CN account, fixed 127.0.0.1:9178 binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_api_key")]
    pub api_key: String,
    #[serde(default)]
    pub credential: Option<CredentialData>,
    #[serde(default)]
    pub desensitize: bool,
    #[serde(default)]
    pub model_sync_interval_hours: u32,
}

fn default_api_key() -> String {
    String::new()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            credential: None,
            desensitize: false,
            model_sync_interval_hours: 24,
        }
    }
}

/// Credential stored inside config (single CN account).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialData {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub domain: String,
    pub uid: String,
    #[serde(default)]
    pub enterprise_id: String,
    #[serde(default)]
    pub nickname: String,
}

impl AppConfig {
    pub fn config_path() -> Option<PathBuf> {
        let base = dirs_next();
        base.map(|p| p.join("buddyaigateway").join("config.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::config_path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self).unwrap_or_default();
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Generate a random API key if empty, persist it.
    pub fn ensure_api_key(&mut self) -> &str {
        if self.api_key.is_empty() {
            self.api_key = generate_api_key();
            let _ = self.save();
        }
        &self.api_key
    }

    /// Redacted view for /health — masks api_key.
    pub fn redacted(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        if let Some(obj) = v.as_object_mut() {
            if obj.contains_key("api_key") {
                let has_key = self.api_key.is_empty();
                obj.insert(
                    "api_key".to_string(),
                    serde_json::Value::String(if has_key { "".into() } else { "**".into() }),
                );
            }
        }
        v
    }
}

fn dirs_next() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("APPDATA") {
        let pb = PathBuf::from(p);
        if pb.exists() || cfg!(windows) {
            return Some(pb);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let pb = PathBuf::from(home);
        #[cfg(target_os = "macos")]
        {
            let m = pb.join("Library").join("Application Support");
            if m.exists() {
                return Some(m);
            }
        }
        let xdg = pb.join(".config");
        return Some(xdg);
    }
    dirs::config_dir()
}

fn generate_api_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("sk-buddy-{}", hex::encode(bytes))
}

mod hex {
    pub fn encode(bytes: [u8; 16]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[allow(dead_code)]
mod dirs {
    use std::path::PathBuf;
    pub fn config_dir() -> Option<PathBuf> {
        #[cfg(windows)]
        {
            std::env::var("APPDATA").ok().map(PathBuf::from)
        }
        #[cfg(target_os = "macos")]
        {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join("Library").join("Application Support"))
        }
        #[cfg(all(not(windows), not(target_os = "macos")))]
        {
            std::env::var("XDG_CONFIG_HOME")
                .ok()
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var("HOME")
                        .ok()
                        .map(|h| PathBuf::from(h).join(".config"))
                })
        }
    }
}
