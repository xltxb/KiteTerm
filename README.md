# KiteTerm

自用优先的桌面 SSH 终端:远程连接、SFTP 传输、密钥管理、端口转发、服务器监控与端到端加密云同步。
本仓库为《KiteTerm 产品需求与开发文档》与 UI 原型 `design/prototype.html` 的实现(设计系统:Vela Design System;
原型提供 A 石墨沉浸 / B 命令中心 / C 工作台三个结构方案,已采用 **A**;
应用图标源见 `design/icon.svg`,由 `pnpm tauri icon design/icon.svg` 生成 `desktop/src-tauri/icons/` 全套)。

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
| 样式 | Vela Design System(CSS variables + `data-theme` 切换深/浅);四层级联 token → base → components → utilities,组件里不写内联样式 |
| SSH | russh + russh-sftp(Rust) |
| 存储 | SQLite(rusqlite,配置/片段/密钥/录制索引) |
| 加密 | XChaCha20-Poly1305 + Argon2id(密钥落库、同步 E2E) |
| 同步 | **管理平台管控**:注册/登录后由平台下发 S3 凭据(平台持桶,按用户前缀隔离);桶内仅密文,平台零知识 |
| 管理平台 | `server/`(Rust axum + SQLite):账户注册/登录、注册策略与邀请码、令牌、**功能开关下发**、同步授权、凭据下发 + 内嵌 Web 管理控制台(`/admin`) |

## 开发

客户端在 `desktop/`,管理系统在 `server/`,两者互不依赖、各自独立构建。
根目录只放一层命令转发,下面的 `pnpm` 命令在仓库根和 `desktop/` 里都能跑。

```bash
pnpm install:desktop        # 等价于 pnpm -C desktop install

# 桌面应用(需 Rust 工具链)—— 唯一的运行方式
pnpm tauri dev

# 构建(macOS 产出 .app/.dmg;Windows 产出 .msi/.nsis,需在对应系统上执行)
pnpm tauri build

# 管理平台(账户/同步授权/凭据下发;S3 指向任意兼容存储,如本地 MinIO)
KITE_ADMIN_KEY=<管理密钥> \
S3_ENDPOINT=http://127.0.0.1:19000 S3_BUCKET=vela-sync \
S3_ACCESS_KEY=<ak> S3_SECRET_KEY=<sk> \
cargo run --manifest-path server/Cargo.toml   # 或 pnpm server
# 可选:KITE_FEATURE_GATING=on|off       功能门禁总闸,默认 off(关闭时三项开关不起作用)
#       KITE_REGISTRATION=open|invite|closed 注册策略初值(之后以管理后台设置为准)
#       KITE_REGISTER_LIMIT=5            同一 IP 每小时注册尝试上限
# 应用内「同步」页默认平台地址 http://127.0.0.1:8787,可在“平台设置”修改
# Web 管理控制台:http://127.0.0.1:8787/admin(输入管理密钥)——
#   功能开关(登录 / 云同步 / AI 助手)、注册策略(开放/仅邀请码/关闭)、邀请码生成与停用、新用户默认同步开关、
#   用户列表/搜索、同步启停开关、强制下线(吊销令牌)、按用户查看存储用量、平台统计
# 管理 API(X-Admin-Key):GET /api/admin/stats、GET|POST /api/admin/settings、
#   GET|POST /api/admin/invites、POST /api/admin/invites/{code}/disable、
#   GET /api/admin/users、POST /api/admin/users/{email}/sync {enabled}、
#   POST /api/admin/users/{email}/revoke、GET /api/admin/users/{email}/usage
```

> 开发提示:`pnpm tauri dev` 建议加 `KITE_NO_KEYCHAIN=1`。每次重新编译都会产生新二进制,
> 系统钥匙串会视其为新程序而弹出授权框;该开关让设备密钥走 0600 文件回退,避免反复打断。
>
> ⚠️ 该开关用的是 `device.key` 文件密钥,**与钥匙串里那把不是同一个**。带开关跑过之后再用
> 正常版本启动,同一个库里就会混着两批密钥的密文。应用启动时会自动做**设备密钥迁移**
> (`desktop/src-tauri/src/rewrap.rs`):历史 `device.key` 作为解密兜底,把存量密文逐条改用当前设备
> 密钥重新加密,**全部成功才删除该文件**;有任何一条没搞定就保留文件、下次启动重试。
> 因此不会再出现「解密失败(设备密钥不匹配?)」。

## 目录结构

客户端与管理系统是两个顶层目录,零代码共享 —— 只通过 HTTP 接口往来。

