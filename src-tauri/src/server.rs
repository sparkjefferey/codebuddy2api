//! Axum HTTP gateway — CN 多账号轮询负载均衡 + 连续 429 自动切换。
use axum::{
    body::Body,
    extract::State,
    http::{header::HeaderName, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};

use crate::anthropic::{anthropic_to_chat, ensure_system_first, AnthropicStreamConverter};
use crate::billing;
use crate::catalog::ModelCatalog;
use crate::config::{AccountEntry, AppConfig, CredentialData};
use crate::credential;
use crate::ccswitch;
use crate::desensitize;
use crate::pool::{AccountPool, AccountState};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub pool: AccountPool,
    pub catalog: Arc<ModelCatalog>,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            pool: AccountPool::default(),
            catalog: Arc::new(ModelCatalog::default()),
        }
    }
}

/// 单账号对外摘要（含运行态），/health 与前端共用。
pub fn account_json(a: &AccountEntry, st: Option<&AccountState>) -> serde_json::Value {
    serde_json::json!({
        "id": a.id,
        "uid": a.credential.uid,
        "nickname": a.credential.nickname,
        "domain": a.credential.domain,
        "expires_at": a.credential.expires_at,
        "enabled": a.enabled,
        "consecutive_429": st.map(|s| s.consecutive_429).unwrap_or(0),
        "cooldown_until": st.map(|s| s.cooldown_until_ms).unwrap_or(0),
        "last_error": st.and_then(|s| s.last_error.clone()),
        "last_used_ms": st.and_then(|s| s.last_used_ms),
    })
}

// ---------------------------------------------------------------------------
// Auth helper
// ---------------------------------------------------------------------------

fn check_auth(headers: &HeaderMap, expected: &str) -> bool {
    if expected.is_empty() {
        return true;
    }
    if let Some(v) = headers.get("authorization") {
        if let Ok(s) = v.to_str() {
            if s.starts_with("Bearer ") && s[7..].trim() == expected {
                return true;
            }
        }
    }
    if let Some(v) = headers.get("x-api-key") {
        if let Ok(s) = v.to_str() {
            if s.trim() == expected {
                return true;
            }
        }
    }
    false
}

fn require_auth(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    // Read api_key without holding lock across await — clone it
    // We use try_read to avoid blocking; fallback to empty check
    let key = {
        // SAFETY: in Axum handlers we can block briefly; config is small
        let guard = state.config.try_read();
        guard.map(|g| g.api_key.clone()).unwrap_or_default()
    };
    if check_auth(headers, &key) {
        None
    } else {
        Some(
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": {"message": "invalid api key", "type": "auth_error"}})),
            )
                .into_response(),
        )
    }
}

// ---------------------------------------------------------------------------
// Account helpers
// ---------------------------------------------------------------------------

/// 读取指定账号凭据，临期自动刷新并落盘。
pub async fn fresh_credential(state: &AppState, id: &str) -> Result<CredentialData, String> {
    let cred = {
        let cfg = state.config.read().await;
        cfg.accounts
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.credential.clone())
            .ok_or_else(|| "账号不存在".to_string())?
    };
    if !credential::is_expired(&cred) {
        return Ok(cred);
    }
    let updated = credential::refresh(&cred).await?;
    persist_credential(state, id, updated.clone()).await;
    Ok(updated)
}

/// 将刷新后的凭据写回配置并落盘。
async fn persist_credential(state: &AppState, id: &str, updated: CredentialData) {
    let mut cfg = state.config.write().await;
    if let Some(a) = cfg.accounts.iter_mut().find(|a| a.id == id) {
        a.credential = updated;
        let _ = cfg.save();
    }
}

/// 对单个账号发起一次上游 chat 请求。
async fn send_for_account(
    cred: &CredentialData,
    chat_body: &serde_json::Value,
    timeout_secs: u64,
) -> Result<reqwest::Response, String> {
    let hdrs = credential::build_headers(cred);
    let base = credential::backend_base(cred);
    let url = format!("{base}/v2/chat/completions");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.post(&url).json(chat_body);
    for (k, v) in &hdrs {
        req = req.header(k.as_str(), v.as_str());
    }
    req.send().await.map_err(|e| e.to_string())
}

