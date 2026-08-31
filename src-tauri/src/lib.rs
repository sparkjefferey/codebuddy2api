// BuddyAIGateway — Tauri backend

mod anthropic;
mod billing;
mod catalog;
mod ccswitch;
mod config;
mod credential;
mod desensitize;
mod pool;
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
async fn get_credential_status(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let cfg = state.config.read().await;
    let snap = state.pool.snapshot();
    let accounts: Vec<serde_json::Value> = cfg
        .accounts
        .iter()
        .map(|a| server::account_json(a, snap.get(&a.id)))
        .collect();
    Ok(serde_json::json!({
        "configured": !cfg.accounts.is_empty(),
        "accounts": accounts,
    }))
}

/// 导入账号：uid 相同则更新该账号凭据，否则新增
#[tauri::command]
async fn import_credential(
    state: tauri::State<'_, AppState>,
    json_str: String,
) -> Result<String, String> {
    let cred = credential::validate_import(&json_str)?;
    let mut cfg = state.config.write().await;
    let action = cfg.upsert_account(cred);
    cfg.save().map_err(|e| e.to_string())?;
    Ok(action.into())
}

#[tauri::command]
async fn remove_account(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    let mut cfg = state.config.write().await;
    if !cfg.remove_account(&id) {
        return Err("账号不存在".into());
    }
    cfg.save().map_err(|e| e.to_string())?;
    Ok("ok".into())
}

#[tauri::command]
async fn set_account_enabled(
    state: tauri::State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<String, String> {
    let mut cfg = state.config.write().await;
    if !cfg.set_account_enabled(&id, enabled) {
        return Err("账号不存在".into());
    }
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
    "1.1.0".into()
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
            // Spawn catalog sync — 取第一个启用账号拉取模型目录
            let catalog_state = app.state::<AppState>().inner().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                loop {
                    let first_id = {
                        let cfg = catalog_state.config.read().await;
                        cfg.accounts
                            .iter()
                            .find(|a| a.enabled)
                            .map(|a| a.id.clone())
                    };
                    if let Some(id) = first_id {
                        if let Ok(cred) = server::fresh_credential(&catalog_state, &id).await {
                            catalog_state
                                .catalog
                                .sync(&credential::build_headers(&cred))
                                .await;
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
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
            remove_account,
            set_account_enabled,
            get_api_key,
            toggle_desensitize,
            build_ccswitch_link,
            open_ccswitch_link,
            get_version
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
