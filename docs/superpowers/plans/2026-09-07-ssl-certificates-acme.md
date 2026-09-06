# F11 SSL 证书(ACME)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** KiteTerm 桌面端实现一键 ACME 证书签发/续期/部署:域名 + 部署主机 + HTTP-01(经活跃 SSH 会话)/DNS-01(手动),90 天有效期与到期前 30 天自动续期,`/certs` 屏严格照原型。

**Architecture:** 后端新增 `acme.rs`(基于 `instant-acme`/`rcgen`/`x509-parser`),仿 `forward.rs`/`monitor.rs` 用一个 `CertManager` + 续期守护;`certs` 表入库,私钥与 ACME 账户密钥经现有 `Vault` 加密。7 个 `cert_*` IPC 命令,进度事件 `cert://progress/{id}`。前端新增 `/certs` 屏、侧栏入口、`certs` store,UI 复用既有 Vela 组件与 token。

**Tech Stack:** Rust(rustc 1.98, edition 2021)+ instant-acme 0.8 + rcgen 0.14 + x509-parser 0.18 / Tauri 2 / russh / React 18 + Zustand / xterm.js。

**Spec:** `docs/superpowers/specs/2026-09-07-ssl-certificates-acme.md`

## Global Constraints

- 面向用户文案中文;注释中文;标识符英文;不写硬编码色值,用 Vela token 变量;组件复用 `@/components/ui`。
- 证书功能不受 F12 的 AI/同步开关约束(始终可见)。
- 签发/部署要求目标主机有活跃 SSH 会话;无会话时 `cert_issue`/`cert_deploy` 返回 `AppError("ERR_NO_SESSION".into())`,前端表单降级为「去连接主机」空态。
- DNS-01 手动模式:签发时 emit 待添加的 TXT 记录,等前端 `cert_dns_confirm(id)` 再继续;不接任何 DNS 提供商 API。
- 证书私钥、ACME 账户密钥经 `Vault::encrypt` 后入库(`decrypt_current` 读回)。
- 进度事件 `cert://progress/{id}` 负载:`{ certId, step: 0..4, state: "running"|"ok"|"error", label, error? }`;五步 = 放置校验 / 验证 / 签发 / 上传 / reload。
- 真实签发依赖真实域名 + Let's Encrypt + 真实主机,只能由用户端到端验证;单元测试覆盖纯函数与 db,构建须通过。
- 提交信息结尾附带:
  ```
  Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_0148LdwcwZw8wjtea3koVBC6
  ```
- 工作目录 `desktop/`;cargo 在 `desktop/src-tauri/` 下跑。一个 `pnpm tauri dev` 实例在运行并热更新此树,属正常,勿杀;cargo 报锁时等待重试。分支由控制器指定,勿切换。

## 关键既有 API(实现者复用,勿改签名)

- `state.sessions.get_handle(host_id: &str) -> Result<Arc<Handle<ClientHandler>>>`(会话不存在/本地终端返回 Err;用它做活跃会话门禁)。
- `ssh::exec_capture(handle: &Handle<ClientHandler>, cmd: &str) -> Result<String>`(在远端执行命令并收集输出)。
- SFTP 写文件参照 `sftp.rs`:`open_sftp(...)`→`sftp.create(path).await` / `open_with_flags(path, OpenFlags::WRITE)`,再 `write_all`。部署上传复用此法(实现者读 `sftp.rs` 现有上传路径)。
- `Vault::encrypt(&[u8]) -> Result<Vec<u8>>` / `decrypt_current(&[u8]) -> Result<Vec<u8>>`。
- `Db::kv_get/kv_set`;新表 CRUD 仿 `db.rs` 现有 `list_forwards` 等写法。
- 事件发送仿 `forward.rs`:`app.emit("事件名", payload)`。
- 前端 store 仿 `stores/data.ts`;屏组件仿 `screens/ForwardsScreen.tsx`;图标仿 `components/icons.tsx` 的 `Icon` 包装。

---

### Task 1: certs 表、模型与纯函数

