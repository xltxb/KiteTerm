//! SSH 会话管理:russh 客户端、PTY shell、输出事件流、录制旁路。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use base64::Engine;
use russh::client::{self, Handle};
use russh::keys::PrivateKeyWithHashAlg;
use russh::ChannelMsg;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::{SessionInfo, TermData};
use crate::recorder::Recorder;
use crate::vault::{self, Vault};

pub struct ClientHandler;

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    // TODO: known_hosts 指纹校验(首次连接确认、变更告警)
    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        Ok(true)
    }
}

pub enum Ctl {
    Data(Vec<u8>),
    Resize(u16, u16),
    Close,
}

pub struct SessionEntry {
    pub info: SessionInfo,
    pub ctl: mpsc::UnboundedSender<Ctl>,
    pub handle: Arc<Handle<ClientHandler>>,
    pub recorder: Arc<Mutex<Option<Recorder>>>,
}

#[derive(Default)]
pub struct SessionManager {
    pub sessions: Mutex<HashMap<String, SessionEntry>>,
}

fn emit_state(app: &AppHandle, info: &SessionInfo) {
    let _ = app.emit("session://state", info.clone());
}

fn emit_data(app: &AppHandle, id: &str, data: &[u8]) {
    let payload = TermData {
        id: id.to_string(),
        data: base64::engine::general_purpose::STANDARD.encode(data),
    };
    let _ = app.emit("term://data", payload);
}

impl SessionManager {
    pub fn get_handle(&self, id: &str) -> Result<Arc<Handle<ClientHandler>>> {
        let sessions = self.sessions.lock().unwrap();
        sessions
            .get(id)
            .map(|e| e.handle.clone())
            .ok_or_else(|| AppError(format!("会话不存在: {id}")))
    }

    pub fn get_info(&self, id: &str) -> Result<SessionInfo> {
        let sessions = self.sessions.lock().unwrap();
        sessions
            .get(id)
            .map(|e| e.info.clone())
            .ok_or_else(|| AppError(format!("会话不存在: {id}")))
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        self.sessions.lock().unwrap().values().map(|e| e.info.clone()).collect()
    }
}

pub async fn connect_host(
    db: &Db,
    vault: &Vault,
    host_id: &str,
) -> Result<(Arc<Handle<ClientHandler>>, crate::models::Host, u32)> {
    let host = db.get_host(host_id)?;
    let key_id = host
        .key_id
        .clone()
        .ok_or_else(|| AppError(format!("主机 {} 未配置认证密钥", host.name)))?;
    let pem = String::from_utf8(vault.decrypt(&db.get_key_private_enc(&key_id)?)?)
        .map_err(|_| AppError("私钥解码失败".into()))?;
    let key = vault::parse_private(&pem)?;

    let config = Arc::new(client::Config {
        keepalive_interval: Some(std::time::Duration::from_secs(15)),
        ..Default::default()
    });

    let t0 = Instant::now();
    let mut handle = client::connect(config, (host.addr.as_str(), host.port), ClientHandler)
        .await
        .map_err(|e| AppError(format!("连接 {}:{} 失败: {e}", host.addr, host.port)))?;
    let latency = t0.elapsed().as_millis() as u32;

    let rsa_hash = handle
        .best_supported_rsa_hash()
        .await
        .map_err(|e| AppError(format!("协商失败: {e}")))?
        .flatten();
    let auth = handle
        .authenticate_publickey(
            host.user.clone(),
            PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash),
        )
        .await
        .map_err(|e| AppError(format!("认证失败: {e}")))?;
    if !auth.success() {
        return Err(AppError(format!("公钥认证被拒绝(user={})", host.user)));
    }
    db.touch_host_connected(host_id)?;
    Ok((Arc::new(handle), host, latency))
}