/// 多账号故障转移调度。
///
/// 轮询选号 → 401/403 先强制刷新重试一次 → 429 计数冷却并换号 →
/// 5xx/408/网络错误换号 → 其余 4xx 视为请求问题直接透传。
/// 仅在流开始前重试；一旦上游 200 即返回响应交给调用方流式转发。
async fn dispatch(
    state: &AppState,
    chat_body: &serde_json::Value,
    timeout_secs: u64,
) -> Result<reqwest::Response, Response> {
    let (candidates, total_enabled) = {
        let cfg = state.config.read().await;
        let ids: Vec<String> = cfg
            .accounts
            .iter()
            .filter(|a| a.enabled)
            .map(|a| a.id.clone())
            .collect();
        let enabled_n = ids.len();
        let order = state.pool.order_candidates(&ids);
        let candidates: Vec<(String, CredentialData)> = order
            .into_iter()
            .filter_map(|id| {
                cfg.accounts
                    .iter()
                    .find(|a| a.id == id)
                    .map(|a| (a.id.clone(), a.credential.clone()))
            })
            .collect();
        (candidates, enabled_n)
    };

    if candidates.is_empty() {
        let (msg, err_type) = if total_enabled == 0 {
            (
                "未配置可用账号，请先在「账号」页导入凭据",
                "auth_error",
            )
        } else {
            (
                "所有账号均处于 429 限流冷却中，请稍后重试",
                "rate_limit_error",
            )
        };
        return Err(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": {"message": msg, "type": err_type}})),
            )
                .into_response(),
        );
    }

    let mut last_err = String::new();
    let mut saw_429 = false;
    for (id, _) in &candidates {
        let mut cred = match fresh_credential(state, id).await {
            Ok(c) => c,
            Err(e) => {
                let m = format!("token 刷新失败: {e}");
                state.pool.on_error(id, &m);
                last_err = format!("账号 {id}: {m}");
                continue;
            }
        };
        let mut force_refresh = false;
        loop {
            if force_refresh {
                // 401/403：token 可能被吊销，强制刷新后重试同一账号一次
                match credential::refresh(&cred).await {
                    Ok(updated) => {
                        persist_credential(state, id, updated.clone()).await;
                        cred = updated;
                    }
                    Err(e) => {
                        let m = format!("token 强制刷新失败: {e}");
                        state.pool.on_error(id, &m);
                        last_err = format!("账号 {id}: {m}");
                        break;
                    }
                }
            }
            match send_for_account(&cred, chat_body, timeout_secs).await {
                Err(net) => {
                    let m = format!("网络错误: {net}");
                    state.pool.on_error(id, &m);
                    last_err = format!("账号 {id}: {m}");
                    break;
                }
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if status == 200 {
                        state.pool.on_success(id);
                        return Ok(resp);
                    }
                    if status == 429 {
                        if let Some(secs) = state.pool.on_429(id) {
                            eprintln!("[pool] 账号 {id} 连续 429，冷却 {secs}s");
                        } else {
                            eprintln!("[pool] 账号 {id} 上游 429，切换下一账号");
                        }
                        saw_429 = true;
                        last_err = format!("账号 {id}: 上游限流(429)");
                        break;
                    }
                    if (status == 401 || status == 403) && !force_refresh {
                        force_refresh = true;
                        continue;
                    }
                    if status == 408 || status >= 500 {
                        let m = format!("上游 {status}");
                        state.pool.on_error(id, &m);
                        last_err = format!("账号 {id}: {m}");
                        break;
                    }
                    // 其它 4xx：请求本身的问题，换账号无意义 → 直接透传
                    let m = format!("上游 {status}");
                    state.pool.on_error(id, &m);
                    let text = resp.text().await.unwrap_or_default();
                    let err_body: serde_json::Value = serde_json::from_str(&text).unwrap_or(
                        serde_json::json!({
                            "error": {
                                "message": text.chars().take(500).collect::<String>(),
                                "type": "upstream_error",
                                "code": status,
                            }
                        }),
                    );
                    return Err(
                        (
                            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                            Json(err_body),
                        )
                            .into_response(),
                    );
                }
            }
        }
    }

    let status = if saw_429 {
        StatusCode::TOO_MANY_REQUESTS
    } else {
        StatusCode::BAD_GATEWAY
    };
    Err(
        (
            status,
            Json(serde_json::json!({
                "error": {
                    "message": format!("所有可用账号均请求失败：{last_err}"),
                    "type": if saw_429 { "rate_limit_error" } else { "api_error" },
                }
            })),
        )
            .into_response(),
    )
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let cfg = state.config.read().await;
    let snap = state.pool.snapshot();
    let enabled = cfg.accounts.iter().filter(|a| a.enabled).count();
    let status = if enabled > 0 { "ok" } else { "degraded" };
    let accounts: Vec<serde_json::Value> = cfg
        .accounts
        .iter()
        .map(|a| account_json(a, snap.get(&a.id)))
        .collect();
    let info = serde_json::json!({
        "status": status,
        "version": "1.1.0",
        "mode": "buddyaigateway",
        "accounts": accounts,
        "config": cfg.redacted(),
    });
    Json(info)
}

