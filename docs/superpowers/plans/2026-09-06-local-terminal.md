# 本地终端 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 KiteTerm 桌面端新增「本地终端」会话:本机 PTY 跑默认 shell,复用全部终端能力,四处入口可开。

**Architecture:** Rust 侧新增 `local.rs`,用 `portable-pty` 起 PTY,输出走既有 `term://data` 事件、输入走既有 `Ctl` 通道,所以 write/resize/close/录制命令零改动。`SessionInfo` 加 `kind` 区分 ssh/local,`SessionEntry.handle` 变为 `Option`,依赖 SSH handle 的模块跳过本地会话。前端 store 把「创建 xterm 并挂接」抽成 `attach`,新增 `openLocal`,四个入口都调它。

**Tech Stack:** Rust / Tauri 2 / portable-pty 0.9 / React 18 + Zustand / xterm.js

**Spec:** `docs/superpowers/specs/2026-09-06-local-terminal-design.md`

## Global Constraints

- 所有面向用户的文案用中文;代码注释中文、标识符英文(仓库惯例)。
- 组件里不写内联样式,复用 `@/components/ui` 的 `Button`。
- 本地会话:`kind = "local"`、`host_id = "local"`、`host_name = "本地终端"`、`addr = "localhost"`、`port = 0`、`cipher = ""`、`latency_ms = None`。
- shell:非 Windows 读 `SHELL` 缺省 `/bin/zsh` 并加 `-l`;Windows 用 `powershell.exe`。环境追加 `TERM=xterm-256color`、`COLORTERM=truecolor`。初始尺寸 120×32。
- 本地会话不参与启动恢复、SFTP、转发、监控。
- 提交信息结尾附带:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_0148LdwcwZw8wjtea3koVBC6
  ```
- 工作目录:`desktop/`。Rust 命令在 `desktop/src-tauri/` 下跑 `cargo`。

---

### Task 1: 会话模型支持本地会话(kind 字段与可选 handle)

**Files:**
- Modify: `desktop/src-tauri/src/models.rs:96-112`(`SessionInfo`)
- Modify: `desktop/src-tauri/src/ssh.rs:147-153`(`SessionEntry`)、`:164-173`(emit 函数可见性)、`:176-182`(`get_handle`)、`:189-196`(`update_info`)、`:332-347`(构造 `SessionInfo`)、`:373-380`(插入 `SessionEntry`)
- Modify: `desktop/src-tauri/src/forward.rs:44-49`
- Modify: `desktop/src-tauri/src/monitor.rs:116-127`
- Modify: `desktop/src-tauri/src/commands.rs:542-552`(`persist_open_sessions`)

**Interfaces:**
- Produces: `SessionInfo.kind: String`;`SessionEntry.handle: Option<Arc<Mutex<Arc<Handle<ClientHandler>>>>>`;`ssh::emit_data / emit_line / emit_state` 与 `SessionManager::update_info` 变为 `pub(crate)`。

- [ ] **Step 1: 写 serde 缺省值的失败测试**

在 `desktop/src-tauri/src/models.rs` 文件末尾追加:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_info_kind_defaults_to_ssh() {
        let json = r#"{"id":"1","hostId":"h","hostName":"n","user":"u","addr":"a","port":22,
            "state":"connected","latencyMs":null,"cipher":"c","cols":80,"rows":24,
            "recording":false,"recordingSince":null}"#;
        let info: SessionInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.kind, "ssh");
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd desktop/src-tauri && cargo test session_info_kind_defaults_to_ssh`
Expected: 编译错误 `no field kind`

- [ ] **Step 3: 给 `SessionInfo` 加 `kind`**

在 `models.rs` 的 `SessionInfo` 中 `recording_since` 之后追加:

```rust
    /// 会话类型:ssh | local
    #[serde(default = "default_kind_ssh")]
    pub kind: String,
```

在 `SessionInfo` 定义之后追加:

```rust
fn default_kind_ssh() -> String {
    "ssh".into()
}
```

在 `ssh.rs` `open_shell_session` 构造 `SessionInfo` 处(`recording_since: None,` 之后)加 `kind: "ssh".into(),`。

- [ ] **Step 4: `SessionEntry.handle` 改为 Option,放开辅助函数可见性**

`ssh.rs`:

```rust
pub struct SessionEntry {
    pub info: SessionInfo,
    pub ctl: mpsc::UnboundedSender<Ctl>,
    /// SSH 会话持有连接句柄(重连后被替换,取用时克隆内层 Arc);本地终端为 None
    pub handle: Option<Arc<Mutex<Arc<Handle<ClientHandler>>>>>,
    pub recorder: Arc<Mutex<Option<Recorder>>>,
}
```

