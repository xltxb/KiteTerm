//! SSH 会话管理:russh 客户端、PTY shell、输出事件流、known_hosts 校验(TOFU)、
//! 密钥/密码认证、断线自动重连(指数退避 1/2/4/8/8s,上限 5 次)、录制旁路。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use russh::client::{self, Handle};
use russh::keys::ssh_key::HashAlg;
use russh::keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{Channel, ChannelMsg};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::{AppSettings, SessionInfo, TermData};
use crate::recorder::Recorder;
use crate::vault::{self, Vault};

/// R 型远程转发的投递目标(远端监听端口 → 本地 host:port)
pub struct RTarget {
    pub host: String,
    pub port: u16,
    pub up: Arc<AtomicU64>,
    pub down: Arc<AtomicU64>,
}

pub type RForwardMap = Arc<Mutex<HashMap<u32, RTarget>>>;

pub struct ClientHandler {
    db: Arc<Db>,
    host_id: String,
    /// 指纹不匹配时记录服务器本次出示的指纹,供上层生成告警信息
    presented_mismatch: Arc<Mutex<Option<String>>>,
    /// 远端端口 → R 转发目标;由 forward 模块在 tcpip_forward 前登记
    r_targets: RForwardMap,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    /// known_hosts 校验(TOFU):首次连接记录指纹;之后不一致即拒绝连接
    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let fp = match server_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => {
                key.fingerprint(HashAlg::Sha256).to_string()
            }
            PublicKeyOrCertificate::Certificate(cert) => {
                cert.public_key().fingerprint(HashAlg::Sha256).to_string()
            }
        };
        match self.db.known_host_fingerprint(&self.host_id) {
            Ok(None) => {
                let _ = self.db.record_known_host(&self.host_id, &fp);
                Ok(true)
            }
            Ok(Some(known)) if known == fp => Ok(true),
            Ok(Some(_)) => {
                *self.presented_mismatch.lock().unwrap() = Some(fp);
                Ok(false)
            }
            Err(_) => Ok(false),
        }
    }

    /// R 型远程转发:服务器上有连接进入被监听端口时回调,接通到登记的本地目标
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: russh::client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> std::result::Result<(), Self::Error> {
        let target = {
            let map = self.r_targets.lock().unwrap();
            map.get(&connected_port)
                .map(|t| (t.host.clone(), t.port, t.up.clone(), t.down.clone()))
        };
        let Some((host, port, up, down)) = target else {
            drop(reply); // 未登记的端口:拒绝
            return Ok(());
        };
        reply.accept().await;
        tokio::spawn(async move {
            match tokio::net::TcpStream::connect((host.as_str(), port)).await {
                Ok(local) => {
                    let _ = pipe_counted(channel.into_stream(), local, up, down).await;
                }
                Err(e) => eprintln!("[forward:R] 连接本地目标 {host}:{port} 失败: {e}"),
            }
        });
        Ok(())
    }
}

/// 双向搬运并计数:a 为远端通道,b 为本地连接;down = 远端→本地,up = 本地→远端
async fn pipe_counted(
    a: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    b: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    up: Arc<AtomicU64>,
    down: Arc<AtomicU64>,
) -> std::io::Result<()> {
    let (mut ar, mut aw) = tokio::io::split(a);
    let (mut br, mut bw) = tokio::io::split(b);
    let a2b = async {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            let n = ar.read(&mut buf).await?;
            if n == 0 { break; }
            bw.write_all(&buf[..n]).await?;
            down.fetch_add(n as u64, Ordering::Relaxed);
        }
        bw.shutdown().await.ok();
        Ok::<_, std::io::Error>(())
    };
    let b2a = async {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            let n = br.read(&mut buf).await?;
            if n == 0 { break; }
            aw.write_all(&buf[..n]).await?;
            up.fetch_add(n as u64, Ordering::Relaxed);
        }
        aw.shutdown().await.ok();
        Ok::<_, std::io::Error>(())
    };
    let _ = tokio::join!(a2b, b2a);
    Ok(())
}