async fn models_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_auth(&state, &headers) {
        return r;
    }
    let list = state.catalog.to_api_list().await;
    Json(serde_json::json!({"object": "list", "data": list})).into_response()
}

async fn credits_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_auth(&state, &headers) {
        return r;
    }
    let accounts: Vec<(String, String, String, CredentialData)> = {
        let cfg = state.config.read().await;
        cfg.accounts
            .iter()
            .filter(|a| a.enabled)
            .map(|a| {
                (
                    a.id.clone(),
                    a.credential.uid.clone(),
                    a.credential.nickname.clone(),
                    a.credential.clone(),
                )
            })
            .collect()
    };
    if accounts.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": {"message": "未配置账号", "type": "auth_error"}})),
        )
            .into_response();
    }
    let mut out: Vec<serde_json::Value> = Vec::new();
    let mut total: i64 = 0;
    for (id, uid, nickname, cred) in &accounts {
        let cred = fresh_credential(&state, id)
            .await
            .unwrap_or_else(|_| cred.clone());
        let hdrs = credential::build_headers(&cred);
        let result = billing::query_credits(&hdrs).await;
        let err = result
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let remain = result.get("credits_remaining").and_then(|v| v.as_i64());
        if err.is_none() {
            total += remain.unwrap_or(0);
        }
        out.push(serde_json::json!({
            "id": id,
            "uid": uid,
            "nickname": nickname,
            "credits_remaining": if err.is_none() { remain } else { None },
            "error": err,
        }));
    }
    Json(serde_json::json!({"credits_remaining": total, "accounts": out})).into_response()
}

async fn checkin_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_auth(&state, &headers) {
        return r;
    }
    let accounts: Vec<(String, String, String, CredentialData)> = {
        let cfg = state.config.read().await;
        cfg.accounts
            .iter()
            .filter(|a| a.enabled)
            .map(|a| {
                (
                    a.id.clone(),
                    a.credential.uid.clone(),
                    a.credential.nickname.clone(),
                    a.credential.clone(),
                )
            })
            .collect()
    };
    if accounts.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": {"message": "未配置账号", "type": "auth_error"}})),
        )
            .into_response();
    }
    let mut results: Vec<serde_json::Value> = Vec::new();
    for (id, uid, nickname, cred) in &accounts {
        let cred = fresh_credential(&state, id)
            .await
            .unwrap_or_else(|_| cred.clone());
        let hdrs = credential::build_headers(&cred);
        let result = billing::daily_checkin(&hdrs).await;
        results.push(serde_json::json!({
            "id": id,
            "uid": uid,
            "nickname": nickname,
            "ok": result.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
            "message": result.get("message").cloned().unwrap_or(serde_json::Value::Null),
        }));
    }
    let all_ok = results
        .iter()
        .all(|r| r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
    Json(serde_json::json!({"ok": all_ok, "results": results})).into_response()
}

async fn models_reload_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_auth(&state, &headers) {
        return r;
    }
    let first_id = {
        let cfg = state.config.read().await;
        cfg.accounts.iter().find(|a| a.enabled).map(|a| a.id.clone())
    };
    let st = state.clone();
    tokio::spawn(async move {
        if let Some(id) = first_id {
            if let Ok(cred) = fresh_credential(&st, &id).await {
                st.catalog.sync(&credential::build_headers(&cred)).await;
            }
        }
    });
    Json(serde_json::json!({"status": "reloading"})).into_response()
}

