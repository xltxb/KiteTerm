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
| 同步 | **管理平台管控**:注册/登录后由平台下发 S3 凭据(平台持桶,按用户前缀隔离);桶内仅密文,平台零知识 |
| 管理平台 | `server/`(Rust axum + SQLite):账户、令牌、同步授权开关、凭据下发、管理接口 + 内嵌 Web 管理控制台(`/admin`) |

## 开发

```bash
pnpm install

# 纯前端预览(浏览器 mock 后端,演示数据同原型)
pnpm dev            # http://localhost:1420

# 桌面应用(需 Rust 工具链)
pnpm tauri dev

# 构建
pnpm tauri build

# 管理平台(账户/同步授权/凭据下发;S3 指向任意兼容存储,如本地 MinIO)
VELA_ADMIN_KEY=<管理密钥> \
S3_ENDPOINT=http://127.0.0.1:19000 S3_BUCKET=vela-sync \
S3_ACCESS_KEY=<ak> S3_SECRET_KEY=<sk> \
cargo run --manifest-path server/Cargo.toml
# 应用内「同步」页默认平台地址 http://127.0.0.1:8787,可在“平台设置”修改
# Web 管理控制台:http://127.0.0.1:8787/admin(输入管理密钥)——
#   用户列表/搜索、同步启停开关、强制下线(吊销令牌)、按用户查看存储用量、平台统计
# 管理 API(X-Admin-Key):GET /api/admin/stats、GET /api/admin/users、
#   POST /api/admin/users/{email}/sync {enabled}、POST /api/admin/users/{email}/revoke、
#   GET /api/admin/users/{email}/usage
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

- **F1 终端会话**:多标签、状态点、状态栏(协议/地址/延迟/加密套件/尺寸)、⌘F 搜索输出、断线自动重连(指数退避 1/2/4/8/8s,可在设置关闭)、多行粘贴护栏 ✅
- **F2 主机管理**:分组 + 标签、搜索、一键连接、新建/删除、实时延迟探测(30s TCP 测时)✅
- **F3 文件传输**:SFTP 双栏、上传/下载、队列进度/速率/失败重试、按偏移断点续传 ✅
- **F4 密钥管理**:生成 Ed25519 / RSA4096、导入、指纹、复制公钥、使用统计;密码认证兜底;设备密钥存系统 Keychain(旧文件自动迁移)✅(Touch ID/agent:后续)
- **F5 端口转发**:L / R / D 三类实转发 + 流量统计 + 应用启动自动恢复启用规则 ✅
- **F6 命令片段**:片段库 + 终端右栏快捷面板一键运行 + `{{cursor}}` 占位符停留待编辑 ✅
- **F7 云同步**:**管理平台管控** — 注册/登录后平台下发存储凭据并放开同步,平台可按账户停用;分项开关、设备列表;**oplog 增量同步**(逐条 E2E 加密 + AAD 绑定对象路径、每设备游标增量拉取、实体级 LWW 合并 + lamport 时钟、删除墓碑、分项开关重开自动补发存量);主密码仅本地派生密钥,平台零知识 ✅(30 天压实、STS 临时凭据:后续)
- **F8 会话录制**:asciicast v2 录制(状态栏 REC 计时)+ 内建回放器(播放/暂停/进度/1–4× 倍速)✅
- **F9 命令面板**:⌘K,统一搜索主机/片段/动作 ✅
- **F10 设置**:深/浅主题、终端字号、光标闪烁、行为开关(启动恢复会话 / 断线自动重连 / 退出前确认均已生效)、快捷键表 ✅
- **AI 助手**:右栏对话接入 Claude API(默认 `claude-opus-5`,可换模型;Key 经设备密钥加密存本地);读取当前会话最近输出作为上下文,回复中命令一键「在终端运行」✅

**安全**:known_hosts 指纹校验(TOFU,首连记录;指纹变更即中断连接并告警,需显式「信任新指纹」后重试)。

已知简化(与开发文档 3.x 的差异,均标注于代码注释):
- 同步:manifest 为明文 JSON 且未用 If-Match 乐观锁;30 天 oplog 压实与墓碑清理后续
- SFTP 续传按「目标已有大小」判断偏移;文档 3.2 的 mtime + 前 1MB 哈希校验后续补
- 监控采样命令面向 Linux 远端(/proc);macOS 远端显示离线
- Keychain 不可用的平台回退应用数据目录 0600 文件;恢复短语(BIP39)后续

集成自检(`src-tauri/examples/`):`ssh_check`(真实 SSH 栈 + TOFU)、`sync_check`(双设备同账户,
经管理平台 + 本地 MinIO:注册登录 / 凭据下发 / 初始同步 / LWW / 墓碑 / 幂等重放 / 并发冲突收敛 /
平台停用与恢复管控,桶内均为密文)。
- 同步凭据 v1 为平台共享凭据 + 用户前缀约定;升级 STS 按前缀签发临时凭据为后续硬化项