pub enum Ctl {
    Data(Vec<u8>),
    Resize(u16, u16),
    Close,
    /// rz:前端选好文件后回传(空列表表示用户取消)
    SendFiles(Vec<std::path::PathBuf>),
    /// 用户在浮条点取消
    CancelTransfer,
}

pub struct SessionEntry {
    pub info: SessionInfo,
    pub ctl: mpsc::UnboundedSender<Ctl>,
    /// SSH 会话持有连接句柄(重连后被替换,取用时克隆内层 Arc);本地终端为 None
    pub handle: Option<Arc<Mutex<Arc<Handle<ClientHandler>>>>>,
    pub recorder: Arc<Mutex<Option<Recorder>>>,
}

#[derive(Default)]
pub struct SessionManager {
    pub sessions: Mutex<HashMap<String, SessionEntry>>,
}

pub(crate) fn emit_state(app: &AppHandle, info: &SessionInfo) {
    let _ = app.emit("session://state", info.clone());
}

pub(crate) fn emit_data(app: &AppHandle, id: &str, data: &[u8]) {
    let payload = TermData {
        id: id.to_string(),
        data: base64::engine::general_purpose::STANDARD.encode(data),
    };
    let _ = app.emit("term://data", payload);
}

pub(crate) fn emit_line(app: &AppHandle, id: &str, text: &str) {
    emit_data(app, id, format!("\r\n\x1b[90m{text}\x1b[0m\r\n").as_bytes());
}

impl SessionManager {
    pub fn get_handle(&self, id: &str) -> Result<Arc<Handle<ClientHandler>>> {
        let sessions = self.sessions.lock().unwrap();
        let entry = sessions
            .get(id)
            .ok_or_else(|| AppError(format!("会话不存在: {id}")))?;
        entry
            .handle
            .as_ref()
            .map(|h| h.lock().unwrap().clone())
            .ok_or_else(|| AppError("本地终端不支持此操作".into()))
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        self.sessions.lock().unwrap().values().map(|e| e.info.clone()).collect()
    }

    pub(crate) fn update_info(&self, id: &str, f: impl FnOnce(&mut SessionInfo)) -> Option<SessionInfo> {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.get_mut(id).map(|e| {
            f(&mut e.info);
            e.info.clone()
        })
    }
}

fn read_settings(db: &Db) -> AppSettings {
    db.kv_get("settings")
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default()
}

pub async fn connect_host(
    db: Arc<Db>,
    vault: &Vault,
    host_id: &str,
    password: Option<&str>,
) -> Result<(Arc<Handle<ClientHandler>>, crate::models::Host, u32)> {
    let (handle, host, latency, _) = connect_host_full(db, vault, host_id, password).await?;
    Ok((handle, host, latency))
}

