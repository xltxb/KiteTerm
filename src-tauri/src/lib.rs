mod commands;
mod db;
mod error;
mod forward;
mod models;
mod monitor;
mod recorder;
mod sftp;
mod ssh;
mod sync;
mod vault;

use std::sync::Arc;

use tauri::Manager;

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("no app data dir");
            let db = Arc::new(db::Db::open(data_dir).expect("db open failed"));
            let vault = Arc::new(vault::Vault::open(&db.data_dir).expect("vault open failed"));
            let sessions = Arc::new(ssh::SessionManager::default());
            let transfers = Arc::new(sftp::TransferManager::default());
            let forwards = Arc::new(forward::ForwardManager::default());
            let monitor_state = Arc::new(monitor::MonitorState::default());

            monitor::spawn_monitor_loop(app.handle().clone(), sessions.clone(), monitor_state.clone());
            ssh::spawn_latency_loop(app.handle().clone(), db.clone());
            forward::spawn_restore_enabled(
                app.handle().clone(),
                db.clone(),
                vault.clone(),
                sessions.clone(),
                forwards.clone(),
            );

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
            commands::monitor_samples,
            commands::sync_status,
            commands::sync_get_config,
            commands::sync_save_config,
            commands::sync_now,
            commands::settings_get,
            commands::settings_save,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
