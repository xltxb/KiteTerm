# Vela Term

自用优先的桌面 SSH 终端:远程连接、SFTP 传输、密钥管理、端口转发、服务器监控与端到端加密云同步。
本仓库为《Vela Term 产品需求与开发文档》与原型 `design/Vela Term.dc.html` 的实现(设计系统:Vela Design System)。

## 技术栈

| 层 | 选型 |
|---|---|
| 壳 | Tauri 2 |
| UI | React 18 + TypeScript + Vite |
| 终端 | xterm.js(WebGL 渲染 + search/fit addon) |
| 状态 | Zustand(每域一个 store) |
| 样式 | Vela Design System tokens(CSS variables,`data-theme` 切换深/浅) |
| SSH | russh + russh-sftp(Rust) |
| 存储 | SQLite(rusqlite,配置/片段/密钥/录制索引) |
| 加密 | XChaCha20-Poly1305 + Argon2id(密钥落库、同步 E2E) |
| 同步 | 用户自备 S3 兼容桶(AWS S3 / R2 / MinIO),桶内仅密文 |

## 开发

```bash
pnpm install

# 纯前端预览(浏览器 mock 后端,演示数据同原型)
pnpm dev            # http://localhost:1420

# 桌面应用(需 Rust 工具链)
pnpm tauri dev

# 构建
pnpm tauri build
```

## 目录结构

```
design/            # 原型(Vela Term.dc.html)与 Vela 设计系统 tokens
src/
  app/             # 布局:Sidebar、CommandPalette(⌘K)
  screens/         # 终端 / 主机 / 传输 / 转发 / 监控 / 密钥 / 片段 / 同步 / 设置
  components/      # Button、Switch、Badge 等基础组件(对齐 Vela DS)
  stores/          # ui / data / session / transfer / sync / monitor
  ipc/             # IPC 契约 + Tauri 实现 + 浏览器 mock
  theme/           # Vela tokens(colors/typography/spacing/effects/motion)
src-tauri/src/
  ssh.rs           # russh 会话、PTY、输出事件流
  sftp.rs          # SFTP 列目录 + 传输队列(进度事件、重试)
  forward.rs       # L 本地转发 / D 动态 SOCKS5(实时流量统计)
  vault.rs         # 密钥生成(Ed25519/RSA4096)/导入/指纹,私钥加密落库
  monitor.rs       # 经 SSH 采样 CPU/MEM/DISK/负载/网络(无需 agent)
  recorder.rs      # asciicast v2 会话录制
  sync.rs          # S3 兼容桶 E2E 加密快照同步
  db.rs            # SQLite 模型(hosts/keys/snippets/forwards/recordings)
```

## 功能对照(PRD F1–F10)

- **F1 终端会话**:多标签、状态点、状态栏(协议/地址/延迟/加密套件/尺寸)、⌘F 搜索输出 ✅
- **F2 主机管理**:分组 + 标签、搜索、一键连接、新建/删除 ✅
- **F3 文件传输**:SFTP 双栏、上传/下载、队列进度/速率/重试 ✅(断点续传:后续)
- **F4 密钥管理**:生成 Ed25519 / RSA4096、导入、指纹、复制公钥、使用统计 ✅(Touch ID/agent:后续)
- **F5 端口转发**:L / D 实转发 + 流量统计 ✅(R 远程转发:后续)
- **F6 命令片段**:片段库 + 终端右栏快捷面板一键运行 ✅({{cursor}} 占位:后续)
- **F7 云同步**:S3 兼容桶配置、分项开关、E2E 加密快照推拉、设备列表 ✅(oplog 增量/压实:后续)
- **F8 会话录制**:asciicast v2 录制(状态栏 REC 计时)✅(回放器:后续)
- **F9 命令面板**:⌘K,统一搜索主机/片段/动作 ✅
- **F10 设置**:深/浅主题、终端字号、光标闪烁、行为开关、快捷键表 ✅

已知简化(与开发文档 3.x 的差异,均标注于代码注释):
- known_hosts 指纹校验暂未实现(`ssh.rs` TODO)
- 设备密钥存应用数据目录 0600 文件,待迁移系统 Keychain
- 同步为快照式 LWW,oplog 增量与 30 天压实见文档 3.5,后续实现
- 监控采样命令面向 Linux 远端(/proc);macOS 远端显示离线
