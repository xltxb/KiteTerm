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
use crate::models::{AppSettings, SessionInfo, TermData, TransferJob};
use crate::recorder::Recorder;
use crate::sftp::TransferManager;
use crate::vault::{self, Vault};
use crate::zmodem::{self, Detected, Detector, ProgressSink, RunCtx, ZChannel};

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
    /// sz:前端选好保存目录后回传(None 表示用户取消)
    ReceiveInto(Option<std::path::PathBuf>),
    /// 拖文件到终端:自动向远端敲 rz,并把这些文件作为随后 rz 的发送列表(仿 Xshell)
    DropUpload(Vec<std::path::PathBuf>),
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

#[allow(clippy::too_many_arguments)]
pub async fn open_shell_session(
    app: AppHandle,
    db: Arc<Db>,
    vault: Arc<Vault>,
    manager: Arc<SessionManager>,
    transfers: Arc<crate::sftp::TransferManager>,
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
        transfers,
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

/// 会话内层循环的结束原因 —— 决定要不要自动重连。
///
/// 早先这里只有一个 `user_closed: bool`,于是「远端 shell 正常退出」和「连接断了」
/// 无从区分:用户敲 exit,SSH 先发 ExitStatus 再关通道,代码却把它当异常断开,
/// 转头就把人重连回去,结果是退不出这个会话。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionEnd {
    /// 用户关标签,或在 ZMODEM 传输中关掉会话
    UserClosed,
    /// 远端 shell 自己退了(exit / logout),收到了 ExitStatus
    RemoteExited,
    /// 通道没给退出状态就没了 —— 当作异常断开
    Broken,
}

/// 只有异常断开、且设置开着,才自动重连。
///
/// 另外两种都是「有人主动要结束」:用户关标签是他自己点的,远端 shell 退出是他敲的 exit。
/// 这两种情况下重连都是在跟用户对着干。
fn should_reconnect(end: SessionEnd, auto_reconnect: bool) -> bool {
    end == SessionEnd::Broken && auto_reconnect
}