pub async fn open_shell_session(
    app: AppHandle,
    db: &Db,
    vault: &Vault,
    manager: &SessionManager,
    host_id: &str,
) -> Result<SessionInfo> {
    let (handle, host, latency) = connect_host(db, vault, host_id).await?;

    let id = uuid::Uuid::new_v4().to_string();
    let (cols, rows) = (120u16, 32u16);

    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| AppError(format!("打开通道失败: {e}")))?;
    channel
        .request_pty(false, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
        .await
        .map_err(|e| AppError(format!("请求 PTY 失败: {e}")))?;
    channel
        .request_shell(true)
        .await
        .map_err(|e| AppError(format!("启动 shell 失败: {e}")))?;

    let info = SessionInfo {
        id: id.clone(),
        host_id: host.id.clone(),
        host_name: host.name.clone(),
        user: host.user.clone(),
        addr: host.addr.clone(),
        port: host.port,
        state: "connected".into(),
        latency_ms: Some(latency),
        // russh 默认偏好套件;实际协商结果 russh 未暴露,此处展示偏好值
        cipher: "chacha20-poly1305".into(),
        cols,
        rows,
        recording: false,
        recording_since: None,
    };

    let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel::<Ctl>();
    let recorder: Arc<Mutex<Option<Recorder>>> = Arc::new(Mutex::new(None));

    {
        let mut sessions = manager.sessions.lock().unwrap();
        sessions.insert(
            id.clone(),
            SessionEntry {
                info: info.clone(),
                ctl: ctl_tx,
                handle: handle.clone(),
                recorder: recorder.clone(),
            },
        );
    }
    emit_state(&app, &info);

    // 会话主循环:通道输出 → 前端事件;控制消息 → 通道
    let task_app = app.clone();
    let task_id = id.clone();
    let task_recorder = recorder.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                msg = channel.wait() => {
                    match msg {
                        Some(ChannelMsg::Data { ref data }) => {
                            emit_data(&task_app, &task_id, data);
                            if let Some(rec) = task_recorder.lock().unwrap().as_mut() {
                                rec.write_output(data);
                            }
                        }
                        Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                            emit_data(&task_app, &task_id, data);
                        }
                        Some(ChannelMsg::ExitStatus { .. }) | Some(ChannelMsg::Close) | None => {
                            break;
                        }
                        _ => {}
                    }
                }
                ctl = ctl_rx.recv() => {
                    match ctl {
                        Some(Ctl::Data(bytes)) => {
                            if channel.data(&bytes[..]).await.is_err() { break; }
                        }
                        Some(Ctl::Resize(c, r)) => {
                            let _ = channel.window_change(c as u32, r as u32, 0, 0).await;
                        }
                        Some(Ctl::Close) | None => {
                            let _ = channel.eof().await;
                            break;
                        }
                    }
                }
            }
        }
        // 会话结束
        emit_data(&task_app, &task_id, b"\r\n\x1b[90m[\xe4\xbc\x9a\xe8\xaf\x9d\xe5\xb7\xb2\xe6\x96\xad\xe5\xbc\x80]\x1b[0m\r\n");
    });

    Ok(info)
}

pub fn write_session(manager: &SessionManager, id: &str, data: &str) -> Result<()> {
    let sessions = manager.sessions.lock().unwrap();
    let entry = sessions.get(id).ok_or_else(|| AppError(format!("会话不存在: {id}")))?;
    entry
        .ctl
        .send(Ctl::Data(data.as_bytes().to_vec()))
        .map_err(|_| AppError("会话已关闭".into()))?;
    // 录制输入不写入 asciicast(仅输出流),符合 v2 常规
    Ok(())
}

pub fn resize_session(manager: &SessionManager, id: &str, cols: u16, rows: u16) -> Result<()> {
    let mut sessions = manager.sessions.lock().unwrap();
    let entry = sessions.get_mut(id).ok_or_else(|| AppError(format!("会话不存在: {id}")))?;
    entry.info.cols = cols;
    entry.info.rows = rows;
    let _ = entry.ctl.send(Ctl::Resize(cols, rows));
    Ok(())
}

pub fn close_session(app: &AppHandle, manager: &SessionManager, id: &str) -> Result<()> {
    let mut sessions = manager.sessions.lock().unwrap();
    if let Some(entry) = sessions.remove(id) {
        let _ = entry.ctl.send(Ctl::Close);
        let mut info = entry.info;
        info.state = "disconnected".into();
        emit_state(app, &info);
    }
    Ok(())
}

/// 在既有连接上执行一条命令并收集 stdout(监控采集等用)
pub async fn exec_capture(handle: &Handle<ClientHandler>, cmd: &str) -> Result<String> {
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| AppError(format!("exec 通道失败: {e}")))?;
    channel
        .exec(true, cmd)
        .await
        .map_err(|e| AppError(format!("exec 失败: {e}")))?;
    let mut out = Vec::new();
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { ref data } => out.extend_from_slice(data),
            ChannelMsg::ExitStatus { .. } | ChannelMsg::Close => break,
            _ => {}
        }
    }
    Ok(String::from_utf8_lossy(&out).to_string())
}
