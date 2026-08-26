# KiteTerm

自用优先的桌面 SSH 终端:远程连接、SFTP 传输、密钥管理、端口转发、服务器监控与端到端加密云同步。
本仓库为《KiteTerm 产品需求与开发文档》与原型 `design/Vela Term.dc.html` 的实现(设计系统:Vela Design System)。

## 形态

| 组件 | 形态 | 说明 |
|---|---|---|
| **KiteTerm(APP)** | **macOS / Windows 桌面端** | 无 Web 版本 —— SSH/SFTP/密钥/同步能力均由本地 Rust 核心提供,前端只在 Tauri WebView 内运行;直接用浏览器打开开发服务器会显示提示页而非应用 |
| **管理平台** | **Web** | `server/` 提供管理控制台(`/admin`)与账户、授权、凭据下发接口 |

## 技术栈

| 层 | 选型 |
|---|---|
| 壳 | Tauri 2(macOS / Windows) |
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

# 桌面应用(需 Rust 工具链)—— 唯一的运行方式
pnpm tauri dev

# 构建(macOS 产出 .app/.dmg;Windows 产出 .msi/.nsis,需在对应系统上执行)
pnpm tauri build

# 管理平台(账户/同步授权/凭据下发;S3 指向任意兼容存储,如本地 MinIO)
KITE_ADMIN_KEY=<管理密钥> \
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
  ipc/             # IPC 契约 + Tauri 实现(桌面端唯一实现)
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
- **F2 主机管理**:分组(新建/重命名/删除,删除时组内主机迁移不丢)+ 标签、搜索、一键连接、新建/编辑/删除、实时延迟探测(30s TCP 测时)✅
- **F3 文件传输**:SFTP 双栏、上传/下载、队列进度/速率/失败重试、断点续传(续传前比对两端前 1MB SHA-256,不一致自动从头重传)✅
- **F4 密钥管理**:生成 Ed25519 / RSA4096、导入、指纹、复制公钥、使用统计;密码认证兜底;设备密钥存系统 Keychain(旧文件自动迁移)✅(Touch ID/agent:后续)
- **F5 端口转发**:L / R / D 三类实转发 + 流量统计 + 应用启动自动恢复启用规则 ✅
- **F6 命令片段**:片段库 + 终端右栏快捷面板一键运行 + `{{cursor}}` 占位符停留待编辑 ✅
- **F7 云同步**:**管理平台管控** — 注册/登录后平台下发 **STS 限权临时凭据**(session policy 收敛到 `users/{id}/*`,1h 有效;不支持 STS 的存储自动回退共享凭据,`KITE_STS=require` 可强制);分项开关、设备列表;**oplog 增量同步**(逐条 E2E 加密 + AAD 绑定对象路径、每设备游标增量拉取、实体级 LWW + lamport、删除墓碑、开关重开补发存量);**30 天压实**(旧条目并入加密快照后删除,新设备纯快照秒级引导,阈值可调);**manifest 主密钥加密 + If-Match/If-None-Match 乐观锁**(冲突重读重算,不支持条件写的存储自动降级);**BIP39 恢复短语**(24 词 = 主密钥,忘记主密码时凭短语恢复);主密码仅本地派生密钥,平台零知识 ✅
- **F8 会话录制**:asciicast v2 录制(状态栏 REC 计时)+ 内建回放器(播放/暂停/进度/1–4× 倍速)✅
- **F9 命令面板**:⌘K,统一搜索主机/片段/动作 ✅
- **F10 设置**:深/浅主题、终端字号、光标闪烁、行为开关(启动恢复会话 / 断线自动重连 / 退出前确认均已生效)、快捷键表 ✅
- **AI 助手**:右栏对话接入 Claude API(默认 `claude-opus-5`,可换模型;Key 经设备密钥加密存本地);读取当前会话最近输出作为上下文,回复中命令一键「在终端运行」✅

**安全**:known_hosts 指纹校验(TOFU,首连记录;指纹变更即中断连接并告警,需显式「信任新指纹」后重试)。

已知简化(与开发文档 3.x 的差异,均标注于代码注释):
- 监控采样命令面向 Linux 远端(/proc);macOS 远端显示离线
- Keychain 不可用的平台回退应用数据目录 0600 文件
- MinIO 的 STS AssumeRole 需以非 root 服务账号运行平台(root 不可 AssumeRole)

集成自检(`src-tauri/examples/`,均需 `KITE_NO_KEYCHAIN=1` 以跳过系统钥匙串授权框):
`ssh_check`(真实 SSH 栈 + TOFU)、`sync_check`(四台设备同账户,
经管理平台 + 本地 MinIO 十一步全链路:注册登录/STS 凭据下发 → 初始同步 → LWW → 墓碑 → 幂等重放 →
并发冲突收敛 → 平台停用/恢复管控 → STS 限权(跨前缀访问被拒)→ 压实(oplog 清空、仅存加密快照、
manifest 密文)+ 新设备纯快照引导 → BIP39 恢复短语跨设备一致、凭短语解密同步 →
分组增删改 + 跨设备同步)。