`fn emit_state` / `fn emit_data` / `fn emit_line` 三个函数改为 `pub(crate) fn`;`fn update_info` 改为 `pub(crate) fn update_info`。

`get_handle` 改为:

```rust
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
```

`open_shell_session` 插入 `SessionEntry` 处改为 `handle: Some(shared_handle.clone()),`。

- [ ] **Step 5: 转发、监控、恢复列表跳过本地会话**

`forward.rs:44-49`:

```rust
        s.values()
            .find(|e| e.info.host_id == host_id && e.info.state == "connected" && e.handle.is_some())
            .map(|e| e.handle.as_ref().unwrap().lock().unwrap().clone())
```

`monitor.rs:116-127`:

```rust
                s.values()
                    .filter(|e| e.info.state == "connected")
                    .filter_map(|e| {
                        let handle = e.handle.as_ref()?.lock().unwrap().clone();
                        Some((e.info.host_id.clone(), (e.info.host_name.clone(), handle)))
                    })
                    .collect()
```

`commands.rs` `persist_open_sessions`:

```rust
    let hosts: Vec<String> = state
        .sessions
        .list()
        .into_iter()
        .filter(|s| s.kind == "ssh")
        .map(|s| s.host_id)
        .collect();
```

- [ ] **Step 6: 运行测试与构建**

Run: `cd desktop/src-tauri && cargo test session_info_kind_defaults_to_ssh && cargo build`
Expected: 测试 PASS,构建成功(允许既有 dead_code 警告)

- [ ] **Step 7: 提交**

```bash
git add desktop/src-tauri/src/models.rs desktop/src-tauri/src/ssh.rs desktop/src-tauri/src/forward.rs desktop/src-tauri/src/monitor.rs desktop/src-tauri/src/commands.rs
git commit -m "refactor(desktop): 会话模型加 kind 字段,SSH 句柄改为可选以容纳本地终端"
```

---

### Task 2: 本地 PTY 会话后端与命令

**Files:**
- Modify: `desktop/src-tauri/Cargo.toml:15-45`(依赖)
- Create: `desktop/src-tauri/src/local.rs`
- Modify: `desktop/src-tauri/src/lib.rs:12-13`(mod)、`:73-95`(handler 列表)
- Modify: `desktop/src-tauri/src/commands.rs:207-222`(新增命令)

**Interfaces:**
- Consumes: Task 1 的 `SessionEntry.handle: Option`、`pub(crate)` 的 emit 函数与 `update_info`。
- Produces: `local::shell_program(shell_env: Option<&str>) -> String`;`local::open_local_session(app: AppHandle, manager: Arc<SessionManager>) -> Result<SessionInfo>`;Tauri 命令 `local_session_open`。

- [ ] **Step 1: 加依赖**

`Cargo.toml` `[dependencies]` 里 `whoami = "1"` 之后追加:

```toml
# 本地终端的伪终端(macOS openpty / Windows ConPTY)
portable-pty = "0.9"
```

- [ ] **Step 2: 写 shell 选择的失败测试**

创建 `desktop/src-tauri/src/local.rs`,先只放测试:

```rust
//! 本地终端:在本机起伪终端跑默认 shell,输出与输入复用 SSH 会话的事件与控制通道。

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
```

在 `lib.rs` `mod sftp;` 之前加一行 `mod local;`。

- [ ] **Step 3: 运行测试确认失败**

Run: `cd desktop/src-tauri && cargo test shell_program`
Expected: 编译错误 `cannot find function shell_program`

- [ ] **Step 4: 实现 `local.rs`**

把 `local.rs` 的测试模块之前补成:

```rust
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
            ctl: ctl_tx,
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
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
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
            }
            emit_line(&app, &id, "[进程已退出]");
        });
    }

    // 控制任务:前端输入 → PTY;改尺寸;关闭时结束子进程
    tokio::spawn(async move {
        while let Some(ctl) = ctl_rx.recv().await {
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
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        drop(master);
    });

    Ok(info)
}
```

- [ ] **Step 5: 注册 Tauri 命令**

`commands.rs` 在 `session_open` 之后追加:

```rust
/// 打开本地终端(本机 PTY 跑默认 shell),不参与启动恢复
#[tauri::command]
pub fn local_session_open(app: AppHandle, state: State<AppState>) -> Result<SessionInfo> {
    crate::local::open_local_session(app, state.sessions.clone())
}
```

`lib.rs` handler 列表中 `commands::session_open,` 之后加 `commands::local_session_open,`。

- [ ] **Step 6: 测试与构建**

Run: `cd desktop/src-tauri && cargo test shell_program && cargo build`
Expected: 两个测试 PASS,构建成功

