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
    pub certs: Arc<crate::acme::CertManager>,
}

// ---- hosts ----

#[tauri::command]
pub fn list_groups(state: State<AppState>) -> Result<Vec<HostGroup>> {
    state.db.list_groups()
}

#[tauri::command]
pub fn create_group(state: State<AppState>, name: String) -> Result<HostGroup> {
    state.db.create_group(&name)
}

#[tauri::command]
pub fn rename_group(state: State<AppState>, id: String, name: String) -> Result<()> {
    state.db.rename_group(&id, &name)
}

#[tauri::command]
pub fn delete_group(state: State<AppState>, id: String) -> Result<()> {
    state.db.delete_group(&id)
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
    let generated = vault::generate(&algo, &format!("{}@kite-term", whoami::username()))?;
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
    if enabled && !crate::features::cached(&state.db).key_sync {
        return Err(AppError("KEY_SYNC_DISABLED: 平台已禁止密钥同步,该密钥只能保留在本机".into()));
    }
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
        state.forwards.stop(&state.db, &id);
    }
    Ok(())
}

#[tauri::command]
pub fn delete_forward(state: State<AppState>, id: String) -> Result<()> {
    state.forwards.stop(&state.db, &id);
    state.db.delete_forward(&id)
}

// ---- sessions ----

#[tauri::command]
pub async fn session_open(
    app: AppHandle,
    state: State<'_, AppState>,
    host_id: String,
    password: Option<String>,
) -> Result<SessionInfo> {
    let info = ssh::open_shell_session(
        app,
        state.db.clone(),
        state.vault.clone(),
        state.sessions.clone(),
        state.transfers.clone(),
        &host_id,
        password,
    )
    .await?;
    persist_open_sessions(&state);
    Ok(info)
}

/// 打开本地终端(本机 PTY 跑默认 shell),不参与启动恢复
#[tauri::command]
pub fn local_session_open(app: AppHandle, state: State<AppState>) -> Result<SessionInfo> {
    crate::local::open_local_session(app, state.sessions.clone())
}

/// 主机密钥指纹变更后,用户确认信任:清除旧记录,下次连接 TOFU 重新记录
#[tauri::command]
pub fn trust_host_key(state: State<AppState>, host_id: String) -> Result<()> {
    state.db.forget_known_host(&host_id)
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
    ssh::close_session(&app, &state.sessions, &id)?;
    persist_open_sessions(&state);
    Ok(())
}

