//! 端口转发:L(本地)/ D(SOCKS5 动态)实时转发与流量统计;R(远程)暂未支持。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::ForwardRule;
use crate::ssh::{self, ClientHandler, SessionManager};
use crate::vault::Vault;

pub struct ActiveForward {
    task: JoinHandle<()>,
    pub up: Arc<AtomicU64>,
    pub down: Arc<AtomicU64>,
}

#[derive(Default)]
pub struct ForwardManager {
    pub active: Mutex<HashMap<String, ActiveForward>>,
}

impl ForwardManager {
    pub fn stop(&self, rule_id: &str) {
        if let Some(f) = self.active.lock().unwrap().remove(rule_id) {
            f.task.abort();
        }
    }
}

/// 找到可用的 SSH 连接:优先复用该主机的活跃会话,否则新建专用连接
async fn handle_for_host(
    db: &Arc<Db>,
    vault: &Vault,
    sessions: &SessionManager,
    host_id: &str,
) -> Result<Arc<russh::client::Handle<ClientHandler>>> {
    let existing = {
        let s = sessions.sessions.lock().unwrap();
        s.values()
            .find(|e| e.info.host_id == host_id && e.info.state == "connected" && e.handle.is_some())
            .map(|e| e.handle.as_ref().unwrap().lock().unwrap().clone())
    };
    if let Some(h) = existing {
        return Ok(h);
    }
    let (h, _, _) = ssh::connect_host(db.clone(), vault, host_id, None).await?;
    Ok(h)
}

/// 应用启动时恢复所有启用中的转发规则;起不来的规则自动置为停用
pub fn spawn_restore_enabled(
    app: AppHandle,
    db: Arc<Db>,
    vault: Arc<Vault>,
    sessions: Arc<SessionManager>,
    forwards: Arc<ForwardManager>,
) {
    tokio::spawn(async move {
        let rules = db.list_forwards().unwrap_or_default();
        for rule in rules.into_iter().filter(|r| r.enabled) {
            if let Err(e) = start_forward(
                app.clone(),
                db.clone(),
                vault.clone(),
                sessions.clone(),
                forwards.clone(),
                rule.clone(),
            )
            .await
            {
                eprintln!("[forward] 启动恢复规则 {} 失败: {e}", rule.id);
                let _ = db.set_forward_enabled(&rule.id, false);
            }
        }
        if let Ok(list) = db.list_forwards() {
            let _ = app.emit("forward://stats", list);
        }
    });
}

/// 定期把流量计数落库并推送前端;规则停用后任务自行退出
fn spawn_traffic_flush(
    app: AppHandle,
    db: Arc<Db>,
    rule_id: String,
    up: Arc<AtomicU64>,
    down: Arc<AtomicU64>,
) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let (u, d) = (up.swap(0, Ordering::Relaxed), down.swap(0, Ordering::Relaxed));
            if db.add_forward_traffic(&rule_id, u, d).is_err() {
                break;
            }
            match db.list_forwards() {
                Ok(list) if list.iter().any(|f| f.id == rule_id && f.enabled) => {
                    let _ = app.emit("forward://stats", list);
                }
                _ => break,
            }
        }
    });
}

pub async fn start_forward(
    app: AppHandle,
    db: Arc<Db>,
    vault: Arc<Vault>,
    sessions: Arc<SessionManager>,
    forwards: Arc<ForwardManager>,
    rule: ForwardRule,
) -> Result<()> {
    if rule.kind == "R" {
        return start_remote_forward(app, db, vault, forwards, rule).await;
    }
    let handle = handle_for_host(&db, &vault, &sessions, &rule.via_host_id).await?;
    let listener = TcpListener::bind(("127.0.0.1", rule.local_port))
        .await
        .map_err(|e| AppError(format!("监听 127.0.0.1:{} 失败: {e}", rule.local_port)))?;

    let up = Arc::new(AtomicU64::new(0));
    let down = Arc::new(AtomicU64::new(0));
    let (t_up, t_down) = (up.clone(), down.clone());
    let kind = rule.kind.clone();
    let target = rule.target.clone();

    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = listener.accept().await else { break };
            let handle = handle.clone();
            let kind = kind.clone();
            let target = target.clone();
            let (up, down) = (t_up.clone(), t_down.clone());
            tokio::spawn(async move {
                let r = if kind == "D" {
                    socks5_conn(handle, stream, peer.port(), up, down).await
                } else {
                    direct_conn(handle, stream, peer.port(), &target, up, down).await
                };
                if let Err(e) = r {
                    eprintln!("[forward] 连接处理失败: {e}");
                }
            });
        }
    });

    forwards
        .active
        .lock()
        .unwrap()
        .insert(rule.id.clone(), ActiveForward { task, up: up.clone(), down: down.clone() });
    spawn_traffic_flush(app, db, rule.id.clone(), up, down);
    Ok(())
}

