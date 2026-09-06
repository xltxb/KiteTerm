//! SFTP:目录列举 + 传输队列(顺序执行、进度事件、失败可重试)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};

use crate::error::{AppError, Result};
use crate::models::{FileEntry, TransferJob, TransferRequest};
use crate::ssh::SessionManager;

pub struct TransferManager {
    pub jobs: Mutex<Vec<TransferJob>>,
    worker_running: AtomicBool,
}

impl Default for TransferManager {
    fn default() -> Self {
        Self { jobs: Mutex::new(Vec::new()), worker_running: AtomicBool::new(false) }
    }
}

async fn open_sftp(manager: &SessionManager, session_id: &str) -> Result<SftpSession> {
    let handle = manager.get_handle(session_id)?;
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| AppError(format!("SFTP 通道失败: {e}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| AppError(format!("SFTP 子系统失败: {e}")))?;
    SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| AppError(format!("SFTP 会话失败: {e}")))
}

pub async fn sftp_list(manager: &SessionManager, session_id: &str, path: &str) -> Result<Vec<FileEntry>> {
    let sftp = open_sftp(manager, session_id).await?;
    let entries = sftp
        .read_dir(path)
        .await
        .map_err(|e| AppError(format!("读取目录 {path} 失败: {e}")))?;
    let mut out: Vec<FileEntry> = entries
        .map(|entry| {
            let meta = entry.metadata();
            let mtime = meta
                .mtime
                .map(|t| {
                    chrono::DateTime::from_timestamp(t as i64, 0)
                        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            FileEntry {
                name: entry.file_name(),
                is_dir: meta.is_dir(),
                size: meta.size.unwrap_or(0),
                modified: mtime,
            }
        })
        .filter(|e| e.name != "." && e.name != "..")
        .collect();
    out.sort_by(|a, b| (!a.is_dir, a.name.to_lowercase()).cmp(&(!b.is_dir, b.name.to_lowercase())));
    Ok(out)
}

pub fn local_list(path: &str) -> Result<Vec<FileEntry>> {
    let expanded = shellexpand_home(path);
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&expanded)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        let modified = meta
            .modified()
            .ok()
            .map(|t| {
                chrono::DateTime::<chrono::Local>::from(t)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        out.push(FileEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            is_dir: meta.is_dir(),
            size: meta.len(),
            modified,
        });
    }
    out.sort_by(|a, b| (!a.is_dir, a.name.to_lowercase()).cmp(&(!b.is_dir, b.name.to_lowercase())));
    Ok(out)
}

pub fn shellexpand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.to_string_lossy(), rest);
        }
    }
    path.to_string()
}

/// 读取流前 n 字节的 SHA-256(断点续传一致性校验)
async fn sha256_prefix<R: tokio::io::AsyncRead + Unpin>(r: &mut R, n: u64) -> Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut remaining = n;
    let mut buf = vec![0u8; 64 * 1024];
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        let got = r.read(&mut buf[..want]).await?;
        if got == 0 {
            break;
        }
        hasher.update(&buf[..got]);
        remaining -= got as u64;
    }
    Ok(hasher.finalize().into())
}

pub(crate) fn emit_job(app: &AppHandle, job: &TransferJob) {
    let _ = app.emit("transfer://progress", job.clone());
}

pub(crate) fn update_job(
    app: &AppHandle,
    jobs: &Mutex<Vec<TransferJob>>,
    id: &str,
    f: impl FnOnce(&mut TransferJob),
) {
    let mut jobs = jobs.lock().unwrap();
    if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
        f(job);
        emit_job(app, job);
    }
}

pub fn enqueue(
    app: AppHandle,
    transfers: Arc<TransferManager>,
    sessions: Arc<SessionManager>,
    req: TransferRequest,
) -> Result<TransferJob> {
    let job = TransferJob {
        id: uuid::Uuid::new_v4().to_string(),
        direction: req.direction.clone(),
        name: req.name.clone(),
        local_path: shellexpand_home(&req.local_path),
        remote_path: req.remote_path.clone(),
        session_id: req.session_id.clone(),
        progress: 0.0,
        rate_bps: 0,
        state: "queued".into(),
        error: None,
        elapsed_ms: 0,
        protocol: "sftp".into(),
    };
    transfers.jobs.lock().unwrap().push(job.clone());
    emit_job(&app, &job);
    kick_worker(app, transfers, sessions);
    Ok(job)
}

pub fn retry(app: AppHandle, transfers: Arc<TransferManager>, sessions: Arc<SessionManager>, id: &str) {
    update_job(&app, &transfers.jobs, id, |j| {
        if j.state == "failed" {
            j.state = "queued".into();
            j.error = None;
            j.progress = 0.0;
        }
    });
    kick_worker(app, transfers, sessions);
}

fn kick_worker(app: AppHandle, transfers: Arc<TransferManager>, sessions: Arc<SessionManager>) {
    if transfers.worker_running.swap(true, Ordering::SeqCst) {
        return; // 已有 worker 在跑
    }
    tokio::spawn(async move {
        loop {
            let next = {
                let mut jobs = transfers.jobs.lock().unwrap();
                match jobs.iter_mut().find(|j| j.state == "queued") {
                    Some(job) => {
                        job.state = "running".into();
                        let j = job.clone();
                        emit_job(&app, &j);
                        Some(j)
                    }
                    None => None,
                }
            };
            let Some(job) = next else { break };
            let result = run_job(&app, &transfers, &sessions, &job).await;
            match result {
                Ok(()) => update_job(&app, &transfers.jobs, &job.id, |j| {
                    j.state = "done".into();
                    j.progress = 100.0;
                    j.rate_bps = 0;
                }),
                Err(e) => update_job(&app, &transfers.jobs, &job.id, |j| {
                    j.state = "failed".into();
                    j.error = Some(e.to_string());
                }),
            }
        }
        transfers.worker_running.store(false, Ordering::SeqCst);
    });
}