/// 会话主循环:通道输出 → 前端事件;控制消息 → 通道;异常断开时按设置自动重连
#[allow(clippy::too_many_arguments)]
async fn run_session(
    app: AppHandle,
    db: Arc<Db>,
    vault: Arc<Vault>,
    manager: Arc<SessionManager>,
    transfers: Arc<crate::sftp::TransferManager>,
    id: String,
    host_id: String,
    password: Option<String>,
    mut channel: Channel<client::Msg>,
    mut ctl_rx: mpsc::UnboundedReceiver<Ctl>,
    shared_handle: Arc<Mutex<Arc<Handle<ClientHandler>>>>,
    recorder: Arc<Mutex<Option<Recorder>>>,
) {
    'outer: loop {
        // 默认按异常断开处理:没拿到明确的退出信号就宁可重连,也好过把人静默地踢下线
        let mut end = SessionEnd::Broken;
        // 每个连接维护一个握手探测器;命中并接管后复位
        let mut detector = Detector::new();
        // 拖入文件后的待发列表,等远端 rz 启动时消费
        let mut pending_drop: Option<PendingDrop> = None;
        // 拖入文件时随 rz 一起敲的工作目录探针,以及它抓回来的远端当前目录
        let mut pwd_probe = PwdProbe::default();
        let mut remote_cwd: Option<String> = None;
        loop {
            tokio::select! {
                msg = channel.wait() => {
                    match msg {
                        Some(ChannelMsg::Data { ref data }) => {
                            if let Some(dir) = pwd_probe.push(data) {
                                remote_cwd = Some(dir);
                            }
                            match detector.push(data) {
                                None => {
                                    emit_data(&app, &id, data);
                                    if let Some(rec) = recorder.lock().unwrap().as_mut() {
                                        rec.write_output(data);
                                    }
                                }
                                Some((mode, at)) => {
                                    // 命中点之前的字节仍照常送终端与录制
                                    if at > 0 {
                                        emit_data(&app, &id, &data[..at]);
                                        if let Some(rec) = recorder.lock().unwrap().as_mut() {
                                            rec.write_output(&data[..at]);
                                        }
                                    }
                                    // 命中点之后已到达的字节作为 zmodem 通道的预置缓冲,
                                    // 避免握手头之后紧跟的字节被丢掉
                                    let seed = data[at..].to_vec();
                                    // 远端 rz 是我们替用户敲的:直接用拖入的文件,不再弹框
                                    let preset = match mode {
                                        Detected::Send => PendingDrop::take_fresh(&mut pending_drop, Instant::now()),
                                        Detected::Receive => None,
                                    };
                                    // 探针只跟着我们替用户敲的那条 rz 命令走。手动敲的 rz 没有探针,
                                    // 而上一次拖拽留下的值可能已被 cd 改过 —— 故取走即清,且只在有
                                    // preset(即这次确实是拖拽触发)时采用,宁可不报也不报错目录。
                                    let probed = remote_cwd.take();
                                    let remote_dir = preset.is_some().then_some(probed).flatten();
                                    let closed = handle_zmodem(
                                        &app, &db, &transfers, &id,
                                        &mut channel, &mut ctl_rx, mode, seed, preset, remote_dir,
                                    )
                                    .await;
                                    // 传输结束:复位探测器,恢复正常循环
                                    detector = Detector::new();
                                    if closed {
                                        end = SessionEnd::UserClosed;
                                        break;
                                    }
                                }
                            }
                        }
                        Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                            emit_data(&app, &id, data);
                        }
                        // 远端 shell 正常退出。它之后还会跟 Eof/Close,但退出状态只有这里能看到,
                        // 错过就再也分不清「他敲了 exit」和「线断了」
                        Some(ChannelMsg::ExitStatus { .. }) => {
                            end = SessionEnd::RemoteExited;
                            break;
                        }
                        Some(ChannelMsg::Close) | None => {
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
                            end = SessionEnd::UserClosed;
                            let _ = channel.eof().await;
                            break;
                        }
                        Some(Ctl::DropUpload(paths)) => {
                            let (files, rejected) = split_droppable(paths);
                            if !rejected.is_empty() {
                                let names: Vec<String> = rejected
                                    .iter()
                                    .map(|p| p.display().to_string())
                                    .collect();
                                emit_line(&app, &id, &format!("暂不支持上传目录,已跳过: {}", names.join(", ")));
                            }
                            if files.is_empty() {
                                continue;
                            }
                            emit_line(&app, &id, &format!("已拖入 {} 个文件,正在启动远端 rz…", files.len()));
                            pending_drop = Some(PendingDrop::new(files, Instant::now()));
                            if channel.data(DROP_RZ_COMMAND).await.is_err() { break; }
                        }
                        Some(Ctl::SendFiles(_)) | Some(Ctl::ReceiveInto(_)) | Some(Ctl::CancelTransfer) => {
                            // 非传输态收到,忽略(仅传输态由 zmodem::run 消费)
                        }
                    }
                }
            }
        }

        if !should_reconnect(end, read_settings(&db).auto_reconnect) {
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

// ============================ ZMODEM 接管 ============================

/// 拖入文件后等远端 rz 启动的最长时间:超时视为远端没起 rz(多半是没装 lrzsz),
/// 列表作废,之后手动敲 rz 仍走弹框流程。
pub(crate) const DROP_PENDING_TTL: Duration = Duration::from_secs(15);

/// 拖入文件后自动敲的远端命令:`-E` 同名自动改名,既不静默覆盖也不触发 ZSKIP 跳过。
///
/// 前半段的 `printf` 是工作目录探针 —— ZMODEM 协议不会告诉发送方文件落在哪里
/// (`rz` 写进远端 shell 的当前目录),要在传输完成时报出目标目录只能主动问一次。
/// 输出包成 APC 序列(`ESC _ … ESC \`),xterm.js 会整段忽略,故不会在终端上显形;
/// 用 `\033` 八进制而非裸 ESC 字节,是为了让 shell 回显这行命令时不夹带控制字符。
const DROP_RZ_COMMAND: &[u8] = b"printf '\\033_KTPWD:%s\\033\\\\' \"$PWD\"; rz -E\r";

/// APC 探针的头尾标记。
const PWD_MARK_HEAD: &[u8] = b"\x1b_KTPWD:";
const PWD_MARK_TAIL: &[u8] = b"\x1b\\";
/// 跨块拼接缓冲的上限:远超任何合理路径长度,涨到这里说明流里根本没有标记,清掉防止无界增长。
const PWD_PROBE_CAP: usize = 8192;

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// 从远端输出里抓 [`DROP_RZ_COMMAND`] 探针回传的工作目录。
///
/// 数据按 `ChannelMsg` 分块到达,标记可能被切成两半,故保留一小段尾巴跨块拼接;
/// 没有标记时只留下「可能是半个头部」的那几个字节。
#[derive(Default)]
pub(crate) struct PwdProbe {
    buf: Vec<u8>,
}

impl PwdProbe {
    /// 喂入一块远端输出;凑齐完整标记时返回其中的路径(并清空缓冲)。
    pub(crate) fn push(&mut self, data: &[u8]) -> Option<String> {
        self.buf.extend_from_slice(data);
        match find_sub(&self.buf, PWD_MARK_HEAD) {
            Some(head) => {
                let from = head + PWD_MARK_HEAD.len();
                if let Some(tail) = find_sub(&self.buf[from..], PWD_MARK_TAIL) {
                    let path = String::from_utf8_lossy(&self.buf[from..from + tail]).into_owned();
                    self.buf.clear();
                    return (!path.is_empty()).then_some(path);
                }
                // 头到了尾还没到:丢掉头之前的字节,留住待续部分
                self.buf.drain(..head);
            }
            None => {
                // 没有标记:只保留可能被切断的头部前缀
                let keep = PWD_MARK_HEAD.len() - 1;
                if self.buf.len() > keep {
                    let cut = self.buf.len() - keep;
                    self.buf.drain(..cut);
                }
            }
        }
        if self.buf.len() > PWD_PROBE_CAP {
            self.buf.clear();
        }
        None
    }
}

/// 拖文件后暂存的待发列表,等探测到远端 rz 启动时一次性消费。
#[derive(Debug)]
pub(crate) struct PendingDrop {
    files: Vec<std::path::PathBuf>,
    at: Instant,
}

impl PendingDrop {
    pub(crate) fn new(files: Vec<std::path::PathBuf>, at: Instant) -> Self {
        Self { files, at }
    }

    /// 取走未过期的列表;过期的直接丢弃。两种情况槽位都被清空。
    pub(crate) fn take_fresh(
        slot: &mut Option<PendingDrop>,
        now: Instant,
    ) -> Option<Vec<std::path::PathBuf>> {
        let p = slot.take()?;
        (now.duration_since(p.at) <= DROP_PENDING_TTL).then_some(p.files)
    }
}

/// 拖入路径分拣:ZMODEM 只发普通文件,目录与不存在的路径剔除并返回供提示。
pub(crate) fn split_droppable(
    paths: Vec<std::path::PathBuf>,
) -> (Vec<std::path::PathBuf>, Vec<std::path::PathBuf>) {
    paths.into_iter().partition(|p| p.is_file())
}

/// zmodem 中止序列:5×CAN(0x18)后 5×BS(0x08)。lrzsz 收到即中止当前 sz/rz。
const ZMODEM_ABORT: [u8; 10] = [0x18, 0x18, 0x18, 0x18, 0x18, 0x08, 0x08, 0x08, 0x08, 0x08];

/// 把 russh 交互式 shell 通道适配成 zmodem 的 `ZChannel`:russh 的 `channel.wait()` 取的是
/// 整条 `ChannelMsg`,而 zmodem 状态机要的是字节流,故内部缓存 `ChannelMsg::Data` 的字节,
/// `read` 从缓存里按需切出。构造时用 `seed` 预置探测握手头之后已到达的字节,避免丢失。
struct RusshChannel<'a> {
    channel: &'a mut Channel<client::Msg>,
    buf: Vec<u8>,
    pos: usize,
}