/// R 型远程转发:让服务器监听 rule.local_port,进站连接经 SSH 回送到本地 rule.target。
/// 使用专用 SSH 连接以持有 R 登记表;连接由 keeper 任务保活,停用时随任务中止而断开。
async fn start_remote_forward(
    app: AppHandle,
    db: Arc<Db>,
    vault: Arc<Vault>,
    forwards: Arc<ForwardManager>,
    rule: ForwardRule,
) -> Result<()> {
    let (handle, _, _, r_targets) =
        ssh::connect_host_full(db.clone(), &vault, &rule.via_host_id, None).await?;
    let (host, port) = split_target(&rule.target)?;

    let up = Arc::new(AtomicU64::new(0));
    let down = Arc::new(AtomicU64::new(0));
    r_targets.lock().unwrap().insert(
        rule.local_port as u32,
        crate::ssh::RTarget {
            host: host.to_string(),
            port,
            up: up.clone(),
            down: down.clone(),
        },
    );

    handle
        .tcpip_forward("127.0.0.1", rule.local_port as u32)
        .await
        .map_err(|e| AppError(format!("远端监听 127.0.0.1:{} 被拒绝: {e}", rule.local_port)))?;

    // keeper:持有专用连接;task.abort() 即丢弃 handle → 连接关闭,远端监听随之失效
    let task = tokio::spawn(async move {
        let _keep = handle;
        std::future::pending::<()>().await;
    });
    forwards
        .active
        .lock()
        .unwrap()
        .insert(rule.id.clone(), ActiveForward { task, up: up.clone(), down: down.clone() });
    spawn_traffic_flush(app, db, rule.id.clone(), up, down);
    Ok(())
}

async fn direct_conn(
    handle: Arc<russh::client::Handle<ClientHandler>>,
    mut local: TcpStream,
    peer_port: u16,
    target: &str,
    up: Arc<AtomicU64>,
    down: Arc<AtomicU64>,
) -> Result<()> {
    let (host, port) = split_target(target)?;
    let channel = handle
        .channel_open_direct_tcpip(host, port as u32, "127.0.0.1", peer_port as u32)
        .await
        .map_err(|e| AppError(format!("direct-tcpip 失败: {e}")))?;
    pipe(local.split(), channel.into_stream(), up, down).await
}

async fn socks5_conn(
    handle: Arc<russh::client::Handle<ClientHandler>>,
    mut local: TcpStream,
    peer_port: u16,
    up: Arc<AtomicU64>,
    down: Arc<AtomicU64>,
) -> Result<()> {
    // 握手
    let mut head = [0u8; 2];
    local.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Err(AppError("非 SOCKS5 请求".into()));
    }
    let mut methods = vec![0u8; head[1] as usize];
    local.read_exact(&mut methods).await?;
    local.write_all(&[5, 0]).await?; // no-auth

    // 请求
    let mut req = [0u8; 4];
    local.read_exact(&mut req).await?;
    if req[1] != 1 {
        local.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Err(AppError("仅支持 CONNECT".into()));
    }
    let host = match req[3] {
        1 => {
            let mut ip = [0u8; 4];
            local.read_exact(&mut ip).await?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            local.read_exact(&mut len).await?;
            let mut name = vec![0u8; len[0] as usize];
            local.read_exact(&mut name).await?;
            String::from_utf8_lossy(&name).to_string()
        }
        4 => {
            let mut ip = [0u8; 16];
            local.read_exact(&mut ip).await?;
            std::net::Ipv6Addr::from(ip).to_string()
        }
        _ => return Err(AppError("未知地址类型".into())),
    };
    let mut port_buf = [0u8; 2];
    local.read_exact(&mut port_buf).await?;
    let port = u16::from_be_bytes(port_buf);

    let channel = match handle
        .channel_open_direct_tcpip(&host, port as u32, "127.0.0.1", peer_port as u32)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            local.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            return Err(AppError(format!("SOCKS5 目标不可达 {host}:{port}: {e}")));
        }
    };
    local.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    pipe(local.split(), channel.into_stream(), up, down).await
}

async fn pipe(
    (mut lr, mut lw): (tokio::net::tcp::ReadHalf<'_>, tokio::net::tcp::WriteHalf<'_>),
    remote: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    up: Arc<AtomicU64>,
    down: Arc<AtomicU64>,
) -> Result<()> {
    let (mut rr, mut rw) = tokio::io::split(remote);
    let upload = async {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            let n = lr.read(&mut buf).await?;
            if n == 0 { break; }
            rw.write_all(&buf[..n]).await?;
            up.fetch_add(n as u64, Ordering::Relaxed);
        }
        rw.shutdown().await.ok();
        Ok::<_, std::io::Error>(())
    };
    let download = async {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            let n = rr.read(&mut buf).await?;
            if n == 0 { break; }
            lw.write_all(&buf[..n]).await?;
            down.fetch_add(n as u64, Ordering::Relaxed);
        }
        lw.shutdown().await.ok();
        Ok::<_, std::io::Error>(())
    };
    let _ = tokio::join!(upload, download);
    Ok(())
}

fn split_target(target: &str) -> Result<(&str, u16)> {
    let (h, p) = target
        .rsplit_once(':')
        .ok_or_else(|| AppError(format!("目标格式应为 host:port: {target}")))?;
    Ok((h, p.parse().map_err(|_| AppError(format!("端口无效: {p}")))?))
}