async fn run_job(
    app: &AppHandle,
    transfers: &TransferManager,
    sessions: &SessionManager,
    job: &TransferJob,
) -> Result<()> {
    let sftp = open_sftp(sessions, &job.session_id).await?;
    let start = Instant::now();
    let mut moved: u64;
    let mut last_emit = Instant::now();
    let mut buf = vec![0u8; 64 * 1024];

    if job.direction == "upload" {
        let mut src = tokio::fs::File::open(&job.local_path).await?;
        let total = src.metadata().await?.len().max(1);
        // 断点续传:远端已有部分文件(0 < size < total)则从该偏移续写;
        // 续传前比对两端前 1MB 哈希(文档 3.2),不一致视为不同文件,从头重传
        let mut offset = sftp
            .metadata(&job.remote_path)
            .await
            .ok()
            .and_then(|m| m.size)
            .filter(|s| *s > 0 && *s < total)
            .unwrap_or(0);
        if offset > 0 {
            let check = offset.min(1024 * 1024);
            let mut lf = tokio::fs::File::open(&job.local_path).await?;
            let local_hash = sha256_prefix(&mut lf, check).await?;
            let remote_hash = match sftp.open(&job.remote_path).await {
                Ok(mut rf) => sha256_prefix(&mut rf, check).await.ok(),
                Err(_) => None,
            };
            if remote_hash != Some(local_hash) {
                offset = 0;
            }
        }
        let mut dst = if offset > 0 {
            src.seek(SeekFrom::Start(offset)).await?;
            let mut f = sftp
                .open_with_flags(&job.remote_path, OpenFlags::WRITE)
                .await
                .map_err(|e| AppError(format!("远端打开 {} 失败: {e}", job.remote_path)))?;
            f.seek(SeekFrom::Start(offset))
                .await
                .map_err(|e| AppError(format!("远端定位失败: {e}")))?;
            f
        } else {
            sftp.create(&job.remote_path)
                .await
                .map_err(|e| AppError(format!("远端创建 {} 失败: {e}", job.remote_path)))?
        };
        moved = offset;
        loop {
            let n = src.read(&mut buf).await?;
            if n == 0 { break; }
            dst.write_all(&buf[..n])
                .await
                .map_err(|e| AppError(format!("写入远端失败: {e}")))?;
            moved += n as u64;
            maybe_emit(app, transfers, &job.id, moved, total, start, &mut last_emit);
        }
        dst.shutdown().await.ok();
    } else {
        let mut src = sftp
            .open(&job.remote_path)
            .await
            .map_err(|e| AppError(format!("打开远端 {} 失败: {e}", job.remote_path)))?;
        let total = sftp
            .metadata(&job.remote_path)
            .await
            .ok()
            .and_then(|m| m.size)
            .unwrap_or(0)
            .max(1);
        // 断点续传:本地已有部分文件则从该偏移续传;先比对两端前 1MB 哈希
        let mut offset = tokio::fs::metadata(&job.local_path)
            .await
            .ok()
            .map(|m| m.len())
            .filter(|s| *s > 0 && *s < total)
            .unwrap_or(0);
        if offset > 0 {
            let check = offset.min(1024 * 1024);
            let mut lf = tokio::fs::File::open(&job.local_path).await?;
            let local_hash = sha256_prefix(&mut lf, check).await?;
            let remote_hash = match sftp.open(&job.remote_path).await {
                Ok(mut rf) => sha256_prefix(&mut rf, check).await.ok(),
                Err(_) => None,
            };
            if remote_hash != Some(local_hash) {
                offset = 0;
            }
        }
        let mut dst = if offset > 0 {
            src.seek(SeekFrom::Start(offset))
                .await
                .map_err(|e| AppError(format!("远端定位失败: {e}")))?;
            let mut f = tokio::fs::OpenOptions::new().write(true).open(&job.local_path).await?;
            f.seek(SeekFrom::Start(offset)).await?;
            f
        } else {
            tokio::fs::File::create(&job.local_path).await?
        };
        moved = offset;
        loop {
            let n = src
                .read(&mut buf)
                .await
                .map_err(|e| AppError(format!("读取远端失败: {e}")))?;
            if n == 0 { break; }
            dst.write_all(&buf[..n]).await?;
            moved += n as u64;
            maybe_emit(app, transfers, &job.id, moved, total, start, &mut last_emit);
        }
        dst.flush().await?;
    }
    Ok(())
}

fn maybe_emit(
    app: &AppHandle,
    transfers: &TransferManager,
    id: &str,
    moved: u64,
    total: u64,
    start: Instant,
    last_emit: &mut Instant,
) {
    if last_emit.elapsed().as_millis() < 200 {
        return;
    }
    *last_emit = Instant::now();
    let elapsed = start.elapsed();
    let rate = if elapsed.as_secs_f64() > 0.0 {
        (moved as f64 / elapsed.as_secs_f64()) as u64
    } else {
        0
    };
    update_job(app, &transfers.jobs, id, |j| {
        j.progress = (moved as f64 / total as f64 * 100.0).min(99.9);
        j.rate_bps = rate;
        j.elapsed_ms = elapsed.as_millis() as u64;
    });
}
