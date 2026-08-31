//! 单账号凭据操作——纯函数化，供多账号池按账号调用。
use crate::config::CredentialData;
use std::collections::HashMap;

pub fn is_expired(c: &CredentialData) -> bool {
    let now_ms = chrono_now_ms();
    now_ms >= c.expires_at - 60_000
}

fn origin_of(domain: &str) -> &'static str {
    if domain.ends_with(".workbuddy.ai") {
        "https://www.workbuddy.ai"
    } else {
        "https://www.codebuddy.cn"
    }
}

/// Build upstream headers — mirrors accounts.py:_build_headers_from
pub fn build_headers(c: &CredentialData) -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert("Content-Type".into(), "application/json".into());
    h.insert("Accept".into(), "application/json, text/plain, */*".into());
    h.insert("X-Requested-With".into(), "XMLHttpRequest".into());
    let origin = origin_of(&c.domain);
    h.insert("Origin".into(), origin.into());
    h.insert("Referer".into(), format!("{origin}/"));
    h.insert("User-Agent".into(), "CLI/2.63.2 CodeBuddy/2.63.2".into());
    h.insert("X-Product".into(), "SaaS".into());
    if c.access_token.is_empty() {
        h.insert("X-No-Authorization".into(), "1".into());
    } else {
        h.insert("Authorization".into(), format!("Bearer {}", c.access_token));
    }
    if c.uid.is_empty() {
        h.insert("X-No-User-Id".into(), "1".into());
    } else {
        h.insert("X-User-Id".into(), c.uid.clone());
    }
    if c.enterprise_id.is_empty() {
        h.insert("X-No-Enterprise-Id".into(), "1".into());
    } else {
        h.insert("X-Enterprise-Id".into(), c.enterprise_id.clone());
    }
    if c.domain.is_empty() {
        h.insert("X-No-Department-Info".into(), "1".into());
    } else {
        h.insert("X-Domain".into(), c.domain.clone());
    }
    h
}

pub fn backend_base(c: &CredentialData) -> String {
    if c.domain.ends_with(".workbuddy.ai") {
        "https://www.workbuddy.ai".into()
    } else {
        "https://copilot.tencent.com".into()
    }
}

#[allow(dead_code)]
pub fn billing_base(c: &CredentialData) -> String {
    if c.domain.ends_with(".workbuddy.ai") {
        "https://www.workbuddy.ai".into()
    } else {
        "https://www.codebuddy.cn".into()
    }
}

