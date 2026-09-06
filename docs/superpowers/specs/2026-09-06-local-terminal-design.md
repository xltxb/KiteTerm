# 本地终端 设计

日期:2026-09-06
状态:已确认

## 目标

在 KiteTerm 里新增「本地终端」会话类型:不经 SSH,直接在本机起一个伪终端(PTY)运行用户的默认 shell,
与 SSH 会话共用标签栏、xterm 渲染、录制、AI 辅助等全部终端能力。

## 非目标

- 不做本地会话的启动时恢复(`restore_sessions` 只管 SSH 会话)。
- 不给本地会话提供 SFTP、端口转发、监控采样。
- 不做多 shell 选择 UI,直接使用系统默认 shell。

## 后端(`desktop/src-tauri`)

### 新模块 `local.rs`

- 依赖 `portable-pty` crate(macOS 走 POSIX openpty,Windows 走 ConPTY)。
- shell 选择:macOS 读环境变量 `SHELL`,为空则 `/bin/zsh`;Windows 用 `powershell.exe`。
  作为登录 shell 启动(macOS 传 `-l`),工作目录为用户主目录。
- 环境变量:继承当前进程环境,追加 `TERM=xterm-256color`、`COLORTERM=truecolor`。
- 尺寸初值与 SSH 一致:120×32。
- 输出读取在 `tokio::task::spawn_blocking` 里循环 `read`,每块数据调用 `ssh::emit_data` 发 `term://data`,
  并写入 Recorder(与 SSH 会话相同)。
- 控制消息复用 `ssh::Ctl`:`Data` 写入 PTY master;`Resize` 调 `resize`;`Close` 杀掉子进程并回收。
- 子进程退出(读到 EOF 或 `try_wait` 返回)后:会话状态置 `disconnected`,通过 `emit_line` 打印「[进程已退出]」,
  不走自动重连。
- `pub async fn open_local_session(app, manager) -> Result<SessionInfo>`:创建 PTY、登记 `SessionEntry`、
  `emit_state`、起读取任务,返回 `SessionInfo`。

### 数据模型

- `SessionInfo` 新增 `kind: String`,取值 `"ssh"` / `"local"`,`#[serde(default = "default_kind_ssh")]`。
- 本地会话字段:`host_id = "local"`,`host_name = "本地终端"`,`user = whoami::username()`,
  `addr = "localhost"`,`port = 0`,`latency_ms = None`,`cipher = ""`。
- `SessionEntry.handle` 改为 `Option<Arc<Mutex<Arc<Handle<ClientHandler>>>>>`,本地会话为 `None`。
- `SessionManager::get_handle` 遇到 `None` 返回 `AppError("本地终端不支持此操作")`。
- `forward.rs` 与 `monitor.rs` 遍历会话时过滤掉 `handle` 为 `None` 的条目。

### 命令

- 新增 `local_session_open() -> SessionInfo`,注册到 `lib.rs` 的 handler 列表。
- `session_write` / `session_resize` / `session_close` / 录制命令不变,通过 `Ctl` 通道自然覆盖本地会话。

## 前端(`desktop/src`)

### ipc

- `api.ts` 增加 `openLocalSession(): Promise<SessionInfo>`;`tauri.ts` 映射到 `local_session_open`。
- `types.ts` 的 `SessionInfo` 增加 `kind: "ssh" | "local"`。

### session store

- 把 `open` 中「创建 xterm、挂 onData/onResize、登记 terminals、追加 sessions 并激活」抽成内部函数 `attach(info)`。
- 新增 `openLocal(): Promise<string>`:`wire()` → `ipc.openLocalSession()` → `attach(info)`。
- 现有 `open(hostId, password)` 改为调用 `attach`。

### 入口(四处)

1. `TerminalScreen` 标签栏「+」:`onClick` 改为 `openLocal()`;⌘T 快捷键同样改为 `openLocal()`。
2. `CommandPalette`:`new-tab` 命令改为「新建本地终端」并调用 `openLocal()`,关键词 `local shell tab`。
3. `HostsScreen` 顶部工具栏:「新建主机」左侧新增 outline 按钮「本地终端」。
4. `TerminalScreen` 空状态:「浏览主机」旁新增 outline 按钮「打开本地终端」。

### 展示差异

- 状态栏:`kind === "local"` 时左侧显示「本地」标识与 `{user}@localhost`,不显示 SSH 锁图标、延迟、密码套件;
  尺寸与编码照旧。不展示 shell 名,不为此新增字段。
- 标签:沿用 `host_name`(「本地终端」)与状态点。
- 传输页、监控页、转发页的会话下拉只列 `kind === "ssh"` 的会话。

## 错误处理

- PTY 创建或 shell 启动失败:命令返回 `AppError`,前端走现有的 `showError` 原生提示。
- shell 退出:标签保留、状态 `disconnected`,用户自行关闭;此时 `session_write` 返回「会话已关闭」。

## 测试

- Rust:`local.rs` 内对 shell 选择与环境构造的纯函数写单元测试(`cargo test`)。
- 构建:`cargo build`、`npm run build` 通过。
- 手动:在运行中的客户端点四个入口各开一次本地终端,输入 `echo hi`、`exit`,确认输出、退出提示与关闭行为。