**Files:**
- Modify: `desktop/src-tauri/src/models.rs`(新增 `CertInfo`、`IssueRequest`)
- Modify: `desktop/src-tauri/src/db.rs`(建 `certs` 表;CRUD)
- Create: `desktop/src-tauri/src/acme.rs`(仅本任务的纯函数 + 单测;`CertManager` 骨架)
- Modify: `desktop/src-tauri/src/lib.rs`(`mod acme;` 按字母序在 `mod account;` 后)

**Interfaces:**
- Produces:
  - `models::CertInfo`(serde camelCase):`id: String, domains: Vec<String>, challenge: String, ca: String, host_id: String, deploy_path: String, reload_cmd: String, not_after: i64, auto_renew: bool, state: String, created_at: String`(不含 pem/私钥等敏感字段——那些只在后端)。
  - `models::IssueRequest`(serde camelCase):`domains: Vec<String>, challenge: String, ca: String, host_id: String, deploy_path: String, reload_cmd: String`。
  - `acme::renew_due(not_after_ms: i64, now_ms: i64) -> bool`(距今 ≤30 天为 true)。
  - `acme::days_left(not_after_ms: i64, now_ms: i64) -> i64`。
  - `acme::challenge_path(token: &str) -> String`(返回 `/.well-known/acme-challenge/<token>`)。
  - `acme::Step`(枚举 `Place, Verify, Sign, Upload, Reload`,`as_index() -> u8` 0..4,`label(&self) -> &str`)。
  - `db` 方法:`list_certs() -> Result<Vec<CertInfo>>`、`get_cert_full(id) -> Result<CertRow>`、`upsert_cert(row: &CertRow) -> Result<()>`、`delete_cert(id) -> Result<()>`,其中 `CertRow` 是 `db.rs` 内含 pem/enc 字段的完整行结构。

- [ ] **Step 1: 写纯函数测试**

`acme.rs`:

```rust
//! ACME 证书:签发/续期/部署状态机与续期守护。纯函数在文件末尾单测。

pub fn renew_due(not_after_ms: i64, now_ms: i64) -> bool {
    // 距到期 ≤ 30 天(含已过期)即应续期
    not_after_ms - now_ms <= 30 * 24 * 60 * 60 * 1000
}

pub fn days_left(not_after_ms: i64, now_ms: i64) -> i64 {
    (not_after_ms - now_ms) / (24 * 60 * 60 * 1000)
}

pub fn challenge_path(token: &str) -> String {
    format!("/.well-known/acme-challenge/{token}")
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Place,
    Verify,
    Sign,
    Upload,
    Reload,
}

impl Step {
    pub fn as_index(self) -> u8 {
        match self {
            Step::Place => 0,
            Step::Verify => 1,
            Step::Sign => 2,
            Step::Upload => 3,
            Step::Reload => 4,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Step::Place => "放置校验文件",
            Step::Verify => "等待 CA 验证",
            Step::Sign => "签发证书",
            Step::Upload => "上传证书链",
            Step::Reload => "重载服务",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renew_due_within_30_days() {
        let now = 1_000_000_000_000;
        assert!(renew_due(now + 29 * 86_400_000, now));
        assert!(renew_due(now - 86_400_000, now)); // 已过期
        assert!(!renew_due(now + 31 * 86_400_000, now));
    }

    #[test]
    fn days_left_floor() {
        let now = 1_000_000_000_000;
        assert_eq!(days_left(now + 90 * 86_400_000, now), 90);
        assert_eq!(days_left(now, now), 0);
    }

    #[test]
    fn challenge_path_format() {
        assert_eq!(challenge_path("abc"), "/.well-known/acme-challenge/abc");
    }

    #[test]
    fn step_index_and_label() {
        assert_eq!(Step::Reload.as_index(), 4);
        assert_eq!(Step::Place.label(), "放置校验文件");
    }
}
```

`lib.rs` 加 `mod acme;`(字母序,在 `mod account;` 之后)。

- [ ] **Step 2: 运行确认失败**

