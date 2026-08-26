use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::forward::{self, ForwardManager};
use crate::models::*;
use crate::monitor::MonitorState;
use crate::recorder::Recorder;
use crate::sftp::{self, TransferManager};
use crate::ssh::{self, SessionManager};
use crate::sync;
use crate::vault::{self, Vault};

pub struct AppState {
    pub db: Arc<Db>,
    pub vault: Arc<Vault>,
    pub sessions: Arc<SessionManager>,
    pub transfers: Arc<TransferManager>,
    pub forwards: Arc<ForwardManager>,
    pub monitor: Arc<MonitorState>,
}

// ---- hosts ----

#[tauri::command]
pub fn list_groups(state: State<AppState>) -> Result<Vec<HostGroup>> {
    state.db.list_groups()
}

#[tauri::command]
pub fn list_hosts(state: State<AppState>) -> Result<Vec<Host>> {
    let mut hosts = state.db.list_hosts()?;
    // 用活跃会话的实测延迟填充
    let sessions = state.sessions.list();
    for h in &mut hosts {
        if let Some(s) = sessions.iter().find(|s| s.host_id == h.id && s.state == "connected") {
            h.latency_ms = s.latency_ms;
        }
    }
    Ok(hosts)
}

#[tauri::command]
pub fn save_host(state: State<AppState>, host: HostInput) -> Result<Host> {
    state.db.save_host(host)
}

#[tauri::command]
pub fn delete_host(state: State<AppState>, id: String) -> Result<()> {
    state.db.delete_host(&id)
}

// ---- keys ----

#[tauri::command]
pub fn list_keys(state: State<AppState>) -> Result<Vec<SshKeyMeta>> {
    state.db.list_keys()
}

#[tauri::command]
pub fn generate_key(state: State<AppState>, name: String, algo: String) -> Result<SshKeyMeta> {
    let generated = vault::generate(&algo, &format!("{}@vela-term", whoami::username()))?;
    let enc = state.vault.encrypt(generated.private_pem.as_bytes())?;
    let id = state.db.insert_key(&name, &generated.algo, &generated.fingerprint, &generated.public_openssh, &enc)?;
    state
        .db
        .list_keys()?
        .into_iter()
        .find(|k| k.id == id)
        .ok_or_else(|| AppError("生成后读取失败".into()))
}

#[tauri::command]
pub fn import_key(state: State<AppState>, name: String, pem: String) -> Result<SshKeyMeta> {
    let imported = vault::import(&pem)?;
    let enc = state.vault.encrypt(imported.private_pem.as_bytes())?;
    let id = state.db.insert_key(&name, &imported.algo, &imported.fingerprint, &imported.public_openssh, &enc)?;
    state
        .db
        .list_keys()?
        .into_iter()
        .find(|k| k.id == id)
        .ok_or_else(|| AppError("导入后读取失败".into()))
}

#[tauri::command]
pub fn get_public_key(state: State<AppState>, id: String) -> Result<String> {
    state.db.get_key_public(&id)
}

#[tauri::command]
pub fn set_key_sync(state: State<AppState>, id: String, enabled: bool) -> Result<()> {
    state.db.set_key_sync(&id, enabled)
}

#[tauri::command]
pub fn delete_key(state: State<AppState>, id: String) -> Result<()> {
    state.db.delete_key(&id)
}

// ---- snippets ----

#[tauri::command]
pub fn list_snippets(state: State<AppState>) -> Result<Vec<Snippet>> {
    state.db.list_snippets()
}

#[tauri::command]
pub fn save_snippet(state: State<AppState>, snippet: SnippetInput) -> Result<Snippet> {
    state.db.save_snippet(snippet)
}

#[tauri::command]
pub fn bump_snippet(state: State<AppState>, id: String) -> Result<()> {
    state.db.bump_snippet(&id)
}

#[tauri::command]
pub fn delete_snippet(state: State<AppState>, id: String) -> Result<()> {
    state.db.delete_snippet(&id)
}

// ---- forwards ----

#[tauri::command]
pub fn list_forwards(state: State<AppState>) -> Result<Vec<ForwardRule>> {
    state.db.list_forwards()
}

#[tauri::command]
pub async fn create_forward(
    app: AppHandle,
    state: State<'_, AppState>,
    rule: ForwardInput,
) -> Result<ForwardRule> {
    let saved = state.db.insert_forward(&rule)?;
    forward::start_forward(
        app,
        state.db.clone(),
        state.vault.clone(),
        state.sessions.clone(),
        state.forwards.clone(),
        saved.clone(),
    )
    .await?;
    Ok(saved)
}

