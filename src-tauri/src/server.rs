//! Axum HTTP gateway — mirrors converter.py routes, single CN account.
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

use crate::anthropic::{anthropic_to_chat, AnthropicStreamConverter};
use crate::billing;
use crate::catalog::ModelCatalog;
use crate::config::AppConfig;
use crate::credential::{validate_import, CredentialManager};
use crate::ccswitch;
use crate::desensitize;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub credential: CredentialManager,
    pub catalog: Arc<ModelCatalog>,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        let cred_data = config.credential.clone();
        Self {
            config: Arc::new(RwLock::new(config)),
            credential: CredentialManager::new(cred_data),
            catalog: Arc::new(ModelCatalog::default()),
        }
    }
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
// Handlers
// ---------------------------------------------------------------------------

async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let cfg = state.config.read().await;
    let cred = state.credential.get().await;
    let status = if cred.is_some() { "ok" } else { "degraded" };
    let mut info = serde_json::json!({
        "status": status,
        "version": "1.0.0",
        "mode": "buddyaigateway",
        "config": cfg.redacted(),
    });
    if let Some(c) = cred {
        info["account"] = serde_json::json!({
            "uid": c.uid,
            "nickname": c.nickname,
            "domain": c.domain,
        });
    }
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
    let hdrs = match state.credential.build_headers().await {
        Some(h) => h,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": {"message": "未配置账号", "type": "auth_error"}})),
            )
                .into_response()
        }
    };
    let result = billing::query_credits(&hdrs).await;
    Json(serde_json::json!({"account": result})).into_response()
}

async fn checkin_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_auth(&state, &headers) {
        return r;
    }
    let hdrs = match state.credential.build_headers().await {
        Some(h) => h,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": {"message": "未配置账号", "type": "auth_error"}})),
            )
                .into_response()
        }
    };
    let result = billing::daily_checkin(&hdrs).await;
    Json(serde_json::json!({"account": result})).into_response()
}

async fn models_reload_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_auth(&state, &headers) {
        return r;
    }
    let hdrs = state.credential.build_headers().await;
    let catalog = state.catalog.clone();
    tokio::spawn(async move {
        if let Some(h) = hdrs {
            catalog.sync(&h).await;
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
    let hdrs = match state.credential.build_headers().await {
        Some(h) => h,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": {"message": "未配置账号", "type": "auth_error"}})),
            )
                .into_response()
        }
    };
    let prompt = body
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("ping");
    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("hy3");

    // Quick chat via upstream
    let chat_body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 32,
        "stream": true,
        "stream_options": {"include_usage": true}
    });

    let base = state.credential.backend_base().await;
    let url = format!("{base}/v2/chat/completions");
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok": false, "error": e.to_string()})),
            )
                .into_response()
        }
    };
    let mut req = client.post(&url).json(&chat_body);
    for (k, v) in &hdrs {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return Json(serde_json::json!({"ok": false, "error": e.to_string()})).into_response()
        }
    };
    if resp.status() != reqwest::StatusCode::OK {
        let text = resp.text().await.unwrap_or_default();
        return Json(serde_json::json!({"ok": false, "http": 502, "error": text.chars().take(300).collect::<String>()}))
            .into_response();
    }
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
    match validate_import(&json_str) {
        Ok(cred) => {
            state.credential.set(cred.clone()).await;
            let mut cfg = state.config.write().await;
            cfg.credential = Some(cred);
            let _ = cfg.save();
            Json(serde_json::json!({"ok": true})).into_response()
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

async fn messages_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if let Some(r) = require_auth(&state, &headers) {
        return r;
    }

    let cred_data = state.credential.get().await;
    if cred_data.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": {"message": "未配置账号，请先导入凭据", "type": "auth_error"}})),
        )
            .into_response();
    }

    // Ensure token fresh
    if let Err(e) = state.credential.ensure_fresh().await {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": {"message": format!("token 刷新失败: {e}"), "type": "auth_error"}})),
        )
            .into_response();
    }

    let hdrs = match state.credential.build_headers().await {
        Some(h) => h,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": {"message": "无可用凭据", "type": "auth_error"}})),
            )
                .into_response()
        }
    };

    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("hy3").to_string();

    let mut chat_body = anthropic_to_chat(&body);

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

    let base = state.credential.backend_base().await;
    let url = format!("{base}/v2/chat/completions");

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": {"message": e.to_string(), "type": "api_error"}})),
            )
                .into_response()
        }
    };

    let mut req = client.post(&url).json(&chat_body);
    for (k, v) in &hdrs {
        req = req.header(k.as_str(), v.as_str());
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": {"message": e.to_string(), "type": "api_error"}})),
            )
                .into_response()
        }
    };

    if resp.status() != reqwest::StatusCode::OK {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let err_body: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({"error": {"message": text.chars().take(500).collect::<String>(), "type": "upstream_error", "code": status}}));
        return (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), Json(err_body)).into_response();
    }

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