/// Refresh token — 返回更新后的凭据（由调用方决定落盘）。
pub async fn refresh(cred: &CredentialData) -> Result<CredentialData, String> {
    let base = if cred.domain.ends_with(".workbuddy.ai") {
        "https://www.workbuddy.ai"
    } else {
        "https://copilot.tencent.com"
    };
    let url = format!("{base}/v2/plugin/auth/token/refresh");

    let mut headers: HashMap<String, String> = HashMap::new();
    // Build refresh headers: same as build_headers but without Authorization, with X-Refresh-Token
    headers.insert("Content-Type".into(), "application/json".into());
    headers.insert("Accept".into(), "application/json, text/plain, */*".into());
    headers.insert("X-Requested-With".into(), "XMLHttpRequest".into());
    let origin = origin_of(&cred.domain);
    headers.insert("Origin".into(), origin.into());
    headers.insert("Referer".into(), format!("{origin}/"));
    headers.insert("User-Agent".into(), "CLI/2.63.2 CodeBuddy/2.63.2".into());
    headers.insert("X-Product".into(), "SaaS".into());
    headers.insert("X-Refresh-Token".into(), cred.refresh_token.clone());
    headers.insert("X-Auth-Refresh-Source".into(), "workbuddy".into());
    if cred.uid.is_empty() {
        headers.insert("X-No-User-Id".into(), "1".into());
    } else {
        headers.insert("X-User-Id".into(), cred.uid.clone());
    }
    if cred.enterprise_id.is_empty() {
        headers.insert("X-No-Enterprise-Id".into(), "1".into());
    } else {
        headers.insert("X-Enterprise-Id".into(), cred.enterprise_id.clone());
    }
    if cred.domain.is_empty() {
        headers.insert("X-No-Department-Info".into(), "1".into());
    } else {
        headers.insert("X-Domain".into(), cred.domain.clone());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let mut req = client.post(&url).json(&serde_json::json!({}));
    for (k, v) in &headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if data.get("code").and_then(|v| v.as_i64()) != Some(0) {
        return Err(format!("refresh failed: {}", data));
    }
    let new_auth = data.get("data").ok_or("refresh: missing data")?;

    let now_ms = chrono_now_ms();
    let mut updated = cred.clone();
    if let Some(v) = new_auth.get("accessToken").and_then(|v| v.as_str()) {
        updated.access_token = v.to_string();
    }
    if let Some(v) = new_auth.get("refreshToken").and_then(|v| v.as_str()) {
        updated.refresh_token = v.to_string();
    }
    if let Some(v) = new_auth.get("domain").and_then(|v| v.as_str()) {
        if !v.is_empty() {
            updated.domain = v.to_string();
        }
    }
    if let Some(v) = new_auth.get("expiresAt").and_then(|v| v.as_i64()) {
        updated.expires_at = v;
    } else if let Some(v) = new_auth.get("expiresIn").and_then(|v| v.as_i64()) {
        updated.expires_at = now_ms + v * 1000;
    }
    Ok(updated)
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Validate imported credential JSON.
pub fn validate_import(json_str: &str) -> Result<CredentialData, String> {
    let v: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("JSON 解析失败: {e}"))?;

    // Support two shapes:
    // 1. Raw CredentialData {access_token, refresh_token, ...}
    // 2. Full .info file {auth: {accessToken,...}, account: {uid,...}}
    if v.get("auth").is_some() {
        let auth = &v["auth"];
        let account = v.get("account").unwrap_or(&serde_json::Value::Null);
        Ok(CredentialData {
            access_token: auth
                .get("accessToken")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            refresh_token: auth
                .get("refreshToken")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            expires_at: auth.get("expiresAt").and_then(|x| x.as_i64()).unwrap_or(0),
            domain: auth
                .get("domain")
                .and_then(|x| x.as_str())
                .unwrap_or("www.codebuddy.cn")
                .to_string(),
            uid: account
                .get("uid")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            enterprise_id: account
                .get("enterpriseId")
                .and_then(|x| x.as_str())
                .or_else(|| account.get("enterpriseName").and_then(|x| x.as_str()))
                .unwrap_or("")
                .to_string(),
            nickname: account
                .get("nickname")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        })
    } else {
        let cd: CredentialData =
            serde_json::from_value(v).map_err(|e| format!("字段缺失: {e}"))?;
        if cd.access_token.is_empty() || cd.refresh_token.is_empty() {
            return Err("access_token / refresh_token 不能为空".into());
        }
        Ok(cd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cred() -> CredentialData {
        CredentialData {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at: 0,
            domain: "www.codebuddy.cn".into(),
            uid: "u1".into(),
            enterprise_id: "e1".into(),
            nickname: "n".into(),
        }
    }

    #[test]
    fn headers_carry_identity() {
        let h = build_headers(&cred());
        assert_eq!(h.get("Authorization").unwrap(), "Bearer at");
        assert_eq!(h.get("X-User-Id").unwrap(), "u1");
        assert_eq!(h.get("X-Domain").unwrap(), "www.codebuddy.cn");
        assert_eq!(h.get("Origin").unwrap(), "https://www.codebuddy.cn");
        assert!(!h.contains_key("X-No-Authorization"));
    }

    #[test]
    fn workbuddy_domain_maps_origins() {
        let mut c = cred();
        c.domain = "abc.workbuddy.ai".into();
        let h = build_headers(&c);
        assert_eq!(h.get("Origin").unwrap(), "https://www.workbuddy.ai");
        assert_eq!(backend_base(&c), "https://www.workbuddy.ai");
        assert_eq!(billing_base(&c), "https://www.workbuddy.ai");
        assert_eq!(backend_base(&cred()), "https://copilot.tencent.com");
    }

    #[test]
    fn import_rejects_empty_tokens() {
        assert!(validate_import("{\"access_token\":\"\",\"refresh_token\":\"\"}").is_err());
        assert!(validate_import("not json").is_err());
    }

    #[test]
    fn import_accepts_info_shape() {
        let raw = r#"{"auth":{"accessToken":"a","refreshToken":"r","expiresAt":5,"domain":"www.codebuddy.cn"},"account":{"uid":"u9","nickname":"nick","enterpriseId":"ent"}}"#;
        let c = validate_import(raw).unwrap();
        assert_eq!(c.uid, "u9");
        assert_eq!(c.access_token, "a");
        assert_eq!(c.expires_at, 5);
        assert_eq!(c.nickname, "nick");
    }
}
