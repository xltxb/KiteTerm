//! 本地终端:在本机起伪终端跑默认 shell,输出与输入复用 SSH 会话的事件与控制通道。

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tauri::AppHandle;
use tokio::sync::mpsc;

use crate::error::{AppError, Result};
use crate::models::SessionInfo;
use crate::recorder::Recorder;
use crate::ssh::{emit_data, emit_line, emit_state, Ctl, SessionEntry, SessionManager};

/// 选择本地 shell:非 Windows 读 SHELL、缺省 /bin/zsh;Windows 固定 powershell
pub fn shell_program(shell_env: Option<&str>) -> String {
    if cfg!(windows) {
        return "powershell.exe".into();
    }
    match shell_env {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => "/bin/zsh".into(),
    }
}

/// 以登录 shell 启动,工作目录为主目录,终端能力变量按 xterm 真彩声明
fn build_command(shell: &str, home: Option<&Path>) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(shell);
    if !cfg!(windows) {
        cmd.arg("-l");
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    if let Some(h) = home {
        cmd.cwd(h);
    }
    cmd
}

fn pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }
}

/// 打开一个本地终端会话:创建 PTY、登记到会话表、起读取线程与控制任务
pub fn open_local_session(app: AppHandle, manager: Arc<SessionManager>) -> Result<SessionInfo> {
    let (cols, rows) = (120u16, 32u16);
    let pair = native_pty_system()
        .openpty(pty_size(cols, rows))
        .map_err(|e| AppError(format!("创建伪终端失败: {e}")))?;
    let shell = shell_program(std::env::var("SHELL").ok().as_deref());
    let home = dirs::home_dir();
    let mut child = pair
        .slave
        .spawn_command(build_command(&shell, home.as_deref()))
        .map_err(|e| AppError(format!("启动 shell 失败: {e}")))?;
    drop(pair.slave);
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AppError(format!("读取伪终端失败: {e}")))?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| AppError(format!("写入伪终端失败: {e}")))?;
    let master = pair.master;

    let id = uuid::Uuid::new_v4().to_string();
    let info = SessionInfo {
        id: id.clone(),
        host_id: "local".into(),
        host_name: "本地终端".into(),
        user: whoami::username(),
        addr: "localhost".into(),
        port: 0,
        state: "connected".into(),
        latency_ms: None,
        cipher: String::new(),
        cols,
        rows,
        recording: false,
        recording_since: None,
        kind: "local".into(),
    };

    let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel::<Ctl>();
    let recorder: Arc<Mutex<Option<Recorder>>> = Arc::new(Mutex::new(None));
    manager.sessions.lock().unwrap().insert(
        id.clone(),
        SessionEntry {
            info: info.clone(),
            ctl: ctl_tx.clone(),
            handle: None,
            recorder: recorder.clone(),
        },
    );
    emit_state(&app, &info);

    // 读取线程:PTY 输出 → 前端事件(+ 录制);读到 EOF/错误即 shell 已退出
    {
        let app = app.clone();
        let id = id.clone();
        let manager = manager.clone();
        let ctl_tx = ctl_tx.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    // 被信号打断不是 shell 退出,重试;否则退出循环会向控制线程发 Close 把活着的 shell 杀掉
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                    Ok(n) => {
                        emit_data(&app, &id, &buf[..n]);
                        if let Some(rec) = recorder.lock().unwrap().as_mut() {
                            rec.write_output(&buf[..n]);
                        }
                    }
                }
            }
            if let Some(info) = manager.update_info(&id, |i| i.state = "disconnected".into()) {
                emit_state(&app, &info);
                emit_line(&app, &id, "[进程已退出]");
            }
            // shell 已自行退出:主动通知控制线程收尾,避免子进程与 master fd 一直挂着
            // (前端后续调用 session_close 会再发一次 Close,那时 receiver 多半已丢弃,Err 被忽略)
            let _ = ctl_tx.send(Ctl::Close);
        });
    }

    // 控制线程:前端输入 → PTY;改尺寸;关闭时结束子进程。
    // 用阻塞线程而非 tokio::spawn:master fd 是阻塞的(portable-pty 0.9 在 unix 上只设了
    // cloexec、没设 O_NONBLOCK),write_all 在 PTY 输入缓冲区满时会阻塞;child.kill()
    // 内部还会 sleep 到 ~200ms、child.wait() 是阻塞 waitpid——都不能占用 tokio worker。
    std::thread::spawn(move || {
        while let Some(ctl) = ctl_rx.blocking_recv() {
            match ctl {
                Ctl::Data(bytes) => {
                    if writer.write_all(&bytes).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
                Ctl::Resize(c, r) => {
                    let _ = master.resize(pty_size(c, r));
                }
                Ctl::Close => break,
                // 本地会话不涉及 zmodem 传输,忽略
                Ctl::SendFiles(_) | Ctl::ReceiveInto(_) | Ctl::DropUpload(_) | Ctl::CancelTransfer => {}
                // 本地会话暂不支持 AI 分步执行,直接丢掉 —— sender 一 drop,
                // 等在另一头的 exec_step 就会收到 AGENT_STEP_ABORTED
                Ctl::AgentStep(..) | Ctl::AgentCancel | Ctl::AgentReset => {}
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        drop(master);
    });

    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_program_prefers_shell_env() {
        if cfg!(windows) {
            assert_eq!(shell_program(Some("/bin/bash")), "powershell.exe");
        } else {
            assert_eq!(shell_program(Some("/bin/bash")), "/bin/bash");
        }
    }

    #[test]
    fn shell_program_falls_back_when_env_missing_or_blank() {
        let expect = if cfg!(windows) { "powershell.exe" } else { "/bin/zsh" };
        assert_eq!(shell_program(None), expect);
        assert_eq!(shell_program(Some("   ")), expect);
    }
}
