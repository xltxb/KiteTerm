# F11 SSL 证书(ACME)设计

日期:2026-09-07
状态:已确认
来源:Vela Term 产品与开发文档 v1.1(Claude Design d49d6720)F11 / 2.3 / 2.5 / 3.2 / 3.3;原型「证书」屏

## 目标

在 KiteTerm 桌面端实现一键 ACME 证书签发:域名 + 部署主机 + HTTP-01/DNS-01,经目标主机的活跃 SSH 会话完成校验、签发、上传证书链并 reload 服务;90 天有效期进度、到期前 30 天自动续期;证书随配置加密同步。

## 非目标

- DNS-01 不接任何 DNS 提供商 API,采用手动模式(展示 TXT 记录,用户添加后点验证)。
- 不做证书的多 CA 账户管理 UI(账户密钥自动生成并随配置同步)。
- 移动端、团队共享不在范围。

## 门禁

- 证书功能不受 F12 的 AI/同步开关约束(始终可见)。
- 签发与部署要求目标主机有活跃 SSH 会话;无活跃会话时 `cert_issue` 返回 `AppError("ERR_NO_SESSION")`,前端表单降级为「去连接主机」空态。

## 后端(`desktop/src-tauri`)

### 依赖
`Cargo.toml` 加 `instant-acme`(HTTP-01 + DNS-01 均支持)、`rcgen`(生成 CSR 与私钥)、`x509-parser`(解析签发结果的 not_after/域名)。

### 数据库 `certs` 表(`db.rs`)
`id, domains(JSON array), challenge(http|dns), ca(letsencrypt|zerossl), host_id, deploy_path, reload_cmd, not_after(i64 ms), auto_renew(bool), state(active|expiring|error), account_key_enc(BLOB), cert_pem, chain_pem, created_at`。私钥与 ACME 账户密钥经 vault 加密后入库(与 keys 表同一 vault)。

### 新模块 `acme.rs`
- `CertManager`:`Mutex<HashMap<String, JoinHandle>>` 跟踪进行中的签发/续期;续期守护 `spawn_renew_loop`(仿 `monitor.rs`/`spawn_latency_loop`)每 12h 扫描 `certs`,对 `auto_renew` 且 `not_after` 距今 ≤30 天者重签,失败置 `state=error` 并 emit。
- 签发状态机 `issue(app, db, vault, sessions, req)`:
  1. **放置校验**:HTTP-01 经 `sessions.get_handle(host_id)` 拿活跃会话,`ssh::exec_capture` 在远端 `mkdir -p <webroot>/.well-known/acme-challenge` 并写入 token 文件(webroot 默认从 reload/deploy 上下文推断,表单可填);DNS-01 手动模式 emit 需要添加的 TXT 记录,等前端 `cert_dns_confirm` 再继续。
  2. **验证**:通知 ACME 就绪,轮询 order 状态。
  3. **签发**:`rcgen` 生成密钥+CSR,finalize,拉取证书链,`x509-parser` 解析 not_after。
  4. **上传**:经 SFTP 把 `fullchain.pem`/`privkey.pem` 写到 `deploy_path`。
  5. **reload**:`ssh::exec_capture` 执行 `reload_cmd`(默认 `sudo nginx -s reload`)。
  每步 emit `cert://progress/{id}` `{ step: 0..4, state: running|ok|error, label, error? }`。落库 `state=active`。
- `renew(id)`:等价于用已存参数重跑签发。
- `revoke(id)`:调 ACME revoke(可得则),删库行。
- `deploy(id, host_id)`:对已签发证书,复用/新建到 host_id 的会话,SFTP 上传 + reload。
- 纯函数:`renew_due(not_after_ms, now_ms) -> bool`(≤30 天)、`days_left(not_after_ms, now_ms) -> i64`、`challenge_path(token) -> String`、五步 `Step` 枚举与推进——这些写单元测试。

### IPC 命令(`commands.rs` + `lib.rs` 注册)
`cert_list() -> Vec<CertInfo>`、`cert_issue(req) -> CertInfo`、`cert_dns_confirm(id)`、`cert_renew(id)`、`cert_revoke(id)`、`cert_deploy(id, host_id)`、`cert_download_chain(id) -> String`(返回链 PEM 供前端保存)。`AppState` 加 `certs: Arc<CertManager>`。

## 前端(`desktop/src`)

### 类型与路由
`types.ts` 加 `CertInfo`、`IssueRequest`;`Screen` 加 `"certs"`;`App.tsx` 加 `{screen === "certs" && <CertsScreen />}`;`Sidebar.tsx` NAV 在「转发」后加 `{ screen: "certs", label: "证书", icon: <KtCerts /> }`;`icons.tsx` 加 `KtCerts`(盾牌/锁,lucide `shield-check` 风格,与现有自绘图标同规格)。

### ipc
`api.ts` + `tauri.ts` 加上述 7 个命令与事件订阅 `onCertProgress(cb)`(`cert://progress/{id}`)。

### store
新增 `stores/certs.ts`:`certs` 列表、`issuing` 状态、五步 `issueSteps`、`dnsRecord`(手动模式待添加的 TXT)、`issue()/renew()/revoke()/deploy()/confirmDns()`;订阅进度事件增量更新。

### CertsScreen(严格照原型)
- 头部:eyebrow「SSL CERTIFICATES」+「SSL 证书 {N} 有效」。
- 签发卡:标题「一键申请 · ACME (Let's Encrypt / ZeroSSL)」+ 已连接徽章显示主机名;已连接时显示 域名输入、部署主机(取当前活跃会话主机)、HTTP-01/DNS-01 分段、「申请并部署」按钮、帮助文字;未连接时 `opacity` 降低并显示 plug-zap 空态 +「去连接主机」跳转 hosts。
- 签发中:五步流水线卡(图标 + mono 文案 + 每步颜色随状态)。
- DNS-01 手动:签发中若为 DNS-01,展示 TXT 记录(名称/值,可复制)与「我已添加,验证」按钮 → `confirmDns`。
- 证书列表卡:每行 图标 + 域名 + 状态徽章 + meta + 有效期进度条({daysLeft}/90 天,颜色随剩余天数)+ 主机 + 「立即续期」+ ellipsis 菜单(详情 / 复制证书路径 / 下载证书链 / 部署到其他主机… / 重试验证 / 暂停续期 / 吊销删除;含分隔线)。
- 脚注:自动续期说明。操作成功走 toast(复用现有 toast 或原型的圆角浮条)。
- 组件复用仓库既有 `Button`/`Badge`/`TextInput`/`Segmented`/`Progress`/`ScreenHeader`;不写内联硬编码色值,用 Vela token 变量(与现有屏一致)。

## 测试

- Rust:`acme.rs` 纯函数(renew_due、days_left、challenge_path、五步推进、certs serde、CertInfo 映射)单元测试;`cargo test`、`cargo build`。
- 前端:store 的进度合并与状态派生若有纯逻辑则单测;`npm run build`、`npm test` 通过。
- 手动:签发流程依赖真实域名 + Let's Encrypt + 真实主机,由用户在真实环境端到端验证;UI 空态/门禁/列表渲染可在运行中的客户端观察。

## 里程碑对应
文档 M4:F11 证书(ACME 签发 + 自动续期)+ F12 管理平台。F12 已实现,本次补齐 F11。