impl<'a> RusshChannel<'a> {
    fn new(channel: &'a mut Channel<client::Msg>, seed: Vec<u8>) -> Self {
        Self { channel, buf: seed, pos: 0 }
    }
}

impl ZChannel for RusshChannel<'_> {
    async fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            // 先把缓存里剩余字节吐给调用方
            if self.pos < self.buf.len() {
                let n = (self.buf.len() - self.pos).min(out.len());
                out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
                self.pos += n;
                if self.pos >= self.buf.len() {
                    self.buf.clear();
                    self.pos = 0;
                }
                return Ok(n);
            }
            // 缓存空:等下一条通道消息
            match self.channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    self.buf = data[..].to_vec();
                    self.pos = 0;
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    self.buf = data[..].to_vec();
                    self.pos = 0;
                }
                // EOF/Close/通道结束:按 ZChannel 约定返回 Ok(0)
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => return Ok(0),
                // 窗口调整、ExitStatus 等与字节流无关的消息:忽略,继续等
                _ => {}
            }
        }
    }

    async fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.channel
            .data(data)
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::BrokenPipe, e.to_string()))
    }
}

/// zmodem 进度回调适配到 TransferManager:`start` push 一个 protocol="zmodem" 的
/// TransferJob(state="running"),`advance` 按字节算百分比与速率(200ms 节流),
/// `finish` 置 done/failed。`ProgressSink` 的方法都是 `&self`,故内部用 Mutex 存可变状态。
struct JobSink {
    app: AppHandle,
    transfers: Arc<TransferManager>,
    session_id: String,
    /// 上传落盘的远端目录,仅拖拽触发时由 [`PwdProbe`] 探到;手动 rz 为 None
    remote_dir: Option<String>,
    inner: Mutex<JobSinkState>,
}

struct JobSinkState {
    id: Option<String>,
    total: u64,
    start: Option<Instant>,
    last_emit: Option<Instant>,
    /// 当前文件的方向与本地路径,finish 成功时打到终端上
    direction: String,
    path: std::path::PathBuf,
}

impl JobSink {
    fn new(
        app: AppHandle,
        transfers: Arc<TransferManager>,
        session_id: String,
        remote_dir: Option<String>,
    ) -> Self {
        Self {
            app,
            transfers,
            session_id,
            remote_dir,
            inner: Mutex::new(JobSinkState {
                id: None,
                total: 0,
                start: None,
                last_emit: None,
                direction: String::new(),
                path: std::path::PathBuf::new(),
            }),
        }
    }
}

impl ProgressSink for JobSink {
    fn start(
        &self,
        direction: &str,
        name: &str,
        path: &std::path::Path,
        total: u64,
        _index: usize,
        _count: usize,
    ) {
        let job_id = uuid::Uuid::new_v4().to_string();
        let job = TransferJob {
            id: job_id.clone(),
            direction: direction.to_string(),
            name: name.to_string(),
            local_path: path.to_string_lossy().into_owned(),
            remote_path: String::new(),
            session_id: self.session_id.clone(),
            progress: 0.0,
            rate_bps: 0,
            state: "running".into(),
            error: None,
            elapsed_ms: 0,
            protocol: "zmodem".into(),
        };
        self.transfers.jobs.lock().unwrap().push(job.clone());
        crate::sftp::emit_job(&self.app, &job);
        let mut st = self.inner.lock().unwrap();
        st.id = Some(job_id);
        st.total = total;
        st.start = Some(Instant::now());
        st.last_emit = None;
        st.direction = direction.to_string();
        st.path = path.to_path_buf();
    }

