use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 上游凭据（单条账号的登录态）
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

/// 单条账号：凭据 + 启用开关。多账号轮询负载均衡。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountEntry {
    /// 稳定唯一 id（优先 uid，其次随机 hex），运行态（冷却/计数）以此为主键
    #[serde(default)]
    pub id: String,
    pub credential: CredentialData,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl AccountEntry {
    pub fn new(credential: CredentialData) -> Self {
        let mut e = Self {
            id: String::new(),
            credential,
            enabled: true,
        };
        e.ensure_id();
        e
    }

    /// 补齐稳定唯一 id：优先 uid，缺失则随机生成
    pub fn ensure_id(&mut self) {
        if self.id.is_empty() {
            self.id = if self.credential.uid.is_empty() {
                rand_hex(8)
            } else {
                self.credential.uid.clone()
            };
        }
    }
}

/// Application config — CN 多账号，固定 127.0.0.1:9178 绑定。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_api_key")]
    pub api_key: String,
    /// 多账号池（顺序即轮询起始顺序）
    #[serde(default)]
    pub accounts: Vec<AccountEntry>,
    /// 旧版单账号字段：仅用于读取迁移，迁移后不再序列化
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
            accounts: Vec::new(),
            credential: None,
            desensitize: false,
            model_sync_interval_hours: 24,
        }
    }
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
        let mut cfg = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str::<AppConfig>(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        let mut dirty = cfg.migrate_legacy();
        dirty |= cfg.normalize_ids();
        if dirty {
            let _ = cfg.save();
        }
        cfg
    }

    /// 旧版单账号 credential → accounts[0] 迁移。返回是否发生变更。
    pub fn migrate_legacy(&mut self) -> bool {
        if self.accounts.is_empty() {
            if let Some(c) = self.credential.take() {
                self.accounts.push(AccountEntry::new(c));
                return true;
            }
        } else if self.credential.is_some() {
            // 已有多账号：旧字段视为残留，直接丢弃
            self.credential = None;
            return true;
        }
        false
    }

    /// 规范化账号 id 并去重。返回是否发生变更。
    fn normalize_ids(&mut self) -> bool {
        let mut dirty = false;
        let mut seen = std::collections::HashSet::new();
        for a in self.accounts.iter_mut() {
            if a.id.is_empty() {
                a.ensure_id();
                dirty = true;
            }
            while !seen.insert(a.id.clone()) {
                a.id = rand_hex(8);
                dirty = true;
            }
        }
        dirty
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
            self.api_key = format!("sk-buddy-{}", rand_hex(16));
            let _ = self.save();
        }
        &self.api_key
    }

    /// 导入/更新账号：uid 相同则视为更新凭据，否则追加。返回动作名。
    pub fn upsert_account(&mut self, cred: CredentialData) -> &'static str {
        if !cred.uid.is_empty() {
            if let Some(a) = self
                .accounts
                .iter_mut()
                .find(|a| a.credential.uid == cred.uid)
            {
                a.credential = cred;
                return "updated";
            }
        }
        self.accounts.push(AccountEntry::new(cred));
        "added"
    }

    pub fn remove_account(&mut self, id: &str) -> bool {
        let before = self.accounts.len();
        self.accounts.retain(|a| a.id != id);
        self.accounts.len() != before
    }

    pub fn set_account_enabled(&mut self, id: &str, enabled: bool) -> bool {
        match self.accounts.iter_mut().find(|a| a.id == id) {
            Some(a) => {
                a.enabled = enabled;
                true
            }
            None => false,
        }
    }

    #[allow(dead_code)]
    pub fn enabled_accounts(&self) -> Vec<&AccountEntry> {
        self.accounts.iter().filter(|a| a.enabled).collect()
    }

    /// Redacted view for /health and get_config — never expose tokens.
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
            // accounts 只输出摘要字段，access/refresh token 绝不外泄
            let accounts: Vec<serde_json::Value> = self
                .accounts
                .iter()
                .map(|a| {
                    serde_json::json!({
                        "id": a.id,
                        "uid": a.credential.uid,
                        "nickname": a.credential.nickname,
                        "domain": a.credential.domain,
                        "expires_at": a.credential.expires_at,
                        "enabled": a.enabled,
                    })
                })
                .collect();
            obj.insert("accounts".to_string(), serde_json::Value::Array(accounts));
            // 旧版单账号字段：一律不输出
            obj.insert("credential".to_string(), serde_json::Value::Null);
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cred(uid: &str) -> CredentialData {
        CredentialData {
            access_token: "at-secret".into(),
            refresh_token: "rt-secret".into(),
            expires_at: 123,
            domain: "d".into(),
            uid: uid.into(),
            enterprise_id: "e1".into(),
            nickname: "n1".into(),
        }
    }

    #[test]
    fn redacted_hides_tokens() {
        let mut cfg = AppConfig::default();
        cfg.api_key = "sk-buddy-x".into();
        cfg.accounts.push(AccountEntry::new(cred("u1")));
        cfg.accounts.push(AccountEntry::new(cred("u2")));
        cfg.accounts[1].enabled = false;
        let v = cfg.redacted();
        let s = serde_json::to_string(&v).unwrap();
        assert!(!s.contains("at-secret"), "access_token 泄漏");
        assert!(!s.contains("rt-secret"), "refresh_token 泄漏");
        assert_eq!(v["api_key"], "**");
        assert_eq!(v["accounts"].as_array().unwrap().len(), 2);
        assert_eq!(v["accounts"][0]["uid"], "u1");
        assert_eq!(v["accounts"][1]["enabled"], false);
    }

    #[test]
    fn legacy_single_account_migrates() {
        let raw = r#"{"api_key":"sk-buddy-x","credential":{"access_token":"at","refresh_token":"rt","expires_at":1,"domain":"d","uid":"u1","enterprise_id":"","nickname":"n"},"desensitize":false,"model_sync_interval_hours":24}"#;
        let mut cfg: AppConfig = serde_json::from_str(raw).unwrap();
        assert!(cfg.migrate_legacy());
        assert_eq!(cfg.accounts.len(), 1);
        assert!(cfg.accounts[0].enabled);
        assert_eq!(cfg.accounts[0].credential.uid, "u1");
        assert_eq!(cfg.accounts[0].id, "u1");
        assert!(cfg.credential.is_none());
        // 迁移后旧字段不再序列化（注意 accounts 内部也有 credential 键，只查顶层）
        let v: serde_json::Value = serde_json::to_value(&cfg).unwrap();
        assert!(v.get("credential").is_none(), "legacy 字段残留: {v}");
        // 幂等
        assert!(!cfg.migrate_legacy());
    }

    #[test]
    fn normalize_ids_dedups() {
        let mut cfg = AppConfig::default();
        let mut a1 = AccountEntry::new(cred("u1"));
        a1.id = "same".into();
        let mut a2 = AccountEntry::new(cred("u2"));
        a2.id = "same".into();
        cfg.accounts = vec![a1, a2];
        assert!(cfg.normalize_ids());
        assert_ne!(cfg.accounts[0].id, cfg.accounts[1].id);
        // 无 id 时按 uid 补齐（new() 构造即已补齐）
        let mut cfg2 = AppConfig::default();
        cfg2.accounts.push(AccountEntry::new(cred("uid-x")));
        assert_eq!(cfg2.accounts[0].id, "uid-x");
    }

    #[test]
    fn upsert_updates_same_uid() {
        let mut cfg = AppConfig::default();
        assert_eq!(cfg.upsert_account(cred("u1")), "added");
        let mut c2 = cred("u1");
        c2.access_token = "at-new".into();
        assert_eq!(cfg.upsert_account(c2), "updated");
        assert_eq!(cfg.accounts.len(), 1);
        assert_eq!(cfg.accounts[0].credential.access_token, "at-new");
        assert_eq!(cfg.upsert_account(cred("u2")), "added");
        assert_eq!(cfg.accounts.len(), 2);
    }

    #[test]
    fn remove_and_toggle() {
        let mut cfg = AppConfig::default();
        cfg.accounts.push(AccountEntry::new(cred("u1")));
        cfg.accounts.push(AccountEntry::new(cred("u2")));
        let id1 = cfg.accounts[0].id.clone();
        assert!(cfg.set_account_enabled(&id1, false));
        assert!(!cfg.set_account_enabled("nope", false));
        assert_eq!(cfg.enabled_accounts().len(), 1);
        assert!(cfg.remove_account(&id1));
        assert!(!cfg.remove_account(&id1));
        assert_eq!(cfg.accounts.len(), 1);
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

fn rand_hex(n: usize) -> String {
    use rand::RngCore;
    let mut bytes = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