async fn agents_test_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if let Some(r) = require_auth(&state, &headers) {
        return r;
    }
    let prompt = body
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("ping");
    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("hy3");

    // Quick chat via upstream — 走多账号调度
    let mut chat_body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 32,
        "stream": true,
        "stream_options": {"include_usage": true}
    });
    ensure_system_first(&mut chat_body);
    desensitize::channel_desensitize(&mut chat_body);

    let resp = match dispatch(&state, &chat_body, 30).await {
        Ok(r) => r,
        Err(err_resp) => {
            let status = err_resp.status();
            let bytes = axum::body::to_bytes(err_resp.into_body(), 64 * 1024)
                .await
                .unwrap_or_default();
            let msg = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| {
                    v.pointer("/error/message")
                        .and_then(|m| m.as_str())
                        .map(String::from)
                })
                .unwrap_or_else(|| {
                    String::from_utf8_lossy(&bytes).chars().take(200).collect()
                });
            return Json(
                serde_json::json!({"ok": false, "http": status.as_u16(), "error": msg}),
            )
                .into_response();
        }
    };
    let bytes = resp.bytes().await.unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    // Aggregate SSE
    let mut content_parts: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("data:") {
            continue;
        }
        let data = line[5..].trim();
        if data == "[DONE]" {
            break;
        }
        if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) {
            for choice in chunk.get("choices").and_then(|v| v.as_array()).into_iter().flatten() {
                if let Some(c) = choice.get("delta").and_then(|d| d.get("content")).and_then(|v| v.as_str()) {
                    content_parts.push(c.to_string());
                }
            }
        }
    }
    let preview: String = content_parts.join("").chars().take(160).collect();
    Json(serde_json::json!({"ok": true, "model": model, "content_preview": preview})).into_response()
}

async fn ccswitch_register_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if let Some(r) = require_auth(&state, &headers) {
        return r;
    }
    let endpoint = body
        .get("endpoint")
        .and_then(|v| v.as_str())
        .unwrap_or("http://127.0.0.1:9178");
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("BuddyAIGateway");
    let model = body.get("model").and_then(|v| v.as_str());
    let api_key = {
        let cfg = state.config.read().await;
        body.get("api_key")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| cfg.api_key.clone())
    };
    let url = ccswitch::build_deeplink(endpoint, name, &api_key, model);
    let opened = if body.get("launch").and_then(|v| v.as_bool()).unwrap_or(false) {
        ccswitch::open_deeplink(&url)
    } else {
        false
    };
    Json(serde_json::json!({"ok": true, "url": url, "opened": opened, "model": model})).into_response()
}

/// HTTP 导入端点：新增/更新账号（uid 相同视为更新）
async fn config_import_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if let Some(r) = require_auth(&state, &headers) {
        return r;
    }
    // Accept either {json: "..."} or raw credential fields
    let json_str = if let Some(s) = body.get("json").and_then(|v| v.as_str()) {
        s.to_string()
    } else {
        body.to_string()
    };
    match credential::validate_import(&json_str) {
        Ok(cred) => {
            let mut cfg = state.config.write().await;
            let action = cfg.upsert_account(cred);
            let _ = cfg.save();
            drop(cfg);
            Json(serde_json::json!({"ok": true, "action": action})).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": {"message": e, "type": "invalid_request_error"}})),
        )
            .into_response(),
    }
}

async fn count_tokens_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_auth(&state, &headers) {
        return r;
    }
    Json(serde_json::json!({"input_tokens": 0})).into_response()
}

// ---------------------------------------------------------------------------
// POST /v1/messages — core streaming handler
// ---------------------------------------------------------------------------