    fn advance(&self, transferred: u64) {
        let (id, total, start) = {
            let mut st = self.inner.lock().unwrap();
            let Some(id) = st.id.clone() else { return };
            // 200ms 节流,避免大文件每个子包都推事件
            let now = Instant::now();
            if let Some(last) = st.last_emit {
                if now.duration_since(last).as_millis() < 200 {
                    return;
                }
            }
            st.last_emit = Some(now);
            (id, st.total, st.start)
        };
        let elapsed = start.map(|s| s.elapsed()).unwrap_or_default();
        let rate = if elapsed.as_secs_f64() > 0.0 {
            (transferred as f64 / elapsed.as_secs_f64()) as u64
        } else {
            0
        };
        let pct = if total > 0 {
            (transferred as f64 / total as f64 * 100.0).min(99.9)
        } else {
            99.9
        };
        crate::sftp::update_job(&self.app, &self.transfers.jobs, &id, |j| {
            j.progress = pct;
            j.rate_bps = rate;
            j.elapsed_ms = elapsed.as_millis() as u64;
        });
    }

    fn finish(&self, ok: bool, error: Option<String>) {
        let (id, start, line) = {
            let st = self.inner.lock().unwrap();
            let Some(id) = st.id.clone() else { return };
            (
                id,
                st.start,
                transfer_done_line(&st.direction, &st.path, self.remote_dir.as_deref()),
            )
        };
        let elapsed = start.map(|s| s.elapsed()).unwrap_or_default();
        crate::sftp::update_job(&self.app, &self.transfers.jobs, &id, |j| {
            if ok {
                j.state = "done".into();
                j.progress = 100.0;
                j.rate_bps = 0;
            } else {
                j.state = "failed".into();
                j.error = error.clone();
            }
            j.elapsed_ms = elapsed.as_millis() as u64;
        });
        // 每个文件成功收尾时把本地路径打到终端上,用户不用再去传输页找文件
        if ok {
            emit_line(&self.app, &self.session_id, &line);
        }
    }
}

/// 单个文件传输完成时打到终端的提示行。
///
/// 下载说明落盘位置。上传两段都给:发的是哪个本地文件,以及落到远端哪个目录 ——
/// 后者只有拖拽触发时才探得到(见 [`PwdProbe`]),手动敲 `rz` 时退回只报本地路径,
/// 因为 ZMODEM 协议本身不回传接收端的落盘位置,猜一个不如不报。
fn transfer_done_line(direction: &str, path: &std::path::Path, remote_dir: Option<&str>) -> String {
    let p = path.display();
    if direction == "download" {
        format!("已保存到 {p}")
    } else if let Some(dir) = remote_dir {
        format!("已发送 {p} → {dir}")
    } else {
        format!("已发送 {p}")
    }
}