#[tauri::command]
pub async fn toggle_forward(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<()> {
    state.db.set_forward_enabled(&id, enabled)?;
    if enabled {
        let rule = state
            .db
            .list_forwards()?
            .into_iter()
            .find(|f| f.id == id)
            .ok_or_else(|| AppError("规则不存在".into()))?;
        forward::start_forward(
            app,
            state.db.clone(),
            state.vault.clone(),
            state.sessions.clone(),
            state.forwards.clone(),
            rule,
        )
        .await?;
    } else {
        state.forwards.stop(&id);
    }
    Ok(())
}

#[tauri::command]
pub fn delete_forward(state: State<AppState>, id: String) -> Result<()> {
    state.forwards.stop(&id);
    state.db.delete_forward(&id)
}

// ---- sessions ----

#[tauri::command]
pub async fn session_open(
    app: AppHandle,
    state: State<'_, AppState>,
    host_id: String,
) -> Result<SessionInfo> {
    ssh::open_shell_session(app, &state.db, &state.vault, &state.sessions, &host_id).await
}

#[tauri::command]
pub fn session_write(state: State<AppState>, id: String, data: String) -> Result<()> {
    ssh::write_session(&state.sessions, &id, &data)
}

#[tauri::command]
pub fn session_resize(state: State<AppState>, id: String, cols: u16, rows: u16) -> Result<()> {
    ssh::resize_session(&state.sessions, &id, cols, rows)
}

#[tauri::command]
pub fn session_close(app: AppHandle, state: State<AppState>, id: String) -> Result<()> {
    ssh::close_session(&app, &state.sessions, &id)
}

// ---- sftp / transfer ----

#[tauri::command]
pub async fn sftp_list(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> Result<Vec<FileEntry>> {
    sftp::sftp_list(&state.sessions, &session_id, &path).await
}

#[tauri::command]
pub fn local_list(path: String) -> Result<Vec<FileEntry>> {
    sftp::local_list(&path)
}

#[tauri::command]
pub fn local_home() -> Result<String> {
    Ok(dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".into()))
}

#[tauri::command]
pub fn transfer_enqueue(
    app: AppHandle,
    state: State<AppState>,
    req: TransferRequest,
) -> Result<TransferJob> {
    sftp::enqueue(app, state.transfers.clone(), state.sessions.clone(), req)
}

#[tauri::command]
pub fn transfer_list(state: State<AppState>) -> Result<Vec<TransferJob>> {
    Ok(state.transfers.jobs.lock().unwrap().clone())
}

#[tauri::command]
pub fn transfer_retry(app: AppHandle, state: State<AppState>, id: String) -> Result<()> {
    sftp::retry(app, state.transfers.clone(), state.sessions.clone(), &id);
    Ok(())
}

// ---- recording ----

#[tauri::command]
pub fn recording_start(app: AppHandle, state: State<AppState>, session_id: String) -> Result<()> {
    let mut sessions = state.sessions.sessions.lock().unwrap();
    let entry = sessions
        .get_mut(&session_id)
        .ok_or_else(|| AppError("会话不存在".into()))?;
    let dir = state.db.data_dir.join("recordings");
    let rec = Recorder::start(&dir, &entry.info.host_name, entry.info.cols, entry.info.rows)?;
    entry.info.recording = true;
    entry.info.recording_since = Some(rec.started_at_ms);
    *entry.recorder.lock().unwrap() = Some(rec);
    let _ = app.emit("session://state", entry.info.clone());
    Ok(())
}

#[tauri::command]
pub fn recording_stop(app: AppHandle, state: State<AppState>, session_id: String) -> Result<Recording> {
    let mut sessions = state.sessions.sessions.lock().unwrap();
    let entry = sessions
        .get_mut(&session_id)
        .ok_or_else(|| AppError("会话不存在".into()))?;
    let rec = entry
        .recorder
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| AppError("该会话未在录制".into()))?;
    entry.info.recording = false;
    entry.info.recording_since = None;
    let _ = app.emit("session://state", entry.info.clone());
    let duration = rec.duration_ms();
    let name = format!("{} 会话录制", entry.info.host_name);
    let id = state.db.insert_recording(
        &entry.info.host_name,
        &name,
        &rec.path.to_string_lossy(),
        duration,
    )?;
    state
        .db
        .list_recordings()?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| AppError("录制记录读取失败".into()))
}

#[tauri::command]
pub fn recording_list(state: State<AppState>) -> Result<Vec<Recording>> {
    state.db.list_recordings()
}

// ---- monitor ----

#[tauri::command]
pub fn monitor_samples(state: State<AppState>) -> Result<Vec<MonitorSample>> {
    Ok(state.monitor.samples.lock().unwrap().clone())
}

// ---- sync ----

#[tauri::command]
pub fn sync_status(state: State<AppState>) -> Result<SyncStatus> {
    sync::get_status(&state.db)
}

#[tauri::command]
pub fn sync_get_config(state: State<AppState>) -> Result<SyncConfig> {
    sync::get_config(&state.db)
}

#[tauri::command]
pub fn sync_save_config(state: State<AppState>, cfg: SyncConfig) -> Result<()> {
    sync::save_config(&state.db, &state.vault, cfg)
}

#[tauri::command]
pub async fn sync_now(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    sync::sync_now(app, state.db.clone(), state.vault.clone()).await
}

// ---- settings ----

#[tauri::command]
pub fn settings_get(state: State<AppState>) -> Result<AppSettings> {
    match state.db.kv_get("settings")? {
        Some(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
        None => Ok(AppSettings::default()),
    }
}

#[tauri::command]
pub fn settings_save(state: State<AppState>, settings: AppSettings) -> Result<()> {
    state.db.kv_set("settings", &serde_json::to_string(&settings)?)
}
