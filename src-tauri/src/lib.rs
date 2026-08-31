// BuddyAIGateway — Tauri backend

mod anthropic;
mod billing;
mod catalog;
mod ccswitch;
mod config;
mod credential;
mod desensitize;
mod server;
mod tray;

use config::AppConfig;
use server::AppState;
use tauri::Manager;

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_config(state: tauri::State<'_, AppState>) -> serde_json::Value {
    match state.config.try_read() {
        Ok(g) => g.redacted(),
        Err(_) => serde_json::json!({}),
    }
}

#[tauri::command]
async fn get_credential_status(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    match state.credential.get().await {
        Some(c) => Ok(serde_json::json!({
            "configured": true,
            "uid": c.uid,
            "nickname": c.nickname,
            "domain": c.domain,
            "expires_at": c.expires_at,
        })),
        None => Ok(serde_json::json!({"configured": false})),
    }
}

#[tauri::command]
async fn import_credential(
    state: tauri::State<'_, AppState>,
    json_str: String,
) -> Result<String, String> {
    let cred = credential::validate_import(&json_str)?;
    state.credential.set(cred.clone()).await;
    let mut cfg = state.config.write().await;
    cfg.credential = Some(cred);
    cfg.save().map_err(|e| e.to_string())?;
    Ok("ok".into())
}

#[tauri::command]
async fn clear_credential(state: tauri::State<'_, AppState>) -> Result<String, String> {
    state.credential.clear().await;
    let mut cfg = state.config.write().await;
    cfg.credential = None;
    cfg.save().map_err(|e| e.to_string())?;
    Ok("ok".into())
}

#[tauri::command]
async fn get_api_key(state: tauri::State<'_, AppState>) -> Result<String, String> {
    Ok(state.config.read().await.api_key.clone())
}

#[tauri::command]
async fn toggle_desensitize(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<String, String> {
    let mut cfg = state.config.write().await;
    cfg.desensitize = enabled;
    cfg.save().map_err(|e| e.to_string())?;
    Ok("ok".into())
}

#[tauri::command]
fn build_ccswitch_link(
    endpoint: String,
    name: String,
    api_key: String,
    model: Option<String>,
) -> String {
    ccswitch::build_deeplink(&endpoint, &name, &api_key, model.as_deref())
}

#[tauri::command]
fn open_ccswitch_link(url: String) -> bool {
    ccswitch::open_deeplink(&url)
}

#[tauri::command]
fn get_version() -> String {
    "1.0.0".into()
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut cfg = AppConfig::load();
    cfg.ensure_api_key();
    let app_state = AppState::new(cfg);

    tauri::Builder::default()
        // 必须最先注册：二次启动时唤起已有窗口，而不是开第二个网关进程
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .setup(|app| {
            tray::setup(app)?;
            let state = app.state::<AppState>().inner().clone();
            // Spawn Axum gateway
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server::start_server(state.clone()).await {
                    eprintln!("[gateway] server error: {e}");
                }
            });
            // Spawn catalog sync
            let catalog_state = app.state::<AppState>().inner().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if let Some(hdrs) = catalog_state.credential.build_headers().await {
                    catalog_state.catalog.sync(&hdrs).await;
                }
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    if let Some(hdrs) = catalog_state.credential.build_headers().await {
                        catalog_state.catalog.sync(&hdrs).await;
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 关窗 = 隐藏到托盘，网关继续服务；退出走托盘菜单
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            get_credential_status,
            import_credential,
            clear_credential,
            get_api_key,
            toggle_desensitize,
            build_ccswitch_link,
            open_ccswitch_link,
            get_version
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