/// 解析下载目录:优先 `AppSettings.download_dir`(支持 `~` 展开),空则回退到系统下载目录、
/// 再回退到用户主目录,最后回退到当前目录。
fn resolve_download_dir(db: &Db) -> std::path::PathBuf {
    let s = read_settings(db);
    let dir = s.download_dir.trim();
    if !dir.is_empty() {
        return std::path::PathBuf::from(crate::sftp::shellexpand_home(dir));
    }
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// 一次 zmodem 传输的结局。
enum TransferEnd {
    /// 状态机跑完(Ok 成功,Err 协议失败)。
    Completed(std::io::Result<()>),
    /// 传输期间被取消;`close` 表示取消源自会话关闭(需结束会话)。
    Cancelled { close: bool },
}

/// 在给定通道上驱动一次 zmodem 传输,同时监听 `ctl_rx`:
/// `CancelTransfer`/`Close` 中止传输,`Resize` 暂存到 `deferred_resize` 待传输后补发。
/// `zmodem::run` 借用了通道(经 `RusshChannel`),而取消后要用同一通道发中止序列,
/// 故把 `run` 的 future 与 `RusshChannel` 一起限定在内层作用域,块结束时先 drop 掉它们、
/// 释放对 `channel` 的借用,再在块外发中止序列——保持 `zmodem::run` 本身不感知 ctl。
async fn run_with_cancel(
    channel: &mut Channel<client::Msg>,
    ctl_rx: &mut mpsc::UnboundedReceiver<Ctl>,
    mode: Detected,
    seed: Vec<u8>,
    sink: &JobSink,
    download_dir: std::path::PathBuf,
    files: Option<Vec<std::path::PathBuf>>,
    deferred_resize: &mut Option<(u16, u16)>,
) -> TransferEnd {
    let end;
    {
        let mut rchan = RusshChannel::new(channel, seed);
        let run_fut = zmodem::run(
            mode,
            RunCtx { chan: &mut rchan, sink, download_dir, files },
        );
        tokio::pin!(run_fut);
        end = loop {
            tokio::select! {
                r = &mut run_fut => break TransferEnd::Completed(r),
                ctl = ctl_rx.recv() => match ctl {
                    Some(Ctl::CancelTransfer) => break TransferEnd::Cancelled { close: false },
                    Some(Ctl::Close) | None => break TransferEnd::Cancelled { close: true },
                    Some(Ctl::Resize(c, r)) => { *deferred_resize = Some((c, r)); }
                    // 传输期间键入与迟到的选文件/选目录消息一律忽略
                    Some(Ctl::Data(_)) | Some(Ctl::SendFiles(_)) | Some(Ctl::ReceiveInto(_)) | Some(Ctl::DropUpload(_)) => {}
                },
            }
        };
        // rchan(及 run_fut)在此块结束时 drop,随后对 channel 的可变借用释放
    }
    if let TransferEnd::Cancelled { .. } = end {
        // 向对端发 zmodem 中止序列,并把 job 标记为失败(run 被中途 drop,不会自行 finish)
        let _ = channel.data(&ZMODEM_ABORT[..]).await;
        sink.finish(false, Some("已取消".to_string()));
    }
    end
}

/// sz 方向等前端选保存目录的结果。
enum DirChoice {
    Chosen(std::path::PathBuf),
    /// 用户取消或会话关闭;`close` 为真表示应结束会话
    Cancelled { close: bool },
}

/// 阻塞等待前端回传保存目录:期间键入忽略、窗口尺寸暂存到 `deferred_resize`,
/// 取消/关闭即返回 `Cancelled`(与 rz 等选文件的循环语义一致)。
async fn await_receive_dir(
    ctl_rx: &mut mpsc::UnboundedReceiver<Ctl>,
    deferred_resize: &mut Option<(u16, u16)>,
) -> DirChoice {
    loop {
        match ctl_rx.recv().await {
            Some(Ctl::ReceiveInto(Some(dir))) => return DirChoice::Chosen(dir),
            Some(Ctl::ReceiveInto(None)) | Some(Ctl::CancelTransfer) => {
                return DirChoice::Cancelled { close: false }
            }
            Some(Ctl::Close) | None => return DirChoice::Cancelled { close: true },
            Some(Ctl::Resize(c, r)) => *deferred_resize = Some((c, r)),
            Some(Ctl::Data(_)) | Some(Ctl::SendFiles(_)) | Some(Ctl::DropUpload(_)) => {}
        }
    }
}

/// 接管一次 zmodem 传输:构造 `JobSink`,`Receive` 方向先请前端选保存目录、
/// `Send` 方向先请前端选文件,随后驱动 `run_with_cancel`,最后补发暂存的窗口尺寸变化。
/// 返回 `true` 表示用户在传输期间关闭了会话(调用方应结束会话)。
#[allow(clippy::too_many_arguments)]
async fn handle_zmodem(
    app: &AppHandle,
    db: &Db,
    transfers: &Arc<TransferManager>,
    id: &str,
    channel: &mut Channel<client::Msg>,
    ctl_rx: &mut mpsc::UnboundedReceiver<Ctl>,
    mode: Detected,
    seed: Vec<u8>,
    preset: Option<Vec<std::path::PathBuf>>,
    remote_dir: Option<String>,
) -> bool {
    let mut download_dir = resolve_download_dir(db);
    let sink = JobSink::new(app.clone(), transfers.clone(), id.to_string(), remote_dir);
    let mut deferred_resize: Option<(u16, u16)> = None;

    let files: Option<Vec<std::path::PathBuf>> = match mode {
        Detected::Receive => {
            // 请前端弹目录选择框(默认目录按设置解析),阻塞等其回传;远端 sz 此时在等我们的 ZRINIT
            let _ = app.emit(
                "zmodem://pick-dir",
                serde_json::json!({ "sessionId": id, "defaultDir": download_dir.to_string_lossy() }),
            );
            emit_line(app, id, "检测到 ZMODEM 下载(sz),请选择保存目录…");
            match await_receive_dir(ctl_rx, &mut deferred_resize).await {
                DirChoice::Chosen(dir) => download_dir = dir,
                DirChoice::Cancelled { close } => {
                    // 用户取消 / 会话关闭:发中止序列结束远端 sz
                    let _ = channel.data(&ZMODEM_ABORT[..]).await;
                    emit_line(app, id, "已取消 ZMODEM 下载");
                    if let Some((c, r)) = deferred_resize {
                        let _ = channel.window_change(c as u32, r as u32, 0, 0).await;
                    }
                    return close;
                }
            }
            emit_line(app, id, &format!("开始接收,保存到 {}…", download_dir.display()));
            None
        }
        Detected::Send => {
            let mut chosen: Vec<std::path::PathBuf> = Vec::new();
            let mut close = false;
            if let Some(files) = preset {
                // 拖入文件触发的 rz:发送列表已定,跳过选文件框
                chosen = files;
            } else {
                // 请前端弹出文件选择器,并阻塞等待其回传选择结果
                let _ = app.emit("zmodem://pick-files", serde_json::json!({ "sessionId": id }));
                emit_line(app, id, "检测到 ZMODEM 上传(rz),请选择要发送的文件…");
                loop {
                    match ctl_rx.recv().await {
                        Some(Ctl::SendFiles(paths)) => {
                            chosen = paths;
                            break;
                        }
                        Some(Ctl::CancelTransfer) => break,
                        Some(Ctl::Close) | None => {
                            close = true;
                            break;
                        }
                        Some(Ctl::Resize(c, r)) => deferred_resize = Some((c, r)),
                        Some(Ctl::Data(_)) | Some(Ctl::ReceiveInto(_)) | Some(Ctl::DropUpload(_)) => {}
                    }
                }
            }
            if chosen.is_empty() {
                // 用户取消 / 会话关闭:发中止序列结束远端 rz
                let _ = channel.data(&ZMODEM_ABORT[..]).await;
                emit_line(app, id, "已取消 ZMODEM 上传");
                if let Some((c, r)) = deferred_resize {
                    let _ = channel.window_change(c as u32, r as u32, 0, 0).await;
                }
                return close;
            }
            emit_line(app, id, &format!("开始发送 {} 个文件…", chosen.len()));
            Some(chosen)
        }
    };

    let end = run_with_cancel(
        channel,
        ctl_rx,
        mode,
        seed,
        &sink,
        download_dir,
        files,
        &mut deferred_resize,
    )
    .await;

    let close = match end {
        TransferEnd::Completed(Ok(())) => {
            emit_line(app, id, "ZMODEM 传输完成");
            false
        }
        TransferEnd::Completed(Err(e)) => {
            // 防御性收尾:receive 的传输层错误经 `?` 上抛时可能未走到其内部的 finish 分支
            // (仅 unexpected-EOF / 过量 CRC 路径会 finish),否则 job 会永久停在 running。
            // 在此接管点补一次 finish(false);JobSink::finish 在无 job id 时早退、对已失败 job
            // 重复置失败也无害,故幂等安全。
            sink.finish(false, Some(e.to_string()));
            emit_line(app, id, &format!("ZMODEM 传输失败: {e}"));
            false
        }
        TransferEnd::Cancelled { close } => {
            emit_line(app, id, "ZMODEM 传输已取消");
            close
        }
    };

    // 补发传输期间暂存的窗口尺寸变化
    if let Some((c, r)) = deferred_resize {
        let _ = channel.window_change(c as u32, r as u32, 0, 0).await;
    }
    close
}

/// 向指定会话投递一条控制消息(仿 `write_session`),供 zmodem 命令使用。
pub fn send_ctl(manager: &SessionManager, id: &str, ctl: Ctl) -> Result<()> {
    let sessions = manager.sessions.lock().unwrap();
    let entry = sessions
        .get(id)
        .ok_or_else(|| AppError(format!("会话不存在: {id}")))?;
    entry
        .ctl
        .send(ctl)
        .map_err(|_| AppError("会话已关闭".into()))?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{
        await_receive_dir, should_reconnect, split_droppable, transfer_done_line, Ctl, DirChoice,
        PendingDrop, PwdProbe, SessionEnd, DROP_PENDING_TTL, DROP_RZ_COMMAND, PWD_PROBE_CAP,
    };
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc;

    /// 用户敲 exit 让远端 shell 正常退出,必须当场结束会话。
    /// 早先把 ExitStatus 和「连接断了」归成一类,结果用户一 exit 就被自动重连拽回去,退不掉。
    #[test]
    fn remote_shell_exit_never_reconnects() {
        assert!(!should_reconnect(SessionEnd::RemoteExited, true), "开着自动重连也不该拽回来");
        assert!(!should_reconnect(SessionEnd::RemoteExited, false));
    }

    /// 用户关标签同理:是他自己要走
    #[test]
    fn user_closing_tab_never_reconnects() {
        assert!(!should_reconnect(SessionEnd::UserClosed, true));
        assert!(!should_reconnect(SessionEnd::UserClosed, false));
    }

    /// 只有真的断线才重连,且要看设置
    #[test]
    fn broken_link_reconnects_only_when_setting_allows() {
        assert!(should_reconnect(SessionEnd::Broken, true));
        assert!(!should_reconnect(SessionEnd::Broken, false));
    }

    #[test]
    fn pending_drop_is_consumed_once_while_fresh() {
        let now = Instant::now();
        let mut slot = Some(PendingDrop::new(vec![PathBuf::from("/a.txt")], now));
        let got = PendingDrop::take_fresh(&mut slot, now + Duration::from_secs(1));
        assert_eq!(got, Some(vec![PathBuf::from("/a.txt")]));
        // 消费后槽位清空,再来一次 rz 走弹框流程
        assert!(slot.is_none());
        assert_eq!(PendingDrop::take_fresh(&mut slot, now), None);
    }

    #[test]
    fn pending_drop_expires_after_ttl() {
        let now = Instant::now();
        let mut slot = Some(PendingDrop::new(vec![PathBuf::from("/a.txt")], now));
        let got = PendingDrop::take_fresh(&mut slot, now + DROP_PENDING_TTL + Duration::from_secs(1));
        assert_eq!(got, None);
        assert!(slot.is_none(), "过期列表应被丢弃");
    }

    #[test]
    fn split_droppable_keeps_files_and_reports_dirs() {
        let dir = std::env::temp_dir().join(format!("kiteterm-drop-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, b"x").unwrap();
        let missing = dir.join("missing.bin");

        let (files, rejected) = split_droppable(vec![file.clone(), dir.join("sub"), missing.clone()]);
        assert_eq!(files, vec![file]);
        assert_eq!(rejected, vec![dir.join("sub"), missing]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn await_receive_dir_returns_chosen_dir() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(Ctl::ReceiveInto(Some(PathBuf::from("/tmp/dl")))).unwrap();
        let mut resize = None;
        let got = await_receive_dir(&mut rx, &mut resize).await;
        assert!(matches!(got, DirChoice::Chosen(p) if p == Path::new("/tmp/dl")));
    }

    #[tokio::test]
    async fn await_receive_dir_cancel_keeps_session_and_defers_resize() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        // 等待期间的键入忽略、窗口尺寸暂存,用户取消选择 → 不关会话
        tx.send(Ctl::Data(b"x".to_vec())).unwrap();
        tx.send(Ctl::Resize(100, 40)).unwrap();
        tx.send(Ctl::ReceiveInto(None)).unwrap();
        let mut resize = None;
        let got = await_receive_dir(&mut rx, &mut resize).await;
        assert!(matches!(got, DirChoice::Cancelled { close: false }));
        assert_eq!(resize, Some((100, 40)));
    }

    #[tokio::test]
    async fn await_receive_dir_close_requests_session_close() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(Ctl::Close).unwrap();
        let mut resize = None;
        let got = await_receive_dir(&mut rx, &mut resize).await;
        assert!(matches!(got, DirChoice::Cancelled { close: true }));
        // 发送端 drop 后通道结束,同样视为关闭
        let (tx2, mut rx2) = mpsc::unbounded_channel::<Ctl>();
        drop(tx2);
        let got = await_receive_dir(&mut rx2, &mut None).await;
        assert!(matches!(got, DirChoice::Cancelled { close: true }));
    }

    #[test]
    fn transfer_done_line_distinguishes_direction() {
        let p = Path::new("/Users/me/Downloads/foo (1).txt");
        assert_eq!(
            transfer_done_line("download", p, None),
            "已保存到 /Users/me/Downloads/foo (1).txt"
        );
        assert_eq!(
            transfer_done_line("upload", p, None),
            "已发送 /Users/me/Downloads/foo (1).txt"
        );
    }

    /// 探到远端目录时,上传两段都报;下载方向不受影响。
    #[test]
    fn transfer_done_line_appends_remote_dir_for_upload() {
        let p = Path::new("D:\\stuff\\foo.txt");
        assert_eq!(
            transfer_done_line("upload", p, Some("/home/abiu/work")),
            "已发送 D:\\stuff\\foo.txt → /home/abiu/work"
        );
        // 下载的落盘位置是本地路径,远端目录与它无关
        assert_eq!(
            transfer_done_line("download", p, Some("/home/abiu/work")),
            "已保存到 D:\\stuff\\foo.txt"
        );
    }

    /// 注入的命令必须自带探针,否则上传完成时报不出远端目录。
    #[test]
    fn drop_rz_command_carries_pwd_probe() {
        let cmd = String::from_utf8(DROP_RZ_COMMAND.to_vec()).unwrap();
        assert_eq!(cmd, "printf '\\033_KTPWD:%s\\033\\\\' \"$PWD\"; rz -E\r");
        // 命令行本身不含裸控制字符 —— 回显时不会打乱终端
        assert!(!cmd.trim_end_matches('\r').bytes().any(|b| b < 0x20));
    }

    #[test]
    fn pwd_probe_reads_path_from_single_chunk() {
        let mut probe = PwdProbe::default();
        assert_eq!(
            probe.push(b"ls output\r\n\x1b_KTPWD:/home/abiu/work\x1b\\rz waiting"),
            Some("/home/abiu/work".to_string())
        );
    }

    /// 标记被 ChannelMsg 切成两半时必须仍能拼回来。
    #[test]
    fn pwd_probe_joins_split_marker() {
        let mut probe = PwdProbe::default();
        assert_eq!(probe.push(b"noise\x1b_KTP"), None);
        assert_eq!(probe.push(b"WD:/srv/app"), None);
        assert_eq!(probe.push(b"\x1b\\more"), Some("/srv/app".to_string()));
    }

    /// 没有标记的普通输出不该让缓冲无界增长。
    #[test]
    fn pwd_probe_does_not_grow_without_marker() {
        let mut probe = PwdProbe::default();
        for _ in 0..64 {
            assert_eq!(probe.push(&vec![b'x'; 4096]), None);
        }
        assert!(probe.buf.len() < PWD_PROBE_CAP);
    }

    /// 只有头没有尾(远端没跑完 printf)时不能误报空目录。
    #[test]
    fn pwd_probe_ignores_unterminated_marker() {
        let mut probe = PwdProbe::default();
        assert_eq!(probe.push(b"\x1b_KTPWD:/half/way"), None);
    }
}

#[cfg(test)]
mod session_manager_tests {
    use super::{ClientHandler, Handle, SessionEntry, SessionManager};
    use crate::models::SessionInfo;
    use crate::test_support::spawn_ssh_server;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    fn info(id: &str, kind: &str) -> SessionInfo {
        SessionInfo {
            id: id.into(),
            host_id: "h1".into(),
            host_name: "主机".into(),
            user: "tester".into(),
            addr: "127.0.0.1".into(),
            port: 22,
            state: "connected".into(),
            latency_ms: None,
            cipher: String::new(),
            cols: 80,
            rows: 24,
            recording: false,
            recording_since: None,
            kind: kind.into(),
        }
    }

    /// 往管理器里放一条会话;handle 为 None 即本地终端
    fn park(
        mgr: &SessionManager,
        id: &str,
        kind: &str,
        handle: Option<Arc<russh::client::Handle<ClientHandler>>>,
    ) {
        // 这些用例不走控制通道,接收端随即丢弃无妨
        let (ctl, _rx) = mpsc::unbounded_channel();
        let entry = SessionEntry {
            info: info(id, kind),
            ctl,
            handle: handle.map(|h| Arc::new(Mutex::new(h))),
            recorder: Arc::new(Mutex::new(None)),
        };
        mgr.sessions.lock().unwrap().insert(id.to_string(), entry);
    }

    /// 真地连一次测试 SSH 服务端,拿一个货真价实的 Handle。
    /// 在 ssh 模块内部,可以直接拼 ClientHandler,不必绕 connect_host_full 那套主机配置
    async fn real_handle(db: &Arc<crate::db::Db>, ssh_port: u16) -> Arc<Handle<ClientHandler>> {
        let handler = ClientHandler {
            db: db.clone(),
            host_id: "h1".into(),
            presented_mismatch: Arc::new(Mutex::new(None)),
            r_targets: Arc::new(Mutex::new(std::collections::HashMap::new())),
        };
        let mut handle = russh::client::connect(
            Arc::new(russh::client::Config::default()),
            ("127.0.0.1", ssh_port),
            handler,
        )
        .await
        .expect("应能连上测试 SSH 服务端");
        assert!(
            handle.authenticate_password("tester", "pw").await.unwrap().success(),
            "测试服务端应放行任何密码"
        );
        Arc::new(handle)
    }

    fn scratch_db(name: &str) -> Arc<crate::db::Db> {
        let dir = std::env::temp_dir().join("kite-ssh-test").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(crate::db::Db::open(dir).unwrap())
    }

    #[test]
    fn get_handle_reports_missing_session_with_its_id() {
        let mgr = SessionManager::default();
        let err = mgr.get_handle("nope").err().expect("应返回错误");
        assert!(err.0.contains("会话不存在"), "实际: {}", err.0);
        assert!(err.0.contains("nope"), "错误信息应带上会话 id,实际: {}", err.0);
    }

    #[test]
    fn get_handle_rejects_local_terminal() {
        let mgr = SessionManager::default();
        park(&mgr, "s-local", "local", None);
        let err = mgr.get_handle("s-local").err().expect("应返回错误");
        assert!(err.0.contains("本地终端不支持此操作"), "实际: {}", err.0);
    }

    #[tokio::test]
    async fn get_handle_returns_the_current_handle_after_reconnect() {
        let ssh_port = spawn_ssh_server().await;
        let db = scratch_db("reconnect");
        let first = real_handle(&db, ssh_port).await;
        let second = real_handle(&db, ssh_port).await;
        assert!(!Arc::ptr_eq(&first, &second), "两次连接应是不同的句柄");

        let mgr = SessionManager::default();
        park(&mgr, "s-ssh", "ssh", Some(first.clone()));
        assert!(Arc::ptr_eq(&mgr.get_handle("s-ssh").unwrap(), &first));

        // 重连:替换内层 Arc。取用时若是在插入那刻就克隆保存,这里就会拿到旧句柄
        {
            let sessions = mgr.sessions.lock().unwrap();
            let slot = sessions.get("s-ssh").unwrap().handle.as_ref().unwrap();
            *slot.lock().unwrap() = second.clone();
        }
        assert!(
            Arc::ptr_eq(&mgr.get_handle("s-ssh").unwrap(), &second),
            "重连后应拿到新句柄"
        );
    }

    #[test]
    fn list_returns_every_session() {
        let mgr = SessionManager::default();
        assert!(mgr.list().is_empty());

        park(&mgr, "a", "ssh", None);
        park(&mgr, "b", "local", None);
        let mut ids: Vec<String> = mgr.list().into_iter().map(|i| i.id).collect();
        ids.sort(); // HashMap 无序
        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn update_info_mutates_and_returns_the_new_state() {
        let mgr = SessionManager::default();
        park(&mgr, "s1", "ssh", None);

        let got = mgr.update_info("s1", |i| {
            i.state = "disconnected".into();
            i.latency_ms = Some(42);
        });
        let got = got.expect("会话存在时应返回新状态");
        assert_eq!(got.state, "disconnected");
        assert_eq!(got.latency_ms, Some(42));

        // 返回的是快照,内部状态也得真改了
        let stored = mgr.list().into_iter().find(|i| i.id == "s1").unwrap();
        assert_eq!(stored.state, "disconnected");
        assert_eq!(stored.latency_ms, Some(42));
    }

    #[test]
    fn update_info_on_unknown_session_is_none_and_touches_nothing() {
        let mgr = SessionManager::default();
        park(&mgr, "s1", "ssh", None);

        let mut called = false;
        let got = mgr.update_info("nope", |_| called = true);
        assert!(got.is_none());
        assert!(!called, "会话不存在时不应调用改写闭包");
        assert_eq!(mgr.list().len(), 1);
    }
}