#[tauri::command]
pub async fn agent_exec(
    state: State<'_, AppState>,
    session_id: String,
    command: String,
    ai_readonly: bool,
    approved: bool,
) -> Result<crate::agent::AgentStepResult> {
    crate::agent::exec_step(&state.sessions, &session_id, &command, ai_readonly, approved).await
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

/// ZMODEM 上传:前端选好文件后回传路径(空列表表示用户取消),转成 Ctl::SendFiles 投递
#[tauri::command]
pub fn zmodem_send_files(state: State<AppState>, session_id: String, paths: Vec<String>) -> Result<()> {
    let files = paths.into_iter().map(std::path::PathBuf::from).collect();
    ssh::send_ctl(&state.sessions, &session_id, ssh::Ctl::SendFiles(files))
}

/// ZMODEM 下载:前端选好保存目录后回传(None 表示用户取消,后端会中止远端 sz)
#[tauri::command]
pub fn zmodem_receive_into(state: State<AppState>, session_id: String, dir: Option<String>) -> Result<()> {
    let dir = dir.filter(|d| !d.trim().is_empty()).map(std::path::PathBuf::from);
    ssh::send_ctl(&state.sessions, &session_id, ssh::Ctl::ReceiveInto(dir))
}

/// 拖文件到 SSH 终端:后端自动向远端敲 rz,并把这些文件作为发送列表(仿 Xshell)
#[tauri::command]
pub fn zmodem_drop_upload(state: State<AppState>, session_id: String, paths: Vec<String>) -> Result<()> {
    let files = paths.into_iter().map(std::path::PathBuf::from).collect();
    ssh::send_ctl(&state.sessions, &session_id, ssh::Ctl::DropUpload(files))
}

/// ZMODEM 取消:用户在传输浮条点取消,投递 Ctl::CancelTransfer 中止当前传输
#[tauri::command]
pub fn zmodem_cancel(state: State<AppState>, session_id: String) -> Result<()> {
    ssh::send_ctl(&state.sessions, &session_id, ssh::Ctl::CancelTransfer)
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

/// 读取 asciicast v2 原文供前端回放
#[tauri::command]
pub fn recording_read(state: State<AppState>, id: String) -> Result<String> {
    let path = state.db.get_recording_path(&id)?;
    Ok(std::fs::read_to_string(path)?)
}

#[tauri::command]
pub fn set_recording_sync(state: State<AppState>, id: String, enabled: bool) -> Result<()> {
    if enabled && !crate::features::cached(&state.db).recording_sync {
        return Err(AppError("RECORDING_SYNC_DISABLED: 平台已禁止录制同步".into()));
    }
    state.db.set_recording_sync(&id, enabled)
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

#[tauri::command]
pub fn sync_recovery_phrase(state: State<AppState>) -> Result<String> {
    sync::recovery_phrase(&state.db, &state.vault)
}

#[tauri::command]
pub fn sync_restore_master(state: State<AppState>, phrase: String) -> Result<()> {
    sync::restore_master_from_phrase(&state.db, &state.vault, &phrase)
}

// ---- 账户(管理平台)----

#[tauri::command]
pub async fn account_register(
    state: State<'_, AppState>,
    email: String,
    password: String,
    invite_code: Option<String>,
) -> Result<crate::account::AccountInfo> {
    crate::account::register(
        &state.db,
        &state.vault,
        &email,
        &password,
        invite_code.as_deref(),
    )
    .await
}

/// 读缓存,不发网络请求 —— 界面首屏用它,避免等平台响应
#[tauri::command]
pub fn features_get(state: State<AppState>) -> Result<crate::features::Features> {
    Ok(crate::features::cached(&state.db))
}

/// 向平台拉一次并刷新缓存;平台不可达时沿用缓存
#[tauri::command]
pub async fn features_refresh(state: State<'_, AppState>) -> Result<crate::features::Features> {
    crate::features::refresh(&state.db).await
}

#[tauri::command]
pub async fn account_register_policy() -> Result<crate::account::RegisterPolicy> {
    crate::account::register_policy().await
}

#[tauri::command]
pub async fn account_login(
    state: State<'_, AppState>,
    email: String,
    password: String,
    totp: Option<String>,
) -> Result<crate::account::AccountInfo> {
    crate::account::login(&state.db, &state.vault, &email, &password, totp.as_deref()).await
}

#[tauri::command]
pub async fn account_logout(state: State<'_, AppState>) -> Result<()> {
    crate::account::logout(&state.db, &state.vault).await
}

#[tauri::command]
pub fn account_current(state: State<AppState>) -> Result<Option<crate::account::AccountInfo>> {
    crate::account::current(&state.db)
}

#[tauri::command]
pub async fn account_verify_email(
    state: State<'_, AppState>,
    code: String,
) -> Result<crate::account::AccountInfo> {
    crate::account::verify_email(&state.db, &state.vault, &code).await
}

#[tauri::command]
pub async fn account_resend_verification(state: State<'_, AppState>) -> Result<()> {
    crate::account::resend_verification(&state.db, &state.vault).await
}

#[tauri::command]
pub async fn account_totp_setup(state: State<'_, AppState>) -> Result<crate::account::TotpSetup> {
    crate::account::totp_setup(&state.db, &state.vault).await
}

#[tauri::command]
pub async fn account_totp_enable(
    state: State<'_, AppState>,
    code: String,
) -> Result<crate::account::AccountInfo> {
    crate::account::totp_enable(&state.db, &state.vault, &code).await
}

#[tauri::command]
pub async fn account_totp_disable(
    state: State<'_, AppState>,
    code: String,
) -> Result<crate::account::AccountInfo> {
    crate::account::totp_disable(&state.db, &state.vault, &code).await
}

#[tauri::command]
pub async fn account_refresh(state: State<'_, AppState>) -> Result<crate::account::AccountInfo> {
    crate::account::refresh(&state.db, &state.vault).await
}

// ---- AI 助手 ----

#[tauri::command]
pub fn ai_get_settings(state: State<AppState>) -> Result<crate::ai::AiSettings> {
    crate::ai::get_settings(&state.db)
}

#[tauri::command]
pub fn ai_save_provider(
    state: State<AppState>,
    provider: crate::ai::AiProvider,
    api_key: Option<String>,
) -> Result<crate::ai::AiSettings> {
    crate::ai::save_provider(&state.db, &state.vault, provider, api_key)
}

#[tauri::command]
pub fn ai_delete_provider(state: State<AppState>, id: String) -> Result<crate::ai::AiSettings> {
    crate::ai::delete_provider(&state.db, &id)
}

#[tauri::command]
pub fn ai_set_active(state: State<AppState>, id: String) -> Result<crate::ai::AiSettings> {
    crate::ai::set_active(&state.db, &id)
}

/// mode: chat(侧栏对话)| explain(解释终端输出)| command(自然语言转命令)| summary(长输出摘要)
#[tauri::command]
pub async fn ai_ask(
    state: State<'_, AppState>,
    messages: Vec<crate::ai::ChatMessage>,
    terminal_context: String,
    mode: Option<String>,
) -> Result<String> {
    crate::ai::ask(
        &state.db,
        &state.vault,
        messages,
        terminal_context,
        mode.as_deref().unwrap_or("chat"),
    )
    .await
}

// ---- 会话恢复 ----

/// 把当前打开会话的主机列表落库(启动恢复用)
pub fn persist_open_sessions(state: &AppState) {
    let hosts: Vec<String> = state
        .sessions
        .list()
        .into_iter()
        .filter(|s| s.kind == "ssh")
        .map(|s| s.host_id)
        .collect();
    let _ = state
        .db
        .kv_set("open_hosts", &serde_json::to_string(&hosts).unwrap_or_default());
}

/// 启动时应恢复的主机 ID 列表(仅当设置开启)
#[tauri::command]
pub fn restore_targets(state: State<AppState>) -> Result<Vec<String>> {
    let settings = settings_get(state.clone())?;
    if !settings.restore_sessions {
        return Ok(vec![]);
    }
    Ok(state
        .db
        .kv_get("open_hosts")?
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default())
}

#[tauri::command]
pub fn app_quit(app: AppHandle) {
    app.exit(0);
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
    let json = serde_json::to_string(&settings)?;
    state.db.kv_set("settings", &json)?;
    state.db.oplog_append("settings", "app", "put", Some(json))?;
    Ok(())
}

// ---- certs ----

#[tauri::command]
pub fn cert_list(state: State<AppState>) -> Result<Vec<CertInfo>> {
    state.db.list_certs()
}

#[tauri::command]
pub async fn cert_issue(app: AppHandle, state: State<'_, AppState>, req: IssueRequest) -> Result<CertInfo> {
    crate::acme::issue(app, state.db.clone(), state.vault.clone(), state.sessions.clone(), state.certs.clone(), req).await
}

#[tauri::command]
pub fn cert_dns_confirm(state: State<AppState>, id: String) -> Result<()> {
    crate::acme::confirm_dns(&state.certs, &id);
    Ok(())
}

#[tauri::command]
pub async fn cert_renew(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<CertInfo> {
    crate::acme::renew(app, state.db.clone(), state.vault.clone(), state.sessions.clone(), state.certs.clone(), &id).await
}

#[tauri::command]
pub async fn cert_revoke(state: State<'_, AppState>, id: String) -> Result<()> {
    crate::acme::revoke(state.db.clone(), state.vault.clone(), &id).await
}

#[tauri::command]
pub async fn cert_deploy(app: AppHandle, state: State<'_, AppState>, id: String, host_id: String) -> Result<()> {
    // 注:deploy 比 brief 里的签名多一个 vault 参数 —— 私钥经 vault 加密存储,部署时需解密取私钥
    crate::acme::deploy(app, state.db.clone(), state.vault.clone(), state.sessions.clone(), &id, &host_id).await
}

#[tauri::command]
pub fn cert_download_chain(state: State<AppState>, id: String) -> Result<String> {
    let row = state.db.get_cert_full(&id)?;
    row.chain_pem.ok_or_else(|| AppError("证书链不存在".into()))
}
