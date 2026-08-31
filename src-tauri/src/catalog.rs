/// Model catalog — single CN account, mirrors models_catalog.py
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMeta {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<f64>,
    #[serde(default)]
    pub free: bool,
    #[serde(default)]
    pub badges: Vec<String>,
}

// CN fallback — from models_catalog.py CN_FALLBACK_MODELS
pub fn cn_fallback_models() -> Vec<ModelMeta> {
    vec![
        m("auto", "Auto", Some(168000), Some(32000), None, false),
        m("hy3", "Hy3", Some(192000), Some(64000), Some(0.0), true),
        m("hy3-x", "Hy3 x", Some(192000), Some(64000), Some(0.05), false),
        m("hy4-preview", "Hy4 preview", Some(1000000), Some(64000), Some(0.0), true),
        m("hy4-preview-x", "Hy4 preview x", Some(1000000), Some(64000), Some(0.29), false),
        m("default", "Default", Some(200000), Some(24000), Some(2.20), false),
        m("glm-5.3", "Glm 5.3", Some(1000000), Some(48000), Some(0.79), false),
        m("glm-5.3-flash", "Glm 5.3 flash", Some(1000000), Some(32000), Some(0.06), false),
        m("glm-5.2", "Glm 5.2", Some(1000000), Some(48000), Some(0.79), false),
        m("glm-5v-turbo", "Glm 5v turbo", Some(200000), Some(64000), Some(0.71), false),
        m("deepseek-v4-flash", "Deepseek v4 flash", Some(1000000), Some(50000), Some(0.17), false),
        m("deepseek-v4-pro", "Deepseek v4 pro", Some(1000000), Some(50000), Some(0.51), false),
        m("kimi-k3-1", "Kimi k3.1", Some(1000000), Some(32000), Some(1.62), false),
        m("kimi-k2.7", "Kimi k2.7", Some(256000), Some(32000), Some(0.57), false),
        m("minimax-m3", "MiniMax m3", Some(512000), Some(128000), Some(0.25), false),
        m("hunyuan-2.0-thinking", "Hunyuan 2.0 thinking", Some(128000), Some(24000), Some(0.04), false),
    ]
}

fn m(
    id: &str,
    name: &str,
    max_in: Option<u32>,
    max_out: Option<u32>,
    credits: Option<f64>,
    free: bool,
) -> ModelMeta {
    ModelMeta {
        id: id.into(),
        name: name.into(),
        max_input_tokens: max_in,
        max_output_tokens: max_out,
        credits,
        free,
        badges: if free {
            vec!["badge:限时免费:#FF0000".into()]
        } else {
            vec![]
        },
    }
}

pub struct ModelCatalog {
    inner: Arc<RwLock<HashMap<String, ModelMeta>>>,
}

impl Default for ModelCatalog {
    fn default() -> Self {
        let mut map = HashMap::new();
        for mm in cn_fallback_models() {
            map.insert(mm.id.clone(), mm);
        }
        Self {
            inner: Arc::new(RwLock::new(map)),
        }
    }
}

impl ModelCatalog {
    #[allow(dead_code)]
    pub async fn all_models(&self) -> Vec<ModelMeta> {
        let map = self.inner.read().await;
        let mut v: Vec<ModelMeta> = map.values().cloned().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    pub async fn to_api_list(&self) -> Vec<serde_json::Value> {
        let map = self.inner.read().await;
        let mut out: Vec<serde_json::Value> = Vec::new();
        let mut ids: Vec<String> = map.keys().cloned().collect();
        ids.sort();
        for id in ids {
            let meta = &map[&id];
            let mut item = serde_json::json!({
                "id": meta.id,
                "object": "model",
                "created": 1700000000,
                "owned_by": "workbuddy",
                "x_free": meta.free,
            });
            if let Some(c) = meta.credits {
                item["x_credits"] = serde_json::json!(c);
            }
            if let Some(v) = meta.max_input_tokens {
                item["context_window"] = serde_json::json!(v);
            }
            if let Some(v) = meta.max_output_tokens {
                item["max_output_tokens"] = serde_json::json!(v);
            }
            if !meta.badges.is_empty() {
                item["badges"] = serde_json::json!(meta.badges);
            }
            out.push(item);
        }
        // Ensure auto present
        if !out.iter().any(|m| m["id"] == "auto") {
            out.insert(
                0,
                serde_json::json!({"id": "auto", "object": "model", "created": 1700000000, "owned_by": "workbuddy"}),
            );
        }
        out
    }

    /// Fetch from upstream, fallback to static.
    pub async fn sync(&self, headers: &HashMap<String, String>) {
        let fetched = fetch_models(headers).await;
        if let Some(models) = fetched {
            if !models.is_empty() {
                let mut map = self.inner.write().await;
                map.clear();
                for mm in models {
                    map.insert(mm.id.clone(), mm);
                }
            }
        }
    }
}

async fn fetch_models(headers: &HashMap<String, String>) -> Option<Vec<ModelMeta>> {
    let paths = ["/console/enterprises/personal/models", "/console/enterprises/models"];
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .ok()?;

    for path in paths {
        let url = format!("https://copilot.tencent.com{path}");
        let mut req = client.get(&url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(_) => continue,
        };
        if resp.status() != 200 {
            continue;
        }
        let data: serde_json::Value = resp.json().await.ok()?;
        let inner = data.get("data")?;
        let models = inner.get("models").and_then(|v| v.as_array())?;
        if models.is_empty() {
            continue;
        }
        let agents = inner.get("agents").and_then(|v| v.as_array());
        let filtered = filter_usable(models, agents);
        if filtered.is_empty() {
            continue;
        }
        let mut out = Vec::new();
        for m in &filtered {
            if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                let credits = m
                    .get("credits")
                    .and_then(|v| v.as_str())
                    .and_then(parse_credits);
                let tags = m
                    .get("tags")
                    .or_else(|| m.get("badges"))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .filter(|s| s.starts_with("badge:") || s.contains("免费"))
                            .collect()
                    })
                    .unwrap_or_default();
                out.push(ModelMeta {
                    id: id.to_string(),
                    name: m
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(id)
                        .to_string(),
                    max_input_tokens: m
                        .get("maxInputTokens")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32),
                    max_output_tokens: m
                        .get("maxOutputTokens")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32),
                    credits,
                    free: credits == Some(0.0),
                    badges: tags,
                });
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    None
}

fn filter_usable<'a>(
    models: &'a [serde_json::Value],
    agents: Option<&Vec<serde_json::Value>>,
) -> Vec<&'a serde_json::Value> {
    if let Some(agents) = agents {
        for ag in agents {
            if ag.get("name").and_then(|v| v.as_str()) == Some("cli") {
                if let Some(ids) = ag.get("models").and_then(|v| v.as_array()) {
                    let id_set: std::collections::HashSet<&str> =
                        ids.iter().filter_map(|v| v.as_str()).collect();
                    let filtered: Vec<&serde_json::Value> = models
                        .iter()
                        .filter(|m| {
                            m.get("id")
                                .and_then(|v| v.as_str())
                                .map(|id| id_set.contains(id))
                                .unwrap_or(false)
                                && !m.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false)
                        })
                        .collect();
                    if !filtered.is_empty() {
                        return filtered;
                    }
                }
            }
        }
    }
    models
        .iter()
        .filter(|m| !m.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false))
        .collect()
}

fn parse_credits(raw: &str) -> Option<f64> {
    let re = regex::Regex::new(r"x([\d.]+)").ok()?;
    let cap = re.captures(raw)?;
    cap.get(1)?.as_str().parse().ok()
}