```
package.json         # 只做命令转发(pnpm dev/build/tauri/test → desktop,pnpm server → server)
design/              # 一次性设计产物(不入库):原型 prototype.html、图标 icon.svg、说明 NOTES.md

desktop/                    ── 客户端(macOS / Windows 桌面端)
  src/
    app.css                 样式入口 —— 分层导入:token → base → components → utilities
    app/                    布局:Sidebar、CommandPalette(⌘K)
    screens/                终端 / 主机 / 传输 / 转发 / 监控 / 密钥 / 片段 / 同步 / 设置
      terminal/               终端屏拆分:XtermPane / SearchBar / AiFab / RightDock
      hosts/                  主机屏拆分:HostRow / HostModal / GroupsModal / PasswordModal
      sync/                   同步屏拆分:账户 / 范围 / 设备 / 加密 / 恢复短语等卡片
    components/             Button、Switch、Badge 等基础组件 + AI 面板(components/ai/)
    lib/                    无依赖工具:nlDetect(白话识别)、format(字节/时长格式化)
    stores/                 ui / data / session / transfer / sync / monitor / ai
    ipc/                    IPC 契约 + Tauri 实现(桌面端唯一实现)
    theme/                  设计系统四层:
                              token           colors / typography / spacing / effects / motion / terminal
                              base.css        reset 与元素默认样式
                              components.css  语义组件类(外壳、卡片、按钮、对话…)
                              utilities.css   排布/字号/颜色工具类(排在最后,可覆盖组件类)
                              terminal.ts     读取 --term-* 变量组装 xterm 调色板
  src-tauri/
    src/ssh.rs              russh 会话、PTY、输出事件流
    src/sftp.rs             SFTP 列目录 + 传输队列(进度事件、重试)
    src/forward.rs          L 本地转发 / D 动态 SOCKS5(实时流量统计)
    src/vault.rs            密钥生成(Ed25519/RSA4096)/导入/指纹,私钥经设备密钥加密落库
    src/rewrap.rs           设备密钥迁移:历史 device.key 加密的存量数据换用当前密钥
    src/monitor.rs          经 SSH 采样 CPU/MEM/DISK/负载/网络(无需 agent)
    src/recorder.rs         asciicast v2 会话录制
    src/sync.rs             S3 兼容桶 E2E 加密增量同步
    src/db.rs               SQLite 模型(hosts/keys/snippets/forwards/recordings)
    examples/               集成自检(见文末)

server/                     ── 管理系统(Web)
  Cargo.toml                独立 crate,与客户端零代码共享
  src/main.rs               axum:账户注册/登录、注册策略与邀请码、令牌、同步授权、S3 凭据下发
  src/sts.rs                STS AssumeRole(手写 SigV4),按用户前缀下发限权临时凭据
  src/admin.html            内嵌 Web 管理控制台(include_str!,随二进制分发)
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
- **AI 协助**:**模型可自由选择** —— 配置多个模型并随时切换,按所选模型的协议请求对应接口:
  `anthropic`(`/v1/messages`)、`openai` 兼容(`/v1/chat/completions`,覆盖 OpenAI / DeepSeek /
  OpenRouter / vLLM 等)、`ollama`(`/api/chat`,本地无需密钥);各模型密钥经设备密钥分别加密存本地。
  **终端内协助**:终端右下角常驻 AI 图标(当前行有内容就直接交给 AI 翻译,空行则唤起输入框);
  **也可在命令行直接敲白话回车** —— 识别出不是 shell 命令时自动交给 AI 翻译,
  采纳后替换该行(取消则原文保留,不丢输入);也可用 `⌘⇧A` 主动唤起,或 `⌘I` 让 AI 解释最近输出。
  生成的命令需确认才写入终端,可选「填入」(不回车)或「直接运行」,破坏性命令显著告警;
  识别规则偏保守(首词是已知命令、含 shell 语法一律按命令执行;加 `?` 前缀可强制走 AI),
  可在设置里关闭;
  右栏对话可读取会话输出,回复中的命令一键「在终端运行」✅

- **账户注册**:平台三档注册策略(开放 / 仅邀请码 / 关闭),邀请码支持次数上限、有效期、备注与停用;
  邮箱结构校验 + 口令强度规则(长度、字符种类、常见弱口令黑名单,前后端同规则)、确认密码与实时强度提示;
  按 IP 滑动窗口限流(前置于校验以防邮箱枚举);新用户默认同步开关可配;注册成功即签发令牌免二次登录 ✅

**功能开关**:注册 / 登录 / 云同步 / AI 助手四项由管理平台统一下发,管理控制台可随时开关。

- **总闸(部署配置)**:`KITE_FEATURE_GATING=on|off`,**默认 `off`**。关闭时登录/云同步/AI
  三项开关全部失效 —— 客户端不隐藏任何入口,服务端也不据此拦截,平台只当账户与凭据的中枢。
  做成部署配置而非控制台开关,是因为它决定这套管控要不要存在,属于部署形态,不该被日常运维误点;
  控制台会把总闸状态显示出来并置灰那三个开关,免得以为"点了没反应"。
  注册策略是平台自己的准入政策,**与总闸无关,总闸关掉也照样生效**。
- **下发**:公开接口 `GET /api/features`(免鉴权 —— 否则「登录开关」自身就没法下发)。
  客户端启动时先读本地缓存上屏,再后台拉一次刷新。
- **取值顺序**:能连上平台 → 用刚拉到的并落缓存;连不上但有缓存 → 沿用上次下发的
  (断网既不该把已被管控的机器一键解锁,也不该把没被管控的机器忽然锁上);
  **连不上且从未拉到过 → 按未启用门禁处理,四项全开**。
  最后一条是刻意的:没配过平台的纯本地用户不该被一套他从未启用的管控挡住。
- 「设置 → 管理平台」里可随时改平台地址并立即重新拉取,页面同时显示总闸与各项状态。
- **关闭时的表现**:客户端隐藏入口(侧栏同步项、终端 AI 图标与 FAB、右栏 AI 页签、
  `⌘⇧A` / `⌘I` 快捷键、设置里的相关项),并在够得着的地方说明是平台关的还是尚未连上平台。
- **服务端一并拦截**:登录关闭时 `/api/login` 返回 403 `LOGIN_DISABLED`(校验放在口令校验**之后**,
  避免通过错误信息区分「密码错」与「登录已关」而泄露邮箱是否存在);云同步关闭时
  `/api/sync/credentials` 返回 403 `SYNC_DISABLED`,已持有的令牌同样拿不到凭据。
- **AI 助手只能靠客户端遵守** —— 它完全跑在本地、没有对应的服务端调用。同理,「装了客户端
  但不配平台地址」也绕得过门禁。桌面端二进制在用户手里,这是**产品管控,不是安全边界**;
  真正需要强约束的能力必须放到服务端 —— 登录与云同步没有平台放行就是用不了。
- 粒度为**全局开关**;云同步另有既存的按用户开关,两者取与(全局关则一律关)。

**安全**:known_hosts 指纹校验(TOFU,首连记录;指纹变更即中断连接并告警,需显式「信任新指纹」后重试)。

已知简化(与开发文档 3.x 的差异,均标注于代码注释):
- 监控采样命令面向 Linux 远端(/proc);macOS 远端显示离线
- Keychain 不可用的平台回退应用数据目录 0600 文件
- MinIO 的 STS AssumeRole 需以非 root 服务账号运行平台(root 不可 AssumeRole)

前端用例:`pnpm test`(白话识别规则的正反例 —— 中文/英文意图句判为白话,已知命令、
含管道或选项、路径、环境变量赋值一律判为命令)。

集成自检(`desktop/src-tauri/examples/`)。`ai_check` / `sync_check` 用独立临时目录,加
`KITE_NO_KEYCHAIN=1` 跳过钥匙串授权框;`ssh_check` 读真实应用数据目录,必须与应用同一把
设备密钥,**不能加**该开关(每次重编译会弹一次授权框);`rewrap_check` 自带两种模式,无所谓。

`ssh_check`(真实 SSH 栈 + TOFU;需先跑 `seed`)、
`features_check`(功能开关:首次未知不做门禁、平台不可达且无缓存仍不做门禁、下发后落缓存、
再次不可达时沿用缓存;需先在 8799 起一个测试平台)、
`rewrap_check`(设备密钥迁移:构造钥匙串密钥与历史 `device.key` 并存的库,验证旧密文可经回退解开、
迁移后改用当前密钥、全部成功才删旧文件、重复执行为空操作;不触碰系统钥匙串,可无人值守跑)、
`ai_check`(三协议分发:各配一个模型,校验请求打到正确端点、带对应鉴权头、按各自格式解析响应,
以及命令模式输出收敛与终端上下文注入;需先跑 mock 服务)、
`sync_check`(四台设备同账户,
经管理平台 + 本地 MinIO 十一步全链路:注册登录/STS 凭据下发 → 初始同步 → LWW → 墓碑 → 幂等重放 →
并发冲突收敛 → 平台停用/恢复管控 → STS 限权(跨前缀访问被拒)→ 压实(oplog 清空、仅存加密快照、
manifest 密文)+ 新设备纯快照引导 → BIP39 恢复短语跨设备一致、凭短语解密同步 →
分组增删改 + 跨设备同步)。