- [ ] **Step 7: 提交**

```bash
git add desktop/src-tauri/Cargo.toml desktop/src-tauri/Cargo.lock desktop/src-tauri/src/local.rs desktop/src-tauri/src/lib.rs desktop/src-tauri/src/commands.rs
git commit -m "feat(desktop): 本地终端会话后端,基于 portable-pty 复用会话事件与控制通道"
```

---

### Task 3: 前端 ipc 与 session store 的 `openLocal`

**Files:**
- Modify: `desktop/src/types.ts:80-94`
- Modify: `desktop/src/ipc/api.ts:66-67`
- Modify: `desktop/src/ipc/tauri.ts:40`
- Modify: `desktop/src/stores/session.ts:84-90`(接口)、`:136-179`(`open`)

**Interfaces:**
- Consumes: Tauri 命令 `local_session_open`。
- Produces: `ipc.openLocalSession(): Promise<SessionInfo>`;`useSessionStore().openLocal(): Promise<string>`;`SessionInfo.kind: "ssh" | "local"`。

- [ ] **Step 1: 类型与 ipc**

`types.ts` `SessionInfo` 中 `recordingSince` 之后加 `kind: "ssh" | "local";`。

`api.ts` 在 `openSession` 声明之后加:

```ts
  /** 打开本地终端(本机默认 shell) */
  openLocalSession(): Promise<SessionInfo>;
```

`tauri.ts` 在 `openSession` 映射之后加:

```ts
    openLocalSession: () => invoke("local_session_open"),
```

- [ ] **Step 2: store 抽出 `attach`,新增 `openLocal`**

`session.ts` 接口 `open` 之后加:

```ts
  /** 打开本地终端 */
  openLocal: () => Promise<string>;
```

在 `export const useSessionStore = create<SessionState>((set, get) => ({` 这一行之前插入:

```ts
/** 为一条已由后端打开的会话创建 xterm、挂接输入与尺寸,并登记到 store */
function attach(
  info: SessionInfo,
  set: (fn: (s: SessionState) => Partial<SessionState>) => void,
  get: () => SessionState,
): string {
  // 此处放原 open 中从 `const term = new Terminal({` 到 `terminals.set(info.id, term);` 的全部代码,原样搬入
  set((s) => ({ sessions: [...s.sessions, info], activeId: info.id }));
  return info.id;
}
```

把原 `open` 里 `const term = new Terminal({ ... });` 直到 `terminals.set(info.id, term);` 的代码整段剪切进 `attach`(其中 `info.id` 与 `get()`、`set(...)` 的引用不用改)。然后 `open` 变为:

```ts
  open: async (hostId, password) => {
    get().wire();
    const info = await ipc.openSession(hostId, password);
    return attach(info, set, get);
  },

  openLocal: async () => {
    get().wire();
    let info: SessionInfo;
    try {
      info = await ipc.openLocalSession();
    } catch (e) {
      // PTY 或 shell 启动失败:弹原生提示,不抛给调用方(入口都是 fire-and-forget)
      void showError(String(e), "打开本地终端失败");
      return "";
    }
    return attach(info, set, get);
  },
```

`session.ts` 顶部 `import { confirmAction } from "@/lib/dialog";` 改为 `import { confirmAction, showError } from "@/lib/dialog";`。

- [ ] **Step 3: 类型检查**

Run: `cd desktop && npx tsc -b`
Expected: 无错误

- [ ] **Step 4: 提交**

```bash
git add desktop/src/types.ts desktop/src/ipc/api.ts desktop/src/ipc/tauri.ts desktop/src/stores/session.ts
git commit -m "feat(desktop): session store 支持打开本地终端,xterm 挂接逻辑抽成 attach"
```

---

### Task 4: 四个入口与展示差异

**Files:**
- Modify: `desktop/src/screens/TerminalScreen.tsx:1-13`(图标导入)、`:33`(store)、`:92-95`(⌘T)、`:135-137`(「+」)、`:200-209`(空状态)、`:215-230`(状态栏)
- Modify: `desktop/src/app/CommandPalette.tsx:1-11`(图标)、`:31`(store)、`:41`(new-tab)
- Modify: `desktop/src/screens/HostsScreen.tsx:69-74`
- Modify: `desktop/src/screens/TransferScreen.tsx:162,220`

**Interfaces:**
- Consumes: `useSessionStore().openLocal`、`SessionInfo.kind`。

- [ ] **Step 1: TerminalScreen**

导入处 lucide 列表加 `SquareTerminal` 已存在,无需新增。第 33 行 `const { sessions, activeId, setActive, close, toggleRecording } = useSessionStore();` 改为 `const { sessions, activeId, setActive, close, toggleRecording, openLocal } = useSessionStore();`

⌘T 分支改为:

```ts
      } else if (mod && (e.key === "t" || e.key === "T")) {
        e.preventDefault();
        void openLocal();
```

并把该 `useEffect` 依赖数组中的 `setPalette` 换成 `openLocal`。

标签栏「+」改为:

```tsx
        <div className="tab-action mb-0" title="新建本地终端(⌘T)" onClick={() => void openLocal()}>
          <Plus size={15} />
        </div>
```

空状态按钮区改为:

```tsx
              <div className="row gap-8">
                <Button variant="outline" size="sm" onClick={() => void openLocal()}>
                  打开本地终端
                </Button>
                <Button variant="primary" size="sm" onClick={() => go("hosts")}>
                  浏览主机
                </Button>
              </div>
```

(`row` 与 `gap-8` 均已在 `desktop/src/theme/utilities.css` 定义。)

状态栏 `active ? (<>...</>)` 内前三个 span 改为按 kind 分支:

```tsx
            {active.kind === "local" ? (
              <>
                <span className="grp t-accent">
                  <SquareTerminal size={12} />
                  本地
                </span>
                <span>{active.user}@localhost</span>
              </>
            ) : (
              <>
                <span className="grp t-accent">
                  <Lock size={12} />
                  SSH
                </span>
                <span>
                  {active.user}@{active.addr}:{active.port}
                </span>
                <span className="grp">
                  <span className={active.state === "connected" ? "dot-sm dot-ok" : "dot-sm dot-warn"} />
                  {active.latencyMs != null ? `${active.latencyMs}ms` : "—"}
                </span>
                <span>{active.cipher}</span>
              </>
            )}
```

- [ ] **Step 2: CommandPalette**

图标导入加 `SquareTerminal`;`const openSession = ...` 之后加 `const openLocal = useSessionStore((s) => s.openLocal);`;`new-tab` 命令改为:

```tsx
      { id: "new-tab", icon: <SquareTerminal size={16} />, label: "打开本地终端", hint: "⌘T", keywords: "local shell new tab", run: () => { go("terminal"); void openLocal(); } },
```

并把 `useMemo` 依赖数组加上 `openLocal`。`Plus` 若不再使用则从导入中删掉。

- [ ] **Step 3: HostsScreen**

导入 `useSessionStore` 处已有 `open`;再取 `openLocal`(按文件既有取法,例如 `const openLocal = useSessionStore((s) => s.openLocal);`)与 `go`(`useUiStore`)。工具栏「管理分组」之后、「新建主机」之前加:

```tsx
          <Button variant="outline" onClick={() => { go("terminal"); void openLocal(); }}>
            <SquareTerminal size={14} />本地终端
          </Button>
```

lucide 导入加 `SquareTerminal`。

- [ ] **Step 4: TransferScreen 只列 SSH 会话**

第 162 行改为:

```ts
  const sessions = useSessionStore((s) => s.sessions.filter((x) => x.kind === "ssh"));
```

若该写法触发 zustand 每次渲染新数组的告警或死循环,改为保留原行并在第 220 行 `sessions.map` 前 `.filter((sn) => sn.kind === "ssh")`。

- [ ] **Step 5: 构建**

Run: `cd desktop && npm run build`
Expected: 成功

- [ ] **Step 6: 提交**

```bash
git add desktop/src/screens/TerminalScreen.tsx desktop/src/app/CommandPalette.tsx desktop/src/screens/HostsScreen.tsx desktop/src/screens/TransferScreen.tsx
git commit -m "feat(desktop): 本地终端四处入口(标签栏 +、⌘T/⌘K、主机页、空状态)与状态栏展示"
```

---

### Task 5: 端到端验证

**Files:** 无代码改动。

- [ ] **Step 1: 全量构建与测试**

Run: `cd desktop/src-tauri && cargo test && cd .. && npm test && npm run build`
Expected: 全部 PASS

- [ ] **Step 2: 在运行中的 `pnpm tauri dev` 实例里手动验证**

Rust 改动会触发 Tauri 自动重编译重启。依次:标签栏「+」、⌘T、⌘K 输入 local 回车、主机页「本地终端」、关掉所有标签后空状态按钮,各开一个本地终端;在任一标签输入 `echo hi` 回车看到输出;输入 `exit` 回车看到灰色「[进程已退出]」且标签状态点变灰;⌘W 关闭该标签;传输页会话下拉不出现「本地终端」。

- [ ] **Step 3: 记录结果**

把手动验证结果如实写进最终回复;若有失败项,回到对应任务修复后重新验证。
