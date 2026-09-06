pub mod account;
pub mod ai;
mod commands;
pub mod db;
pub mod error;
pub mod features;
mod forward;
pub mod models;
mod monitor;
mod recorder;
pub mod rewrap;
mod sftp;
pub mod ssh;
pub mod sync;
pub mod vault;

use std::sync::Arc;

use tauri::{Emitter, Manager};

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 原生对话框:WKWebView 的 window.confirm/alert 在本运行时是哑的(见 Cargo.toml 注释)
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("no app data dir");
            let db = Arc::new(db::Db::open(data_dir).expect("db open failed"));
            let vault = Arc::new(vault::Vault::open(&db.data_dir).expect("vault open failed"));
            // 存量数据若是用历史 device.key 加密的,这里统一换成当前设备密钥。
            // 迁移失败不该挡住启动 —— vault 解密本身已经会回退到历史密钥。
            if let Err(e) = rewrap::run(&db, &vault) {
                eprintln!("[rewrap] 设备密钥迁移未完成: {e}");
            }
            let sessions = Arc::new(ssh::SessionManager::default());
            let transfers = Arc::new(sftp::TransferManager::default());
            let forwards = Arc::new(forward::ForwardManager::default());
            let monitor_state = Arc::new(monitor::MonitorState::default());

            // setup 运行在主线程(无 tokio 上下文),内部的 tokio::spawn 须进入
            // tauri 的 tokio runtime 后再调用
            {
                let app_handle = app.handle().clone();
                let (db, vault, sessions, forwards, monitor_state) = (
                    db.clone(),
                    vault.clone(),
                    sessions.clone(),
                    forwards.clone(),
                    monitor_state.clone(),
                );
                tauri::async_runtime::spawn(async move {
                    monitor::spawn_monitor_loop(app_handle.clone(), sessions.clone(), monitor_state);
                    ssh::spawn_latency_loop(app_handle.clone(), db.clone());
                    forward::spawn_restore_enabled(app_handle, db, vault, sessions, forwards);
                });
            }

            app.manage(AppState {
                db,
                vault,
                sessions,
                transfers,
                forwards,
                monitor: monitor_state,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_groups,
            commands::create_group,
            commands::rename_group,
            commands::delete_group,
            commands::list_hosts,
            commands::save_host,
            commands::delete_host,
            commands::list_keys,
            commands::generate_key,
            commands::import_key,
            commands::get_public_key,
            commands::set_key_sync,
            commands::delete_key,
            commands::list_snippets,
            commands::save_snippet,
            commands::bump_snippet,
            commands::delete_snippet,
            commands::list_forwards,
            commands::create_forward,
            commands::toggle_forward,
            commands::delete_forward,
            commands::session_open,
            commands::trust_host_key,
            commands::session_write,
            commands::session_resize,
            commands::session_close,
            commands::sftp_list,
            commands::local_list,
            commands::local_home,
            commands::transfer_enqueue,
            commands::transfer_list,
            commands::transfer_retry,
            commands::recording_start,
            commands::recording_stop,
            commands::recording_list,
            commands::recording_read,
            commands::set_recording_sync,
            commands::monitor_samples,
            commands::sync_status,
            commands::sync_get_config,
            commands::sync_save_config,
            commands::sync_now,
            commands::sync_recovery_phrase,
            commands::sync_restore_master,
            commands::settings_get,
            commands::settings_save,
            commands::account_register,
            commands::features_get,
            commands::features_refresh,
            commands::account_register_policy,
            commands::account_login,
            commands::account_logout,
            commands::account_current,
            commands::account_verify_email,
            commands::account_resend_verification,
            commands::account_totp_setup,
            commands::account_totp_enable,
            commands::account_totp_disable,
            commands::account_refresh,
            commands::ai_get_settings,
            commands::ai_save_provider,
            commands::ai_delete_provider,
            commands::ai_set_active,
            commands::ai_ask,
            commands::restore_targets,
            commands::app_quit,
        ])
        .on_window_event(|window, event| {
            // 退出前确认:有活跃会话且设置开启时拦截关闭,交由前端确认后调 app_quit
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let confirm = state
                    .db
                    .kv_get("settings")
                    .ok()
                    .flatten()
                    .and_then(|j| serde_json::from_str::<models::AppSettings>(&j).ok())
                    .map(|s| s.confirm_on_quit)
                    .unwrap_or(false);
                let active = state
                    .sessions
                    .list()
                    .iter()
                    .any(|s| s.state == "connected");
                if confirm && active {
                    api.prevent_close();
                    let _ = window.emit("app://close-requested", state.sessions.list().len());
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