Run: `cd desktop/src-tauri && cargo test acme::`
Expected: 编译错误(模块或引用未就绪)

- [ ] **Step 3: 加模型与 db CRUD**

`models.rs` 追加(放在 `ForwardRule` 附近):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertInfo {
    pub id: String,
    pub domains: Vec<String>,
    pub challenge: String,
    pub ca: String,
    pub host_id: String,
    pub deploy_path: String,
    pub reload_cmd: String,
    pub not_after: i64,
    pub auto_renew: bool,
    pub state: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueRequest {
    pub domains: Vec<String>,
    pub challenge: String,
    pub ca: String,
    pub host_id: String,
    pub deploy_path: String,
    pub reload_cmd: String,
}
```

`db.rs` 建表(在 `execute_batch` 的 SQL 末尾、`sync_versions` 之后追加):

```sql
CREATE TABLE IF NOT EXISTS certs (
    id TEXT PRIMARY KEY, domains TEXT NOT NULL DEFAULT '[]',
    challenge TEXT NOT NULL, ca TEXT NOT NULL, host_id TEXT NOT NULL,
    deploy_path TEXT NOT NULL DEFAULT '', reload_cmd TEXT NOT NULL DEFAULT '',
    not_after INTEGER NOT NULL DEFAULT 0, auto_renew INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL DEFAULT 'active',
    account_key_enc BLOB, cert_pem TEXT, chain_pem TEXT,
    created_at TEXT NOT NULL
);
```

在 `db.rs` 加一个 `pub struct CertRow`(含全部列,`account_key_enc: Option<Vec<u8>>`、`cert_pem/chain_pem: Option<String>`)与方法 `list_certs`(映射到 `CertInfo`,不含敏感列)、`get_cert_full`、`upsert_cert`(`INSERT ... ON CONFLICT(id) DO UPDATE`)、`delete_cert`。domains 以 JSON 文本存取(仿 hosts.tags 的做法,`serde_json`)。

- [ ] **Step 4: 运行测试与构建**

Run: `cd desktop/src-tauri && cargo test acme:: && cargo build`
Expected: 4 个纯函数测试 PASS;构建成功(`CertManager` 尚未使用可加 `#[allow(dead_code)]`)。

- [ ] **Step 5: CertManager 骨架**

`acme.rs` 加(纯函数之前):

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::task::JoinHandle;

/// 跟踪进行中的签发/续期任务;续期守护单独 spawn
#[derive(Default)]
pub struct CertManager {
    pub jobs: Mutex<HashMap<String, JoinHandle<()>>>,
}
```

- [ ] **Step 6: 构建**

Run: `cd desktop/src-tauri && cargo build`
Expected: 成功

- [ ] **Step 7: 提交**

```bash
git add desktop/src-tauri/src/models.rs desktop/src-tauri/src/db.rs desktop/src-tauri/src/acme.rs desktop/src-tauri/src/lib.rs
git commit -m "feat(certs): certs 表、CertInfo/IssueRequest 模型与 ACME 纯函数"
```

---

### Task 2: ACME 签发状态机(HTTP-01 + DNS-01 手动)

**Files:**
- Modify: `desktop/src-tauri/Cargo.toml`(依赖)
- Modify: `desktop/src-tauri/src/acme.rs`(签发实现)

**Interfaces:**
- Consumes: Task 1 的 `CertRow`、`CertInfo`、`IssueRequest`、`Step`、`challenge_path`;`ssh::exec_capture`、`SessionManager::get_handle`、`Vault`、`Db`、SFTP 上传(读 `sftp.rs`)。
- Produces:
  - `pub async fn issue(app: AppHandle, db: Arc<Db>, vault: Arc<Vault>, sessions: Arc<SessionManager>, manager: Arc<CertManager>, req: IssueRequest) -> Result<CertInfo>`(前置门禁:`sessions.get_handle(&req.host_id)?`,失败即 `AppError("ERR_NO_SESSION".into())`)。
  - `pub fn confirm_dns(manager: &CertManager, cert_id: &str)`(唤醒等待中的 DNS-01 签发——用一个 `tokio::sync::Notify` 或 oneshot,存于 manager 的一个 `Mutex<HashMap<String, Arc<Notify>>>`)。
  - `emit_progress(app, cert_id, step: Step, state: &str, error: Option<&str>)` 私有辅助,发 `cert://progress/<id>`。

- [ ] **Step 1: 加依赖**

`Cargo.toml` `[dependencies]` 追加:

```toml
# ACME 证书签发
instant-acme = "0.8"
rcgen = "0.14"
x509-parser = "0.18"
```

- [ ] **Step 2: 构建确认依赖可解析**

Run: `cd desktop/src-tauri && cargo build`
Expected: 依赖下载编译成功(可能耗时)。

- [ ] **Step 3: 实现签发状态机**

在 `acme.rs` 实现 `issue`。参照 `instant-acme` 0.8 的实际 API(实现者用 `cargo doc -p instant-acme --open` 或读 `~/.cargo/registry/src/*/instant-acme-0.8*/src/` 确认类型名),流程:
1. 生成 cert_id(uuid),`emit_progress(Place, "running")`。
2. `Account::create`(CA directory:`ca=="zerossl"` 用 ZeroSSL,否则 Let's Encrypt production directory URL),账户密钥 PEM 经 `vault.encrypt` 备用。
3. 新建 `Order`(domains → identifiers)。
4. 取 authorizations:
   - HTTP-01:取 challenge token 与 key authorization,`sessions.get_handle(&req.host_id)?` 拿会话,`exec_capture` 执行 `mkdir -p <webroot>/.well-known/acme-challenge && printf %s '<keyauth>' > <webroot><challenge_path>`;webroot 从 `req.deploy_path` 的父目录推断或默认 `/var/www/html`(实现者定一个合理默认并注释)。
   - DNS-01:计算 `_acme-challenge.<domain>` 的 TXT 值,`emit` 一个 `cert://dns-record/<id>` 事件 `{ name, value }`,然后在 `Notify` 上等待 `confirm_dns`(带 5 分钟超时,超时 `AppError`)。
5. `emit_progress(Verify, "running")`;`order.set_challenge_ready`,轮询 order/authorization 状态至 valid(带超时与间隔)。
6. `emit_progress(Sign, "running")`:`rcgen` 生成密钥与 CSR,`order.finalize`,拉取证书链 PEM;`x509-parser` 解析叶证书 `not_after`(→ ms)。
7. `emit_progress(Upload, "running")`:SFTP 上传 `chain_pem` 到 `req.deploy_path`,私钥 PEM 到同目录 `privkey.pem`(实现者按 sftp.rs 的写法)。
8. `emit_progress(Reload, "running")`:`exec_capture(req.reload_cmd)`(为空则默认 `sudo nginx -s reload`);每步成功 emit `"ok"`,失败 emit `"error"` 并 `upsert_cert(state="error")` 后返回 Err。
9. 全部成功:`upsert_cert`(state="active", not_after, cert_pem, chain_pem, account_key_enc),返回 `CertInfo`。

`CertManager` 加 `Mutex<HashMap<String, Arc<tokio::sync::Notify>>>` 字段支撑 `confirm_dns`。

- [ ] **Step 4: 构建**

Run: `cd desktop/src-tauri && cargo build`
Expected: 成功。此任务无自动化端到端测试(需真实 CA);实现者在报告说明,纯函数测试已在 Task 1。

- [ ] **Step 5: 提交**

```bash
git add desktop/src-tauri/Cargo.toml desktop/src-tauri/Cargo.lock desktop/src-tauri/src/acme.rs
git commit -m "feat(certs): ACME 签发状态机(HTTP-01 经 SSH、DNS-01 手动、五步进度)"
```

---

### Task 3: 续期、吊销、部署与续期守护

**Files:**
- Modify: `desktop/src-tauri/src/acme.rs`

**Interfaces:**
- Consumes: Task 2 的 `issue`;`renew_due`。
- Produces:
  - `pub async fn renew(app, db, vault, sessions, manager, cert_id: &str) -> Result<CertInfo>`(用库中已存 `CertRow` 的参数重跑 `issue`;要求该 host 有活跃会话)。
  - `pub async fn revoke(db, vault, cert_id: &str) -> Result<()>`(尽力调 ACME revoke,失败忽略;`db.delete_cert`)。
  - `pub async fn deploy(app, db, sessions, cert_id: &str, host_id: &str) -> Result<()>`(取已存 chain/key,门禁 `get_handle(host_id)?`,SFTP 上传 + reload)。
  - `pub fn spawn_renew_loop(app: AppHandle, db: Arc<Db>, vault: Arc<Vault>, sessions: Arc<SessionManager>, manager: Arc<CertManager>)`(仿 `ssh::spawn_latency_loop`:每 12h 扫 `list_certs`/`get_cert_full`,对 `auto_renew && renew_due` 者尝试 `renew`;成功不额外操作,失败 `upsert_cert(state="error"/"expiring")` 并 emit `cert://progress`)。

- [ ] **Step 1: 实现四个函数**

按上述接口实现。`renew` 复用 `issue` 的主体(可把 `issue` 内核抽成 `run_issue(...)` 供两者调用,避免重复)。守护循环用 `tokio::time::interval(Duration::from_secs(12*3600))`,循环体对每张到期证书调 `renew` 并吞错记录。

- [ ] **Step 2: 构建**

Run: `cd desktop/src-tauri && cargo build`
Expected: 成功。

- [ ] **Step 3: 提交**

```bash
git add desktop/src-tauri/src/acme.rs
git commit -m "feat(certs): 续期/吊销/部署与到期前 30 天自动续期守护"
```

---

### Task 4: IPC 命令与 AppState 接线

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`(7 个命令 + `AppState.certs`)
- Modify: `desktop/src-tauri/src/lib.rs`(构造 `CertManager`、spawn 守护、注册命令、`app.manage`)

**Interfaces:**
- Consumes: Task 1–3 的 `acme::*`、`CertManager`。
- Produces:Tauri 命令 `cert_list`、`cert_issue`、`cert_dns_confirm`、`cert_renew`、`cert_revoke`、`cert_deploy`、`cert_download_chain`。

- [ ] **Step 1: AppState 加字段并接线**

`commands.rs` `AppState` 加 `pub certs: Arc<crate::acme::CertManager>,`。`lib.rs`:`let certs = Arc::new(acme::CertManager::default());`;在既有 `tauri::async_runtime::spawn` 块内加 `acme::spawn_renew_loop(app_handle.clone(), db.clone(), vault.clone(), sessions.clone(), certs.clone());`(把 `certs` 一并克隆进闭包);`app.manage(AppState { ..., certs })`。

- [ ] **Step 2: 命令**

`commands.rs` 追加(仿现有 async 命令签名):

```rust
// ---- certs ----
#[tauri::command]
pub fn cert_list(state: State<AppState>) -> Result<Vec<CertInfo>> {
    state.db.list_certs()
}

#[tauri::command]
pub async fn cert_issue(app: AppHandle, state: State<'_, AppState>, req: IssueRequest) -> Result<CertInfo> {
    crate::acme::issue(app, state.db.clone(), state.vault.clone(), state.sessions.clone(), state.certs.clone(), req).await
}

#[tauri::command]
pub fn cert_dns_confirm(state: State<AppState>, id: String) -> Result<()> {
    crate::acme::confirm_dns(&state.certs, &id);
    Ok(())
}

#[tauri::command]
pub async fn cert_renew(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<CertInfo> {
    crate::acme::renew(app, state.db.clone(), state.vault.clone(), state.sessions.clone(), state.certs.clone(), &id).await
}

#[tauri::command]
pub async fn cert_revoke(state: State<'_, AppState>, id: String) -> Result<()> {
    crate::acme::revoke(state.db.clone(), state.vault.clone(), &id).await
}

#[tauri::command]
pub async fn cert_deploy(app: AppHandle, state: State<'_, AppState>, id: String, host_id: String) -> Result<()> {
    crate::acme::deploy(app, state.db.clone(), state.sessions.clone(), &id, &host_id).await
}

#[tauri::command]
pub fn cert_download_chain(state: State<AppState>, id: String) -> Result<String> {
    let row = state.db.get_cert_full(&id)?;
    row.chain_pem.ok_or_else(|| AppError("证书链不存在".into()))
}
```

`lib.rs` handler 列表加这 7 个命令(在 `ai_*` 命令附近或末尾)。

- [ ] **Step 3: 构建**

Run: `cd desktop/src-tauri && cargo build && cargo test`
Expected: 构建成功;既有测试(含 Task 1 的 acme 纯函数)全绿。

- [ ] **Step 4: 提交**

```bash
git add desktop/src-tauri/src/commands.rs desktop/src-tauri/src/lib.rs
git commit -m "feat(certs): cert_* IPC 命令、AppState 接线与续期守护启动"
```

---

### Task 5: 前端类型、ipc、路由与侧栏入口

**Files:**
- Modify: `desktop/src/types.ts`(`CertInfo`、`IssueRequest`、`Screen` 加 `"certs"`)
- Modify: `desktop/src/ipc/api.ts` + `desktop/src/ipc/tauri.ts`(7 命令 + 进度/DNS 事件订阅)
- Modify: `desktop/src/app/App.tsx`(路由)
- Modify: `desktop/src/app/Sidebar.tsx`(NAV 项)
- Modify: `desktop/src/components/icons.tsx`(`KtCerts`)

**Interfaces:**
- Consumes: Task 4 的命令与事件。
- Produces:`ipc.certList/certIssue/certDnsConfirm/certRenew/certRevoke/certDeploy/certDownloadChain`;`ipc.onCertProgress(cb)`、`ipc.onCertDnsRecord(cb)`;`CertInfo`、`IssueRequest` TS 类型;`Screen` 含 `"certs"`;`KtCerts` 图标。

- [ ] **Step 1: 类型**

`types.ts` `Screen` 联合加 `| "certs"`(放在 `"forwards"` 后)。追加:

```ts
export interface CertInfo {
  id: string;
  domains: string[];
  challenge: "http" | "dns";
  ca: "letsencrypt" | "zerossl";
  hostId: string;
  deployPath: string;
  reloadCmd: string;
  notAfter: number;
  autoRenew: boolean;
  state: "active" | "expiring" | "error";
  createdAt: string;
}

export interface IssueRequest {
  domains: string[];
  challenge: "http" | "dns";
  ca: "letsencrypt" | "zerossl";
  hostId: string;
  deployPath: string;
  reloadCmd: string;
}

export interface CertProgress {
  certId: string;
  step: number;
  state: "running" | "ok" | "error";
  label: string;
  error?: string;
}

export interface CertDnsRecord {
  certId: string;
  name: string;
  value: string;
}
```

- [ ] **Step 2: ipc**

`api.ts` 加 7 个命令声明 + `onCertProgress(cb: (p: CertProgress) => void): Unsubscribe`、`onCertDnsRecord(cb: (r: CertDnsRecord) => void): Unsubscribe`。`tauri.ts` 加实现:

```ts
    certList: () => invoke("cert_list"),
    certIssue: (req) => invoke("cert_issue", { req }),
    certDnsConfirm: (id) => invoke("cert_dns_confirm", { id }),
    certRenew: (id) => invoke("cert_renew", { id }),
    certRevoke: (id) => invoke("cert_revoke", { id }),
    certDeploy: (id, hostId) => invoke("cert_deploy", { id, hostId }),
    certDownloadChain: (id) => invoke("cert_download_chain", { id }),
    onCertProgress: (cb) => sub<CertProgress>("cert://progress", (p) => cb(p)),
    onCertDnsRecord: (cb) => sub<CertDnsRecord>("cert://dns-record", (r) => cb(r)),
```

(注:后端 emit 的事件名用固定 `cert://progress` / `cert://dns-record`,负载里带 `certId` 区分——与 Task 2/后端保持一致;若后端用了 `/{id}` 后缀,改这里的订阅为按需。控制器已在 Global Constraints 固定为不带后缀 + 负载含 certId。)

- [ ] **Step 3: 路由与侧栏**

`App.tsx` 在 `{screen === "forwards" && ...}` 后加 `{screen === "certs" && <CertsScreen />}`,并 `import { CertsScreen } from "@/screens/CertsScreen";`。`Sidebar.tsx` NAV 在 `forwards` 项后加 `{ screen: "certs", label: "证书", icon: <KtCerts size={18} /> }`,并从 icons 导入 `KtCerts`。

- [ ] **Step 4: 图标**

`icons.tsx` 加(盾牌 + 勾,复用 `Icon` 包装):

```tsx
/** 证书 / SSL 盾牌 */
export function KtCerts(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M12 3 l7 3 v5 c0 4.2 -2.8 7.3 -7 8.5 c-4.2 -1.2 -7 -4.3 -7 -8.5 V6 z" />
      <path d="M9 12 l2 2 l4 -4" />
    </Icon>
  );
}
```

- [ ] **Step 5: 类型检查(先建占位屏免得 tsc 报缺失)**

创建 `desktop/src/screens/CertsScreen.tsx` 占位:`export function CertsScreen() { return <div className="screen vscroll" />; }`(Task 7 填充)。

Run: `cd desktop && npx tsc -b`
Expected: 无错误。

- [ ] **Step 6: 提交**

```bash
git add desktop/src/types.ts desktop/src/ipc/api.ts desktop/src/ipc/tauri.ts desktop/src/app/App.tsx desktop/src/app/Sidebar.tsx desktop/src/components/icons.tsx desktop/src/screens/CertsScreen.tsx
git commit -m "feat(certs): 前端类型、ipc、/certs 路由、侧栏入口与图标"
```

---

### Task 6: certs store

**Files:**
- Create: `desktop/src/stores/certs.ts`

**Interfaces:**
- Consumes: Task 5 的 ipc。
- Produces:`useCertsStore` — `certs: CertInfo[]`、`issuing: boolean`、`issueSteps: CertProgress[]`(五步当前态)、`dnsRecord: CertDnsRecord | null`、`load()`、`issue(req)`、`renew(id)`、`revoke(id)`、`deploy(id, hostId)`、`confirmDns(id)`、`downloadChain(id)`。

- [ ] **Step 1: 实现 store**

仿 `stores/data.ts`。`wire()`(一次性)订阅 `onCertProgress` 增量更新 `issueSteps`(按 step 索引替换,`state==="ok"` 全五步或某步 error 时结束 `issuing`)与 `onCertDnsRecord` 设 `dnsRecord`。`issue` 置 `issuing=true`、清空 `issueSteps/dnsRecord`,`await ipc.certIssue(req)`,完成后 `load()` 刷新列表并 `issuing=false`(错误也复位)。`load` = `set({ certs: await ipc.certList() })`。`confirmDns` = `ipc.certDnsConfirm(id)`。`downloadChain` 取回 PEM 交由屏层用 `pickDirectory`/写文件保存(实现者用现有 dialog 能力;若无保存能力则复制到剪贴板并提示)。

- [ ] **Step 2: 类型检查**

Run: `cd desktop && npx tsc -b`
Expected: 无错误。

- [ ] **Step 3: 提交**

```bash
git add desktop/src/stores/certs.ts
git commit -m "feat(certs): certs store(列表、签发进度、DNS 手动确认)"
```

---

### Task 7: CertsScreen UI(照原型)

**Files:**
- Modify: `desktop/src/screens/CertsScreen.tsx`(填充)
- Modify: `desktop/src/theme/components.css`(如需 certs 专属类)

**Interfaces:**
- Consumes: Task 6 的 `useCertsStore`;`useSessionStore`(取活跃会话/主机)、`useDataStore`(主机名);`useUiStore`(`go`)。

- [ ] **Step 1: 实现屏(照原型 `certs-prototype.html`)**

参照原型片段(见 spec「原型」与下述结构),用仓库 `Button`/`Badge`/`TextInput`/`Segmented`/`Progress`/`ScreenHeader`:
- 头部 `ScreenHeader` eyebrow="SSL CERTIFICATES" title="SSL 证书" count=`${activeCount} 有效`。
- 签发卡:标题「一键申请 · ACME (Let's Encrypt / ZeroSSL)」;活跃会话存在时(从 `useSessionStore` 取第一个 `connected` 的 SSH 会话,`kind==="ssh"`)显示 域名 `TextInput`、部署主机(显示活跃会话主机名,只读)、HTTP-01/DNS-01 `Segmented`、「申请并部署」`Button`(loading=`issuing`,点击 `issue({domains: 域名按逗号拆, challenge, ca:"letsencrypt", hostId, deployPath 默认 "/etc/nginx/ssl/fullchain.pem", reloadCmd 默认 "sudo nginx -s reload"})`),帮助文字;无活跃会话时卡片 `opacity:.6` 并渲染 plug-zap 空态 +「去连接主机」`Button`(`go("hosts")`)。
- 签发中:五步流水线卡,遍历 `issueSteps`,每步图标随 `state`(running/ok/error)着色。
- DNS-01 手动:`dnsRecord` 非空时显示 TXT 记录名/值(可复制)+「我已添加,验证」`Button`(`confirmDns(dnsRecord.certId)`)。
- 证书列表卡:遍历 `certs`,每行 图标 + 域名(mono)+ 状态 `Badge`(active=success/expiring=warning/error=danger)+ meta(CA、challenge)+ 有效期进度条(`Progress value={pct}`,`pct` 由 `daysLeft/90`,颜色随剩余)+ 主机名 + 「立即续期」`Button`(`renew(id)`)+ ellipsis 菜单。
- ellipsis 菜单项:查看证书详情 / 复制证书路径(`deployPath` 写剪贴板)/ 下载证书链(`downloadChain`)/ 部署到其他主机…(选主机后 `deploy`)/ 重试验证(`renew`)/ 暂停续期(切 autoRenew——本版可只前端提示或调 renew 复用)/ 吊销删除(`confirmAction` 后 `revoke`)。分隔线按原型。
- 脚注:「到期前 30 天自动续期并重载服务;账户密钥与证书随主机配置端到端加密同步。」
- 操作成功用现有提示(`showError` 仅错误;成功可用一个短 toast div,复用原型圆角浮条样式,加到 `components.css`)。

进度条剩余天数颜色:>30 天 accent,7–30 天 warning,<7 天 danger(用 token 变量)。

- [ ] **Step 2: 构建**

Run: `cd desktop && npm run build`
Expected: 成功。

- [ ] **Step 3: 提交**

```bash
git add desktop/src/screens/CertsScreen.tsx desktop/src/theme/components.css
git commit -m "feat(certs): CertsScreen 界面(签发表单/门禁空态/五步进度/DNS 手动/证书列表与菜单)"
```

---

### Task 8: 端到端验证

**Files:** 无代码改动(除非发现缺陷)。

- [ ] **Step 1: 全量构建与测试**

Run: `cd desktop/src-tauri && cargo test && cd .. && npm test && npm run build`
Expected: 全部 PASS。

- [ ] **Step 2: 运行中的客户端 UI 验证(控制器可做的部分)**

侧栏出现「证书」入口;未连接任何主机时 `/certs` 显示「去连接主机」空态;连接一台主机后签发卡出现域名/部署主机/challenge 分段/申请按钮。这些不需要真实 CA。

- [ ] **Step 3: 真实签发(用户执行)**

用户用真实域名 + 真实主机跑一次 HTTP-01 签发,确认五步流水线、证书入列、有效期进度、reload 生效;DNS-01 手动模式确认 TXT 记录展示与验证。控制器无法代跑,如实记录用户反馈。