/// 完整版:额外返回 R 转发登记表(forward 模块用)
pub async fn connect_host_full(
    db: Arc<Db>,
    vault: &Vault,
    host_id: &str,
    password: Option<&str>,
) -> Result<(Arc<Handle<ClientHandler>>, crate::models::Host, u32, RForwardMap)> {
    let host = db.get_host(host_id)?;

    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(15)),
        ..Default::default()
    });

    let r_targets: RForwardMap = Arc::new(Mutex::new(HashMap::new()));
    let presented_mismatch = Arc::new(Mutex::new(None));
    let handler = ClientHandler {
        db: db.clone(),
        host_id: host_id.to_string(),
        presented_mismatch: presented_mismatch.clone(),
        r_targets: r_targets.clone(),
    };

    let t0 = Instant::now();
    let connect_result =
        client::connect(config, (host.addr.as_str(), host.port), handler).await;
    let mut handle = match connect_result {
        Ok(h) => h,
        Err(e) => {
            if let Some(new_fp) = presented_mismatch.lock().unwrap().take() {
                let known = db
                    .known_host_fingerprint(host_id)?
                    .unwrap_or_else(|| "?".into());
                return Err(AppError(format!(
                    "HOSTKEY_CHANGED: 服务器主机密钥已变更!已记录 {known},本次出示 {new_fp}。\
                     可能存在中间人攻击;确认服务器确实重装/换钥后,可选择「信任新指纹」重试。"
                )));
            }
            return Err(AppError(format!("连接 {}:{} 失败: {e}", host.addr, host.port)));
        }
    };
    let latency = t0.elapsed().as_millis() as u32;

    // 认证:优先密钥,其次密码
    let auth = if let Some(key_id) = host.key_id.clone() {
        let pem = String::from_utf8(vault.decrypt(&db.get_key_private_enc(&key_id)?)?)
            .map_err(|_| AppError("私钥解码失败".into()))?;
        let key = vault::parse_private(&pem)?;
        let rsa_hash = handle
            .best_supported_rsa_hash()
            .await
            .map_err(|e| AppError(format!("协商失败: {e}")))?
            .flatten();
        handle
            .authenticate_publickey(
                host.user.clone(),
                PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash),
            )
            .await
            .map_err(|e| AppError(format!("认证失败: {e}")))?
    } else if let Some(pw) = password {
        handle
            .authenticate_password(host.user.clone(), pw)
            .await
            .map_err(|e| AppError(format!("认证失败: {e}")))?
    } else {
        return Err(AppError(format!(
            "PASSWORD_REQUIRED: 主机 {} 未配置密钥,需要密码认证",
            host.name
        )));
    };
    if !auth.success() {
        return Err(AppError(format!(
            "认证被拒绝(user={}):请检查{}",
            host.user,
            if host.key_id.is_some() { "公钥是否已部署到服务器" } else { "密码是否正确" }
        )));
    }
    db.touch_host_connected(host_id)?;
    Ok((Arc::new(handle), host, latency, r_targets))
}

async fn open_shell_channel(
    handle: &Handle<ClientHandler>,
    cols: u16,
    rows: u16,
) -> Result<Channel<client::Msg>> {
    let channel = handle
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
    Ok(channel)
}

pub async fn open_shell_session(
    app: AppHandle,
    db: Arc<Db>,
    vault: Arc<Vault>,
    manager: Arc<SessionManager>,
    host_id: &str,
    password: Option<String>,
) -> Result<SessionInfo> {
    let (handle, host, latency) = connect_host(db.clone(), &vault, host_id, password.as_deref()).await?;

    let id = uuid::Uuid::new_v4().to_string();
    let (cols, rows) = (120u16, 32u16);
    let channel = open_shell_channel(&handle, cols, rows).await?;

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
        kind: "ssh".into(),
    };

    let (ctl_tx, ctl_rx) = mpsc::unbounded_channel::<Ctl>();
    let recorder: Arc<Mutex<Option<Recorder>>> = Arc::new(Mutex::new(None));
    let shared_handle = Arc::new(Mutex::new(handle));

    {
        let mut sessions = manager.sessions.lock().unwrap();
        sessions.insert(
            id.clone(),
            SessionEntry {
                info: info.clone(),
                ctl: ctl_tx,
                handle: Some(shared_handle.clone()),
                recorder: recorder.clone(),
            },
        );
    }
    emit_state(&app, &info);

    tokio::spawn(run_session(
        app,
        db,
        vault,
        manager,
        id,
        host.id.clone(),
        password,
        channel,
        ctl_rx,
        shared_handle,
        recorder,
    ));

    Ok(info)
}