fn dump_path() -> std::path::PathBuf {
    let dir = std::env::var("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    dir.join("buddyaigateway").join("last_outgoing.json")
}

async fn messages_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if let Some(r) = require_auth(&state, &headers) {
        return r;
    }

    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("hy3").to_string();

    let mut chat_body = anthropic_to_chat(&body);

    // 调试用：BUDDY_DUMP_BODY=1 时把出站体写入配置目录（默认关闭，不留 Prompt）。
    // 记录净化前（原始）与净化后（实际出站）两份，便于对比渠道层是否生效。
    if std::env::var("BUDDY_DUMP_BODY").as_deref() == Ok("1") {
        let dump = serde_json::to_string(&chat_body).unwrap_or_default();
        let _ = std::fs::write(dump_path(), dump);
    }

    // 上游 2026-09 起对请求内容做官方客户端指纹扫描（11128 Illegal API invocation），
    // 且 workbuddy.ai 后端要求首条消息必须是 system（11128 first message is not system
    // prompt）。渠道指纹中和 + system 兜底常开，与可选的隐私脱敏互不影响。
    ensure_system_first(&mut chat_body);
    desensitize::channel_desensitize(&mut chat_body);

    if std::env::var("BUDDY_DUMP_BODY").as_deref() == Ok("1") {
        let dump = serde_json::to_string(&chat_body).unwrap_or_default();
        let _ = std::fs::write(dump_path().with_file_name("last_outgoing_sanitized.json"), dump);
    }

    // Optional desensitize
    let do_desensitize = state.config.read().await.desensitize;
    if do_desensitize {
        chat_body = desensitize::desensitize_body(chat_body);
    }

    // Force stream
    chat_body["stream"] = serde_json::json!(true);
    if chat_body.get("stream_options").is_none() {
        chat_body["stream_options"] = serde_json::json!({"include_usage": true});
    }

    // 多账号故障转移：拿到 200 响应后才开始流式转发
    let resp = match dispatch(&state, &chat_body, 300).await {
        Ok(r) => r,
        Err(err_resp) => return err_resp,
    };

    // Stream: convert Chat SSE → Anthropic SSE
    let model_clone = model.clone();
    let stream = resp.bytes_stream();
    let converted = async_stream::stream! {
        let mut converter = AnthropicStreamConverter::new(&model_clone);
        let mut buf = bytes::BytesMut::new();
        let mut stream = Box::pin(stream);
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    buf.extend_from_slice(&bytes);
                    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        let line = buf.split_to(pos + 1);
                        let line_str = String::from_utf8_lossy(&line).to_string();
                        let events = converter.feed_line(&line_str);
                        if !events.is_empty() {
                            yield Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from(events));
                        }
                    }
                }
                Err(e) => {
                    let err_evt = format!("event: error\ndata: {{\"type\":\"error\",\"error\":{{\"message\":{},\"type\":\"api_error\"}}}}\n\n", serde_json::to_string(&e.to_string()).unwrap_or_default());
                    yield Ok(bytes::Bytes::from(err_evt));
                    break;
                }
            }
        }
        // Flush remaining buffer
        if !buf.is_empty() {
            let line_str = String::from_utf8_lossy(&buf).to_string();
            let events = converter.feed_line(&line_str);
            if !events.is_empty() {
                yield Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from(events));
            }
        }
        let fin = converter.finish();
        if !fin.is_empty() {
            yield Ok(bytes::Bytes::from(fin));
        }
    };

    let body = Body::from_stream(converted);
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("X-Accel-Buffering", "no")
        .body(body)
        .unwrap()
}

// ---------------------------------------------------------------------------
// Router builder
// ---------------------------------------------------------------------------

pub fn build_router(state: AppState) -> Router {
    // 仅放行本应用自身的 webview 源（dev: Vite 端口；prod: tauri 协议）。
    // Claude Code 等非浏览器客户端不携带 Origin，不受影响。
    let origins: Vec<HeaderValue> = ["http://localhost:1420", "http://127.0.0.1:1420", "http://localhost:5173", "http://127.0.0.1:5173", "http://tauri.localhost", "https://tauri.localhost", "tauri://localhost"]
        .iter()
        .map(|o| HeaderValue::from_static(o))
        .collect();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(AllowMethods::list([Method::GET, Method::POST, Method::OPTIONS]))
        .allow_headers(AllowHeaders::list([
            HeaderName::from_static("authorization"),
            HeaderName::from_static("x-api-key"),
            HeaderName::from_static("content-type"),
        ]))
        .max_age(std::time::Duration::from_secs(600));
    Router::new()
        .route("/health", get(health_handler))
        .route("/v1/models", get(models_handler))
        .route("/v1/messages", post(messages_handler))
        .route("/v1/messages/count_tokens", post(count_tokens_handler))
        .route("/credits", get(credits_handler))
        .route("/credits/checkin", post(checkin_handler))
        .route("/models/reload", post(models_reload_handler))
        .route("/agents/test", post(agents_test_handler))
        .route("/ccswitch/register", post(ccswitch_register_handler))
        .route("/config/import", post(config_import_handler))
        .layer(cors)
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Server lifecycle — bind 127.0.0.1:9178
// ---------------------------------------------------------------------------

pub async fn start_server(state: AppState) -> Result<(), String> {
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:9178")
        .await
        .map_err(|e| format!("端口 9178 被占用或无法绑定: {e}"))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