/// 会话主循环:通道输出 → 前端事件;控制消息 → 通道;异常断开时按设置自动重连
#[allow(clippy::too_many_arguments)]
async fn run_session(
    app: AppHandle,
    db: Arc<Db>,
    vault: Arc<Vault>,
    manager: Arc<SessionManager>,
    id: String,
    host_id: String,
    password: Option<String>,
    mut channel: Channel<client::Msg>,
    mut ctl_rx: mpsc::UnboundedReceiver<Ctl>,
    shared_handle: Arc<Mutex<Arc<Handle<ClientHandler>>>>,
    recorder: Arc<Mutex<Option<Recorder>>>,
) {
    'outer: loop {
        let mut user_closed = false;
        loop {
            tokio::select! {
                msg = channel.wait() => {
                    match msg {
                        Some(ChannelMsg::Data { ref data }) => {
                            emit_data(&app, &id, data);
                            if let Some(rec) = recorder.lock().unwrap().as_mut() {
                                rec.write_output(data);
                            }
                        }
                        Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                            emit_data(&app, &id, data);
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
                            user_closed = true;
                            let _ = channel.eof().await;
                            break;
                        }
                        Some(Ctl::SendFiles(_)) | Some(Ctl::CancelTransfer) => {
                            // 非传输态收到,忽略(仅传输态由 zmodem::run 消费)
                        }
                    }
                }
            }
        }

        if user_closed {
            break;
        }

        // 异常断开:按设置决定是否自动重连
        if !read_settings(&db).auto_reconnect {
            break;
        }
        if let Some(info) = manager.update_info(&id, |i| i.state = "connecting".into()) {
            emit_state(&app, &info);
        } else {
            break; // 会话已被移除
        }

        let mut reconnected = false;
        for (attempt, delay) in [1u64, 2, 4, 8, 8].iter().enumerate() {
            emit_line(&app, &id, &format!("连接断开,{delay}s 后重连(第 {}/5 次)…", attempt + 1));
            // 等待期间用户可关闭标签中止重连
            let closed = tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(*delay)) => false,
                ctl = ctl_rx.recv() => matches!(ctl, Some(Ctl::Close) | None),
            };
            if closed {
                break;
            }
            match connect_host(db.clone(), &vault, &host_id, password.as_deref()).await {
                Ok((new_handle, _, latency)) => {
                    let (cols, rows) = manager
                        .list()
                        .into_iter()
                        .find(|s| s.id == id)
                        .map(|s| (s.cols, s.rows))
                        .unwrap_or((120, 32));
                    match open_shell_channel(&new_handle, cols, rows).await {
                        Ok(new_channel) => {
                            *shared_handle.lock().unwrap() = new_handle;
                            channel = new_channel;
                            if let Some(info) = manager.update_info(&id, |i| {
                                i.state = "connected".into();
                                i.latency_ms = Some(latency);
                            }) {
                                emit_state(&app, &info);
                            }
                            emit_line(&app, &id, "已重新连接");
                            reconnected = true;
                            break;
                        }
                        Err(e) => emit_line(&app, &id, &format!("重连失败: {e}")),
                    }
                }
                Err(e) => emit_line(&app, &id, &format!("重连失败: {e}")),
            }
        }
        if reconnected {
            continue 'outer;
        }
        break;
    }

    if let Some(info) = manager.update_info(&id, |i| i.state = "disconnected".into()) {
        emit_state(&app, &info);
    }
    emit_line(&app, &id, "[会话已断开]");
}

pub fn write_session(manager: &SessionManager, id: &str, data: &str) -> Result<()> {
    let sessions = manager.sessions.lock().unwrap();
    let entry = sessions.get(id).ok_or_else(|| AppError(format!("会话不存在: {id}")))?;
    entry
        .ctl
        .send(Ctl::Data(data.as_bytes().to_vec()))
        .map_err(|_| AppError("会话已关闭".into()))?;
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

/// 主机实时延迟探测:每 30s 对全部主机做一次 TCP 连接测时,推送 hosts://latency
pub fn spawn_latency_loop(app: AppHandle, db: Arc<Db>) {
    tokio::spawn(async move {
        loop {
            let hosts = db.list_hosts().unwrap_or_default();
            let mut result: HashMap<String, Option<u32>> = HashMap::new();
            for h in hosts {
                let t0 = Instant::now();
                let ok = tokio::time::timeout(
                    Duration::from_secs(3),
                    tokio::net::TcpStream::connect((h.addr.as_str(), h.port)),
                )
                .await;
                result.insert(
                    h.id,
                    match ok {
                        Ok(Ok(_)) => Some(t0.elapsed().as_millis() as u32),
                        _ => None,
                    },
                );
            }
            let _ = app.emit("hosts://latency", result);
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}
