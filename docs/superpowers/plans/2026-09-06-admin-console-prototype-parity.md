# 管理平台原型对齐(S3 存储页 + 11 项功能开关全量实现)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `server/`(管理平台)与 `desktop/`(客户端)完整实现 Claude Design 远端原型 `Vela 管理平台.dc.html`:三页签控制台(用户与同步 / S3 存储 / 功能开关)、S3 配置入库可编辑热生效、11 项功能开关全部有真实语义并由客户端消费。

**Architecture:** 服务端 `PlatformConfig` 拆成"部署配置"(admin_key / 门禁总闸 / SMTP)和运行期可改的 `StorageConfig`(`RwLock`,DB 优先、环境变量兜底)。功能开关统一走 `platform_settings` 表的 `feature_*` 键,分"产品开关"(受门禁总闸)与"安全策略"(始终生效)两类,经 `/api/features` 下发;客户端 `features.rs` / `stores/features.ts` 扩字段并每 60 s 刷新。邮箱验证与 TOTP 是新增的账户状态列 + 接口,客户端在账户卡里走完流程;录制同步是 `sync.rs` 新增的一段独立推拉。

**Tech Stack:** Rust(axum 0.8 / rusqlite / rust-s3 0.35 / hmac+sha1 / base32 / lettre 0.11)、Tauri 2 + React 18 + Zustand、原生 HTML 管理控制台(`server/src/admin.html`,`include_str!` 内嵌)。

**Spec:** 远端原型 `Vela 管理平台.dc.html`(Claude Design 项目 d49d6720,通过 DesignSync `get_file` 读取);本地基线见 git `7d1f0ba^:design/Vela 管理平台.dc.html`。用户已确认:S3 页"完整可编辑并热生效",功能开关"全部实现含客户端"。

## Global Constraints

- 命名一律 KiteTerm(原型里的 "Vela 管理平台 / VELA_ADMIN_KEY" 是改名前遗留,以 `KITE_*` 为准)。
- 管理控制台是单文件 `server/src/admin.html`,无构建步骤,样式只用文件顶部已定义的 Vela 语义 token(`--page/--card/--accent/...`),不新增硬编码色值。
- 客户端样式只用 `desktop/src/theme/` 里的 token 与 `utilities.css` 工具类,禁止内联色值。
- 所有新接口沿用现有错误形态 `{ "error": { "code", "message" } }`,错误码全大写下划线(如 `EMAIL_UNVERIFIED`),客户端 `friendlyError` 靠前缀 `CODE: ` 识别。
- 功能门禁总闸 `KITE_FEATURE_GATING` 仍是部署配置;它只作用于"产品开关"(login/sync/ai/keySync/recordingSync/aiCommand/aiDiagnose/aiSummary),"安全策略"(registration/emailVerify/totpRequired/multiDevice)不受总闸影响。
- 提交:每个任务末尾的 commit 步骤只在用户确认允许提交时执行;否则跳过该步,保持工作区改动。
- 服务端验证用本地实例:`KITE_LISTEN=127.0.0.1:8799 KITE_DB=<scratch>/t.db KITE_ADMIN_KEY=test-admin-key S3_ENDPOINT=http://127.0.0.1:9 S3_ACCESS_KEY=x S3_SECRET_KEY=y KITE_FEATURE_GATING=on cargo run`。

---

## 文件结构

**server/**
- `src/main.rs` — 路由、账户与管理接口(已有,改造 `PlatformConfig`/`AppState`,新增迁移与接口)
- `src/features.rs`(新)— 功能开关注册表:名字、默认值、是否受门禁;`raw()` / `effective()` / `payload()`
- `src/storage.rs`(新)— `StorageConfig`:加载(DB→env)、保存、`bucket()`、`user_prefix()`、`probe()`(测试连接)
- `src/totp.rs`(新)— RFC 6238:密钥生成、校验、otpauth URL
- `src/mail.rs`(新)— 验证码邮件:SMTP(lettre)或回退打日志
- `src/sts.rs` — `assume_role_for_user` 增加 `prefix` 参数
- `src/admin.html` — 三页签控制台
- `Cargo.toml` — 新增 `sha1`、`base32`、`lettre`

**desktop/src-tauri/src/**
- `features.rs` — `Features` 扩字段
- `account.rs` — `AccountInfo` 扩字段;`login(totp)`;新增 `verify_email` / `resend_verification` / `totp_setup` / `totp_enable` / `totp_disable` / `refresh`
- `ai.rs` — `AskMode::Summary`
- `sync.rs` — `scope_allows` 看 `key_sync` 开关;新增录制推拉 `sync_recordings()`
- `db.rs` — `set_recording_sync` / `insert_recording_with_id` / `recording_exists`
- `commands.rs` + `lib.rs` — 新命令注册

**desktop/src/**
- `types.ts`、`ipc/api.ts`、`ipc/tauri.ts` — 契约
- `stores/features.ts` — 字段 + 60 s 轮询;`stores/sync.ts` — 账户动作;`stores/ai.ts` — `summarize`
- `lib/errorDetect.ts`(新)+ `lib/errorDetect.test.ts`(新)— 终端错误识别
- `screens/sync/AccountCard.tsx`、`screens/sync/AccountSummary.tsx`、`screens/sync/TotpCard.tsx`(新)、`screens/sync/ScopeCard.tsx`
- `screens/TerminalScreen.tsx`、`screens/terminal/RightDock.tsx`、`components/AiDock.tsx`、`screens/SettingsScreen.tsx`、`screens/KeysScreen.tsx`

---

### Task 1: 服务端功能开关注册表 + `/api/features` 扩展

**Files:**
- Create: `server/src/features.rs`
- Modify: `server/src/main.rs`(删除 `feature_raw`/`feature_on`,改 `features()`、`settings_payload()`、`AdminSettingsBody`、`admin_set_settings()`,`login()`/`sync_credentials()` 的调用点)

**Interfaces:**
- Produces: `features::raw(conn, name) -> bool`、`features::effective(conn, gating, name) -> bool`、`features::payload(conn, gating) -> serde_json::Map`(camelCase 键)、`features::NAMES: &[(&str, bool, bool)]`(名, 默认, 受门禁)、`features::set(conn, name, on)`。

- [ ] **Step 1: 写注册表与单元测试**

`server/src/features.rs`:

```rust
//! 功能开关注册表。两类:
//! - 产品开关(gated=true):受部署总闸 `KITE_FEATURE_GATING` 约束,总闸关时恒为开;
//! - 安全策略(gated=false):始终按设置值生效(邮箱验证、强制 2FA、多设备)。
//! 注册策略(open/invite/closed)不在这里,它是三档而非布尔,见 main.rs。

use rusqlite::Connection;
use serde_json::{json, Map, Value};

/// (名字, 默认值, 是否受门禁总闸)
pub const NAMES: &[(&str, bool, bool)] = &[
    ("login", true, true),
    ("sync", true, true),
    ("ai", true, true),
    ("key_sync", true, true),
    ("recording_sync", true, true),
    ("ai_command", true, true),
    ("ai_diagnose", true, true),
    ("ai_summary", true, true),
    ("email_verify", false, false),
    ("totp_required", false, false),
    ("multi_device", true, false),
];

fn meta(name: &str) -> (bool, bool) {
    NAMES
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, d, g)| (*d, *g))
        .unwrap_or((true, true))
}

pub fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in name.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.push(c.to_ascii_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// 原始设置值(不看总闸)—— 控制台展示用
pub fn raw(conn: &Connection, name: &str) -> bool {
    let (default, _) = meta(name);
    crate::setting_get(conn, &format!("feature_{name}"), if default { "1" } else { "0" }) == "1"
}

/// 生效值:受门禁的开关在总闸关闭时一律视为开
pub fn effective(conn: &Connection, gating: bool, name: &str) -> bool {
    let (_, gated) = meta(name);
    if gated && !gating {
        return true;
    }
    raw(conn, name)
}

pub fn set(conn: &Connection, name: &str, on: bool) -> rusqlite::Result<()> {
    crate::setting_set(conn, &format!("feature_{name}"), if on { "1" } else { "0" })
}

/// `/api/features` 里的开关部分(camelCase)
pub fn payload(conn: &Connection, gating: bool) -> Map<String, Value> {
    let mut m = Map::new();
    for (name, _, _) in NAMES {
        m.insert(camel(name), json!(effective(conn, gating, name)));
    }
    m
}

/// 管理接口里的原始值(`featureXxx` 键)
pub fn admin_payload(conn: &Connection) -> Map<String, Value> {
    let mut m = Map::new();
    for (name, _, _) in NAMES {
        let key = format!("feature{}", {
            let c = camel(name);
            let mut cs = c.chars();
            match cs.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + cs.as_str(),
                None => String::new(),
            }
        });
        m.insert(key, json!(raw(conn, name)));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE platform_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)").unwrap();
        c
    }

    #[test]
    fn defaults_and_gating() {
        let c = db();
        assert!(raw(&c, "login"));
        assert!(!raw(&c, "email_verify"));
        assert!(raw(&c, "multi_device"));
        set(&c, "login", false).unwrap();
        assert!(!raw(&c, "login"));
        // 总闸关:产品开关恒为开
        assert!(effective(&c, false, "login"));
        assert!(!effective(&c, true, "login"));
        // 安全策略不看总闸
        set(&c, "totp_required", true).unwrap();
        assert!(effective(&c, false, "totp_required"));
    }

    #[test]
    fn camel_keys() {
        assert_eq!(camel("recording_sync"), "recordingSync");
        let p = payload(&db(), true);
        assert!(p.contains_key("aiDiagnose"));
        let a = admin_payload(&db());
        assert_eq!(a["featureKeySync"], json!(true));
    }
}
```

- [ ] **Step 2: 接进 main.rs**

在 `main.rs` 顶部 `mod sts;` 旁加 `mod features;`,把 `setting_get`/`setting_set` 改为 `pub(crate) fn`。删除原 `feature_raw` 与 `feature_on`,把 `login()` 里的 `feature_on(&state, &conn, "login")` 改成 `features::effective(&conn, state.cfg.feature_gating, "login")`,`sync_credentials()` 同理改 `"sync"`。

`features()` handler 改为:

```rust
async fn features(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let conn = state.db.lock().unwrap();
    let mut body = features::payload(&conn, state.cfg.feature_gating);
    body.insert("registration".into(), json!(setting_get(&conn, "registration_mode", &state.cfg.registration_default)));
    body.insert("enforced".into(), json!(state.cfg.feature_gating));
    body.insert("passwordMinLength".into(), json!(PASSWORD_MIN_LEN));
    body.insert("passwordRule".into(), json!("至少 8 位,且包含字母、数字、符号中的至少两类"));
    Ok(Json(serde_json::Value::Object(body)))
}
```

`settings_payload()` 改为:

```rust
fn settings_payload(conn: &Connection, state: &AppState) -> serde_json::Value {
    let mut m = features::admin_payload(conn);
    m.insert("registrationMode".into(), json!(setting_get(conn, "registration_mode", &state.cfg.registration_default)));
    m.insert("defaultSyncEnabled".into(), json!(setting_get(conn, "default_sync_enabled", "1") == "1"));
    m.insert("featureGating".into(), json!(state.cfg.feature_gating));
    serde_json::Value::Object(m)
}
```

`AdminSettingsBody` 改成通用形态,`admin_set_settings` 循环写入:

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminSettingsBody {
    registration_mode: Option<String>,
    default_sync_enabled: Option<bool>,
    /// featureLogin / featureKeySync / … 任意子集
    #[serde(flatten)]
    features: HashMap<String, serde_json::Value>,
}

// admin_set_settings 内,替换原来的 for (feature, value) in [...] 段:
for (key, value) in &body.features {
    let Some(rest) = key.strip_prefix("feature") else { continue };
    let Some(on) = value.as_bool() else { continue };
    // featureKeySync → key_sync
    let mut snake = String::new();
    for (i, c) in rest.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 { snake.push('_'); }
        snake.push(c.to_ascii_lowercase());
    }
    if !features::NAMES.iter().any(|(n, _, _)| *n == snake) {
        return Err(err(StatusCode::BAD_REQUEST, "unknown_feature", &format!("未知功能开关 {key}")));
    }
    features::set(&conn, &snake, on).map_err(internal)?;
}
```

- [ ] **Step 3: 跑测试与编译**

Run: `cd server && cargo test features:: 2>&1 | tail -5`
Expected: `test result: ok. 2 passed`

- [ ] **Step 4: 起实例用 curl 验证**

```bash
curl -s http://127.0.0.1:8799/api/features
# 期望含 "keySync":true,"emailVerify":false,"totpRequired":false,"multiDevice":true
curl -s -X POST http://127.0.0.1:8799/api/admin/settings -H 'x-admin-key: test-admin-key' -H 'content-type: application/json' -d '{"featureAiSummary":false,"featureTotpRequired":true}'
# 期望返回体 "featureAiSummary":false,"featureTotpRequired":true
curl -s -X POST http://127.0.0.1:8799/api/admin/settings -H 'x-admin-key: test-admin-key' -H 'content-type: application/json' -d '{"featureNope":true}'
# 期望 400 unknown_feature
```

- [ ] **Step 5: Commit(需用户允许)**

```bash
git add server/src/features.rs server/src/main.rs
git commit -m "feat(server): 功能开关注册表,11 项开关经 /api/features 下发"
```

---

### Task 2: S3 存储配置入库、热生效、测试连接

**Files:**
- Create: `server/src/storage.rs`
- Modify: `server/src/main.rs`(`PlatformConfig` 去掉 s3_* 与 sts_mode;`AppState` 加 `storage: RwLock<StorageConfig>`;`platform_bucket`、`sync_credentials`、`admin_stats`、`admin_usage`、`main()` 启动段;新增 3 个管理路由)
- Modify: `server/src/sts.rs`(`assume_role_for_user` 与 `session_policy` 增加 `prefix`)

**Interfaces:**
- Produces: `StorageConfig { endpoint, region, bucket, prefix, access_key, secret_key, path_style: bool, sts_mode: String }`;`storage::load(conn) -> StorageConfig`;`storage::save(conn, &cfg)`;`cfg.bucket() -> Result<Box<s3::Bucket>, String>`;`cfg.user_prefix(user_id) -> String`(形如 `prod/users/{id}`);`cfg.public_json(source) -> Value`(secret 打码);`storage::probe(&cfg).await -> ProbeReport`。
- Consumes: `sts::assume_role_for_user(endpoint, region, ak, sk, bucket, prefix, user_id, secs)`。

- [ ] **Step 1: 写 storage.rs 与单元测试**

```rust
//! S3 存储配置:运行期可改,DB 优先、环境变量兜底(仅作初值)。
//! 密钥明文落 SQLite(与此前明文放环境变量的强度一致),DB 文件权限由部署方保证。

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const KEY: &str = "storage_config";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StorageConfig {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    /// 空或形如 `prod/`(规范化后必以 `/` 结尾)
    #[serde(default)]
    pub prefix: String,
    pub access_key: String,
    pub secret_key: String,
    /// MinIO / 自建网关需开启;AWS S3 / R2 关闭
    #[serde(default = "default_true")]
    pub path_style: bool,
    /// auto | require | off
    #[serde(default = "default_sts")]
    pub sts_mode: String,
}

fn default_true() -> bool { true }
fn default_sts() -> String { "auto".into() }

/// `prod` / `/prod/` / `prod//` → `prod/`;空 → 空
pub fn normalize_prefix(raw: &str) -> String {
    let t = raw.trim().trim_matches('/');
    if t.is_empty() { String::new() } else { format!("{t}/") }
}

impl StorageConfig {
    pub fn from_env() -> Self {
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        Self {
            endpoint: env("S3_ENDPOINT"),
            region: { let r = env("S3_REGION"); if r.is_empty() { "us-east-1".into() } else { r } },
            bucket: { let b = env("S3_BUCKET"); if b.is_empty() { "vela-sync".into() } else { b } },
            prefix: normalize_prefix(&env("S3_PREFIX")),
            access_key: env("S3_ACCESS_KEY"),
            secret_key: env("S3_SECRET_KEY"),
            path_style: !matches!(env("S3_PATH_STYLE").to_ascii_lowercase().as_str(), "off" | "0" | "false"),
            sts_mode: { let m = env("KITE_STS"); if m.is_empty() { "auto".into() } else { m } },
        }
    }

    pub fn normalized(mut self) -> Self {
        self.endpoint = self.endpoint.trim().trim_end_matches('/').to_string();
        self.region = self.region.trim().to_string();
        self.bucket = self.bucket.trim().to_string();
        self.prefix = normalize_prefix(&self.prefix);
        self.access_key = self.access_key.trim().to_string();
        if !["auto", "require", "off"].contains(&self.sts_mode.as_str()) {
            self.sts_mode = "auto".into();
        }
        self
    }

    pub fn is_configured(&self) -> bool {
        !self.endpoint.is_empty() && !self.bucket.is_empty() && !self.access_key.is_empty()
    }

    pub fn user_prefix(&self, user_id: &str) -> String {
        format!("{}users/{user_id}", self.prefix)
    }

    pub fn bucket(&self) -> Result<Box<s3::Bucket>, String> {
        let region = s3::Region::Custom { region: self.region.clone(), endpoint: self.endpoint.clone() };
        let creds = s3::creds::Credentials::new(Some(&self.access_key), Some(&self.secret_key), None, None, None)
            .map_err(|e| e.to_string())?;
        let b = s3::Bucket::new(&self.bucket, region, creds).map_err(|e| e.to_string())?;
        Ok(if self.path_style { b.with_path_style() } else { b })
    }

    /// 对外形态:secret 只回传"是否已设置"
    pub fn public_json(&self, source: &str) -> Value {
        json!({
            "endpoint": self.endpoint, "region": self.region, "bucket": self.bucket,
            "prefix": self.prefix, "accessKey": self.access_key,
            "hasSecret": !self.secret_key.is_empty(),
            "pathStyle": self.path_style, "stsMode": self.sts_mode,
            "configured": self.is_configured(), "source": source,
        })
    }
}

/// DB 有就用 DB(source=db),否则环境变量(source=env)
pub fn load(conn: &Connection) -> (StorageConfig, &'static str) {
    let from_db = crate::setting_get(conn, KEY, "");
    if !from_db.is_empty() {
        if let Ok(c) = serde_json::from_str::<StorageConfig>(&from_db) {
            return (c.normalized(), "db");
        }
    }
    (StorageConfig::from_env().normalized(), "env")
}

pub fn save(conn: &Connection, cfg: &StorageConfig) -> rusqlite::Result<()> {
    crate::setting_set(conn, KEY, &serde_json::to_string(cfg).unwrap_or_default())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReport {
    pub ok: bool,
    pub latency_ms: u128,
    pub list: Option<String>,
    pub put: Option<String>,
    pub delete: Option<String>,
    pub sts: Option<String>,
}

/// 探测:ListBucket → Put 探针 → Delete 探针;开启 STS 时再试一次 AssumeRole。
/// 每项 None 表示通过,Some(msg) 表示失败原因。
pub async fn probe(cfg: &StorageConfig) -> ProbeReport {
    let t0 = std::time::Instant::now();
    let mut r = ProbeReport { ok: false, latency_ms: 0, list: None, put: None, delete: None, sts: None };
    let bucket = match cfg.bucket() {
        Ok(b) => b,
        Err(e) => { r.list = Some(e); r.latency_ms = t0.elapsed().as_millis(); return r; }
    };
    if let Err(e) = bucket.list(cfg.prefix.clone(), Some("/".into())).await {
        r.list = Some(e.to_string());
    }
    let probe_key = format!("{}.kite-probe", cfg.prefix);
    match bucket.put_object(&probe_key, b"probe").await {
        Ok(_) => {
            if let Err(e) = bucket.delete_object(&probe_key).await { r.delete = Some(e.to_string()); }
        }
        Err(e) => r.put = Some(e.to_string()),
    }
    if cfg.sts_mode != "off" {
        if let Err(e) = crate::sts::assume_role_for_user(
            &cfg.endpoint, &cfg.region, &cfg.access_key, &cfg.secret_key, &cfg.bucket, &cfg.prefix, "probe", 900,
        ).await {
            r.sts = Some(e);
        }
    }
    r.latency_ms = t0.elapsed().as_millis();
    r.ok = r.list.is_none() && r.put.is_none() && r.delete.is_none() && (cfg.sts_mode != "require" || r.sts.is_none());
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_normalization() {
        assert_eq!(normalize_prefix(""), "");
        assert_eq!(normalize_prefix("prod"), "prod/");
        assert_eq!(normalize_prefix("/prod//"), "prod/");
        let c = StorageConfig { endpoint: "http://x/".into(), region: "r".into(), bucket: "b".into(), prefix: "p".into(),
            access_key: "a".into(), secret_key: "s".into(), path_style: true, sts_mode: "weird".into() }.normalized();
        assert_eq!(c.endpoint, "http://x");
        assert_eq!(c.user_prefix("u1"), "p/users/u1");
        assert_eq!(c.sts_mode, "auto");
        assert_eq!(c.public_json("db")["hasSecret"], json!(true));
        assert!(c.public_json("db").get("secretKey").is_none());
    }

    #[test]
    fn db_roundtrip_and_env_fallback() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE platform_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)").unwrap();
        let (_, src) = load(&conn);
        assert_eq!(src, "env");
        let c = StorageConfig { endpoint: "http://m".into(), region: "r".into(), bucket: "b".into(), prefix: "".into(),
            access_key: "a".into(), secret_key: "s".into(), path_style: false, sts_mode: "off".into() };
        save(&conn, &c).unwrap();
        let (got, src) = load(&conn);
        assert_eq!(src, "db");
        assert_eq!(got, c);
    }
}
```

- [ ] **Step 2: sts.rs 加 prefix**

`session_policy(bucket, prefix, user_id)`:资源改为 `format!("arn:aws:s3:::{bucket}/{prefix}users/{user_id}/*")`,ListBucket 条件改为 `format!("{prefix}users/{user_id}/*")`。`assume_role_for_user` 签名在 `bucket` 后加 `prefix: &str` 并透传。

- [ ] **Step 3: main.rs 改造**

`PlatformConfig` 只保留 `admin_key`、`registration_default`、`feature_gating`。`AppState` 加 `storage: std::sync::RwLock<storage::StorageConfig>` 与 `storage_source: Mutex<&'static str>`。启动:

```rust
let (storage_cfg, source) = { storage::load(&conn) };
if storage_cfg.is_configured() {
    match storage_cfg.bucket() {
        Ok(b) => match b.put_object(&format!("{}.kite-init", storage_cfg.prefix), b"").await {
            Ok(_) => { let _ = b.delete_object(&format!("{}.kite-init", storage_cfg.prefix)).await; println!("[server] 存储 {} 可用({source})", storage_cfg.bucket); }
            Err(e) => println!("[server] 存储探测失败({source}),先启动再到控制台修正: {e}"),
        },
        Err(e) => println!("[server] 存储配置无效: {e}"),
    }
} else {
    println!("[server] 存储尚未配置 —— 到控制台「S3 存储」页填写");
}
```
(删除原来的 `create_with_path_style` 建桶段;`S3_*` 环境变量不再 `expect`。)

`platform_bucket` 删除,`admin_usage` 改成 `let cfg = state.storage.read().unwrap().clone(); let bucket = cfg.bucket().map_err(|e| err(StatusCode::BAD_GATEWAY, "s3_error", &e))?; bucket.list(format!("{}/", cfg.user_prefix(&user_id)), None)`。

`sync_credentials`:开头 `let cfg = state.storage.read().unwrap().clone(); if !cfg.is_configured() { return Err(err(StatusCode::SERVICE_UNAVAILABLE, "STORAGE_UNCONFIGURED", "平台尚未配置存储,请联系管理员")); }`;`prefix = cfg.user_prefix(&user.id)`;STS 调用改传 `&cfg.endpoint, &cfg.region, &cfg.access_key, &cfg.secret_key, &cfg.bucket, &cfg.prefix, &user.id, 3600`;两处响应里的 endpoint/bucket/region/accessKey/secretKey 都改读 `cfg`,并新增 `"pathStyle": cfg.path_style`。

`admin_stats`:`"bucket": cfg.bucket, "endpoint": cfg.endpoint, "stsMode": cfg.sts_mode, "storageConfigured": cfg.is_configured()`。

新增管理接口:

```rust
async fn admin_get_storage(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let cfg = state.storage.read().unwrap().clone();
    let source = *state.storage_source.lock().unwrap();
    Ok(Json(cfg.public_json(source)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageBody {
    endpoint: String,
    #[serde(default)] region: String,
    bucket: String,
    #[serde(default)] prefix: String,
    access_key: String,
    /// 空 = 沿用已保存的 secret
    #[serde(default)] secret_key: String,
    #[serde(default = "storage_true")] path_style: bool,
    #[serde(default = "storage_auto")] sts_mode: String,
}
fn storage_true() -> bool { true }
fn storage_auto() -> String { "auto".into() }

fn storage_from_body(state: &AppState, body: StorageBody) -> Result<storage::StorageConfig, ApiError> {
    let current = state.storage.read().unwrap().clone();
    let secret = if body.secret_key.is_empty() { current.secret_key } else { body.secret_key };
    let cfg = storage::StorageConfig {
        endpoint: body.endpoint, region: if body.region.is_empty() { "us-east-1".into() } else { body.region },
        bucket: body.bucket, prefix: body.prefix, access_key: body.access_key, secret_key: secret,
        path_style: body.path_style, sts_mode: body.sts_mode,
    }.normalized();
    if !cfg.endpoint.starts_with("http://") && !cfg.endpoint.starts_with("https://") {
        return Err(err(StatusCode::BAD_REQUEST, "invalid_endpoint", "Endpoint 须以 http:// 或 https:// 开头"));
    }
    if cfg.bucket.is_empty() || cfg.access_key.is_empty() || cfg.secret_key.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "invalid_storage", "Bucket、Access Key、Secret 不能为空"));
    }
    Ok(cfg)
}

/// 保存并热生效:之后的凭据下发、用量统计、STS 都用新配置
async fn admin_set_storage(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(body): Json<StorageBody>) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let cfg = storage_from_body(&state, body)?;
    {
        let conn = state.db.lock().unwrap();
        storage::save(&conn, &cfg).map_err(internal)?;
    }
    *state.storage.write().unwrap() = cfg.clone();
    *state.storage_source.lock().unwrap() = "db";
    Ok(Json(cfg.public_json("db")))
}

/// 用表单里的值(未保存)探测;secret 留空则用已保存的
async fn admin_test_storage(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(body): Json<StorageBody>) -> Result<Json<storage::ProbeReport>, ApiError> {
    require_admin(&state, &headers)?;
    let cfg = storage_from_body(&state, body)?;
    Ok(Json(storage::probe(&cfg).await))
}
```

路由:`.route("/api/admin/storage", get(admin_get_storage).post(admin_set_storage))`、`.route("/api/admin/storage/test", post(admin_test_storage))`。文件头注释的环境变量说明补 `S3_PREFIX`、`S3_PATH_STYLE`,并注明"S3_* 仅作初值,控制台保存后以 DB 为准"。

- [ ] **Step 4: 测试、编译、curl**

Run: `cd server && cargo test 2>&1 | tail -3` → 全部通过。

```bash
curl -s http://127.0.0.1:8799/api/admin/storage -H 'x-admin-key: test-admin-key'
# 期望 "source":"env","hasSecret":true,"endpoint":"http://127.0.0.1:9"
curl -s -X POST http://127.0.0.1:8799/api/admin/storage/test -H 'x-admin-key: test-admin-key' -H 'content-type: application/json' -d '{"endpoint":"http://127.0.0.1:9","bucket":"b","accessKey":"a","secretKey":"s","stsMode":"off"}'
# 期望 "ok":false 且 list/put 带 connection refused 文案
curl -s -X POST http://127.0.0.1:8799/api/admin/storage -H 'x-admin-key: test-admin-key' -H 'content-type: application/json' -d '{"endpoint":"http://127.0.0.1:9","bucket":"b2","prefix":"/prod/","accessKey":"a","secretKey":"","stsMode":"off"}'
# 期望 "source":"db","prefix":"prod/","hasSecret":true(沿用了旧 secret)
curl -s http://127.0.0.1:8799/api/admin/stats -H 'x-admin-key: test-admin-key'
# 期望 "bucket":"b2"(热生效,无需重启)
```

- [ ] **Step 5: Commit(需用户允许)**

```bash
git add server/src/storage.rs server/src/sts.rs server/src/main.rs
git commit -m "feat(server): S3 存储配置入库,控制台可改并热生效,支持测试连接/前缀/path-style"
```

---

### Task 3: 邮箱验证(迁移、验证码邮件、接口、管理动作)

**Files:**
- Create: `server/src/mail.rs`
- Modify: `server/Cargo.toml`(加 `lettre`)
- Modify: `server/src/main.rs`(`users` 表迁移、`email_codes` 表、`register()`/`login()`/`me()`/`sync_credentials()`、新增 `verify_email`/`resend_verification`/`admin_verify_user`、`admin_list_users` 加字段、`PlatformConfig` 加 `smtp: Option<mail::Smtp>`)

**Interfaces:**
- Produces: `mail::Smtp::from_env() -> Option<Smtp>`、`mail::send_code(smtp: Option<&Smtp>, to, code).await -> Result<(), String>`(无 SMTP 时打印日志并返回 Ok)、`add_column_if_missing(conn, table, col, decl)`、错误码 `EMAIL_UNVERIFIED`、`CODE_INVALID`、`CODE_EXPIRED`、`RESEND_TOO_SOON`。
- 登录/注册/`/api/me` 响应 `user` 对象新增 `emailVerified: bool`,`/api/me` 另含 `totpEnabled`(Task 4 填)与 `requirements: { emailVerify, totpRequired }`。

- [ ] **Step 1: Cargo.toml 加依赖**

```toml
lettre = { version = "0.11", default-features = false, features = ["builder", "smtp-transport", "tokio1", "tokio1-rustls-tls"] }
```

- [ ] **Step 2: mail.rs**

```rust
//! 验证码邮件。配置了 KITE_SMTP_HOST 就走 SMTP(STARTTLS),否则把验证码打进服务端日志 ——
//! 小规模自建没有邮件服务时,管理员在控制台也能直接「标记已验证」。

use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

#[derive(Clone)]
pub struct Smtp {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub from: String,
}

impl Smtp {
    /// KITE_SMTP_HOST / KITE_SMTP_PORT(587) / KITE_SMTP_USER / KITE_SMTP_PASS / KITE_SMTP_FROM
    pub fn from_env() -> Option<Self> {
        let host = std::env::var("KITE_SMTP_HOST").ok()?.trim().to_string();
        if host.is_empty() { return None; }
        let user = std::env::var("KITE_SMTP_USER").unwrap_or_default();
        Some(Self {
            port: std::env::var("KITE_SMTP_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(587),
            pass: std::env::var("KITE_SMTP_PASS").unwrap_or_default(),
            from: std::env::var("KITE_SMTP_FROM").unwrap_or_else(|_| user.clone()),
            user, host,
        })
    }
}

pub fn new_code() -> String {
    use rand::Rng;
    format!("{:06}", rand::thread_rng().gen_range(0..1_000_000u32))
}

pub async fn send_code(smtp: Option<&Smtp>, to: &str, code: &str) -> Result<(), String> {
    let Some(s) = smtp else {
        println!("[mail] 未配置 SMTP —— {to} 的邮箱验证码: {code}(10 分钟内有效)");
        return Ok(());
    };
    let msg = Message::builder()
        .from(s.from.parse().map_err(|e| format!("发件人无效: {e}"))?)
        .to(to.parse().map_err(|e| format!("收件人无效: {e}"))?)
        .subject("KiteTerm 邮箱验证码")
        .body(format!("你的 KiteTerm 邮箱验证码是 {code},10 分钟内有效。若非本人操作请忽略。"))
        .map_err(|e| e.to_string())?;
    let mut builder = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&s.host)
        .map_err(|e| e.to_string())?
        .port(s.port);
    if !s.user.is_empty() {
        builder = builder.credentials(Credentials::new(s.user.clone(), s.pass.clone()));
    }
    builder.build().send(msg).await.map(|_| ()).map_err(|e| format!("SMTP 发送失败: {e}"))
}
```

- [ ] **Step 3: 迁移与表**

`main()` 建表 SQL 后追加:

```rust
add_column_if_missing(&conn, "users", "email_verified", "INTEGER NOT NULL DEFAULT 1");
add_column_if_missing(&conn, "users", "totp_secret", "TEXT");
add_column_if_missing(&conn, "users", "totp_enabled", "INTEGER NOT NULL DEFAULT 0");
conn.execute_batch(
    "CREATE TABLE IF NOT EXISTS email_codes (
        user_id TEXT PRIMARY KEY, code TEXT NOT NULL, expires_at INTEGER NOT NULL, sent_at INTEGER NOT NULL
    );",
).expect("email_codes 建表失败");
```

```rust
/// 存量库无该列时补列;存量用户视为已验证(DEFAULT 1)
fn add_column_if_missing(conn: &Connection, table: &str, col: &str, decl: &str) {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).expect("pragma");
    let has = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .expect("pragma rows")
        .filter_map(|c| c.ok())
        .any(|c| c == col);
    if !has {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {col} {decl}"), [])
            .unwrap_or_else(|e| panic!("迁移 {table}.{col} 失败: {e}"));
    }
}
```

- [ ] **Step 4: 注册/登录/me/凭据下发**

`AuthedUser` 加 `email_verified: bool, totp_enabled: bool`,`authenticate` 的 SELECT 改为 `u.id, u.email, u.sync_enabled, u.email_verified, u.totp_enabled, t.expires_at`。

`register()`:在 INSERT 之前 `let need_verify = features::raw(&conn, "email_verify");`,INSERT 语句加 `email_verified` 列 `if need_verify { 0 } else { 1 }`;成功后:

```rust
if need_verify {
    let code = mail::new_code();
    conn.execute(
        "INSERT INTO email_codes (user_id, code, expires_at, sent_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(user_id) DO UPDATE SET code=?2, expires_at=?3, sent_at=?4",
        params![id, code, now_ts() + 600, now_ts()],
    ).map_err(internal)?;
    let smtp = state.cfg.smtp.clone();
    let to = email.clone();
    tokio::spawn(async move {
        if let Err(e) = mail::send_code(smtp.as_ref(), &to, &code).await { eprintln!("[mail] {e}"); }
    });
}
```
响应 `user` 加 `"emailVerified": !need_verify`。注意:`conn` 是 `MutexGuard`,`tokio::spawn` 前不要持有它跨 await —— 这里 spawn 的 future 不引用 conn,可以。

`login()`:SELECT 加 `email_verified, totp_enabled`;响应 `user` 加 `"emailVerified": email_verified != 0`。

`me()` 改成返回 `serde_json::Value`:

```rust
Ok(Json(json!({
    "id": user.id, "email": user.email, "syncEnabled": user.sync_enabled,
    "emailVerified": user.email_verified, "totpEnabled": user.totp_enabled,
    "requirements": {
        "emailVerify": features::raw(&conn, "email_verify"),
        "totpRequired": features::raw(&conn, "totp_required"),
    },
})))
```
(删除 `UserOut` 结构体。)

`sync_credentials()`:在 sync 开关检查后加
```rust
if features::raw(&conn, "email_verify") && !user.email_verified {
    return Err(err(StatusCode::FORBIDDEN, "EMAIL_UNVERIFIED", "请先完成邮箱验证"));
}
```

新接口:

```rust
#[derive(Deserialize)]
struct CodeBody { code: String }

async fn verify_email(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(body): Json<CodeBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let row: Option<(String, i64)> = conn
        .query_row("SELECT code, expires_at FROM email_codes WHERE user_id = ?1", [&user.id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional().map_err(internal)?;
    let Some((code, expires_at)) = row else {
        return Err(err(StatusCode::BAD_REQUEST, "CODE_INVALID", "验证码不存在,请重新发送"));
    };
    if expires_at < now_ts() {
        return Err(err(StatusCode::BAD_REQUEST, "CODE_EXPIRED", "验证码已过期,请重新发送"));
    }
    if code != body.code.trim() {
        return Err(err(StatusCode::BAD_REQUEST, "CODE_INVALID", "验证码错误"));
    }
    conn.execute("UPDATE users SET email_verified = 1 WHERE id = ?1", [&user.id]).map_err(internal)?;
    conn.execute("DELETE FROM email_codes WHERE user_id = ?1", [&user.id]).map_err(internal)?;
    Ok(Json(json!({ "ok": true, "emailVerified": true })))
}

async fn resend_verification(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    let user = authenticate(&state, &headers)?;
    if user.email_verified {
        return Ok(Json(json!({ "ok": true, "emailVerified": true })));
    }
    let code = mail::new_code();
    {
        let conn = state.db.lock().unwrap();
        let last: Option<i64> = conn
            .query_row("SELECT sent_at FROM email_codes WHERE user_id = ?1", [&user.id], |r| r.get(0))
            .optional().map_err(internal)?;
        if let Some(t) = last { if now_ts() - t < 60 {
            return Err(err(StatusCode::TOO_MANY_REQUESTS, "RESEND_TOO_SOON", "发送过于频繁,请一分钟后再试"));
        } }
        conn.execute(
            "INSERT INTO email_codes (user_id, code, expires_at, sent_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(user_id) DO UPDATE SET code=?2, expires_at=?3, sent_at=?4",
            params![user.id, code, now_ts() + 600, now_ts()],
        ).map_err(internal)?;
    }
    mail::send_code(state.cfg.smtp.as_ref(), &user.email, &code).await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, "MAIL_ERROR", &e))?;
    Ok(Json(json!({ "ok": true, "emailVerified": false })))
}

/// 管理员手工标记已验证(没有邮件服务的部署)
async fn admin_verify_user(State(state): State<Arc<AppState>>, Path(email): Path<String>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let n = conn.execute("UPDATE users SET email_verified = 1 WHERE email = ?1", [email.trim().to_lowercase()]).map_err(internal)?;
    if n == 0 { return Err(err(StatusCode::NOT_FOUND, "not_found", "用户不存在")); }
    Ok(Json(json!({ "ok": true })))
}
```

`admin_list_users` SELECT 加 `email_verified, totp_enabled`,输出 `"emailVerified"`、`"totpEnabled"`。

路由:`/api/verify-email` post、`/api/verify-email/resend` post、`/api/admin/users/{email}/verify` post。`PlatformConfig` 加 `smtp: Option<mail::Smtp>`(`mail::Smtp::from_env()`),文件头注释补 SMTP 五个变量。

- [ ] **Step 5: curl 验证**

```bash
curl -s -X POST http://127.0.0.1:8799/api/admin/settings -H 'x-admin-key: test-admin-key' -H 'content-type: application/json' -d '{"featureEmailVerify":true}'
R=$(curl -s -X POST http://127.0.0.1:8799/api/register -H 'content-type: application/json' -d '{"email":"v1@vela.dev","password":"Passw0rd!Strong#1"}'); echo $R
# 期望 "emailVerified":false;服务端日志出现 [mail] ... 验证码: 123456
T=$(echo $R | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')
curl -s http://127.0.0.1:8799/api/sync/credentials -H "authorization: Bearer $T"   # 期望 403 EMAIL_UNVERIFIED
curl -s -X POST http://127.0.0.1:8799/api/verify-email -H "authorization: Bearer $T" -H 'content-type: application/json' -d '{"code":"000000"}'   # 期望 CODE_INVALID
curl -s -X POST http://127.0.0.1:8799/api/verify-email -H "authorization: Bearer $T" -H 'content-type: application/json' -d '{"code":"<日志里的码>"}'   # 期望 ok
curl -s http://127.0.0.1:8799/api/me -H "authorization: Bearer $T"   # 期望 emailVerified:true, requirements.emailVerify:true
```

- [ ] **Step 6: Commit(需用户允许)**

```bash
git add server/Cargo.toml server/Cargo.lock server/src/mail.rs server/src/main.rs
git commit -m "feat(server): 邮箱验证 —— 验证码邮件(SMTP/日志回退)、验证接口、未验证拒发同步凭据"
```

---

### Task 4: TOTP 双因素认证 + 多设备登录策略

**Files:**
- Create: `server/src/totp.rs`
- Modify: `server/Cargo.toml`(加 `sha1 = "0.10"`、`base32 = "0.5"`)
- Modify: `server/src/main.rs`(`Credentials` 加 `totp`;`login()`;新增 `totp_setup`/`totp_enable`/`totp_disable`/`admin_reset_totp`;`register()`/`login()` 多设备清理;`sync_credentials` 强制 2FA 检查)

**Interfaces:**
- Produces: `totp::new_secret() -> String`(base32 无填充)、`totp::verify(secret_b32, code, now_ts) -> bool`(±1 步)、`totp::otpauth_url(secret_b32, email) -> String`;错误码 `TOTP_REQUIRED`、`TOTP_INVALID`、`TOTP_SETUP_REQUIRED`、`TOTP_ENFORCED`、`TOTP_NOT_PENDING`。登录响应新增 `totpSetupRequired: bool`。

- [ ] **Step 1: totp.rs 与 RFC 6238 向量测试**

```rust
//! RFC 6238 TOTP(SHA-1,6 位,30 s),兼容 Google Authenticator / 1Password 等。

use hmac::{Hmac, Mac};
use sha1::Sha1;

const STEP: i64 = 30;
const ALPHABET: base32::Alphabet = base32::Alphabet::Rfc4648 { padding: false };

pub fn new_secret() -> String {
    use rand::RngCore;
    let mut raw = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut raw);
    base32::encode(ALPHABET, &raw)
}

fn hotp(secret: &[u8], counter: u64) -> u32 {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("hmac key");
    mac.update(&counter.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let off = (h[19] & 0x0f) as usize;
    let bin = ((h[off] & 0x7f) as u32) << 24
        | (h[off + 1] as u32) << 16
        | (h[off + 2] as u32) << 8
        | h[off + 3] as u32;
    bin % 1_000_000
}

pub fn code_at(secret_b32: &str, ts: i64) -> Option<String> {
    let secret = base32::decode(ALPHABET, &secret_b32.to_ascii_uppercase())?;
    Some(format!("{:06}", hotp(&secret, (ts / STEP) as u64)))
}

/// 允许前后各一个时间步(±30 s)的时钟漂移
pub fn verify(secret_b32: &str, code: &str, now: i64) -> bool {
    let code = code.trim();
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    [-1i64, 0, 1].iter().any(|d| code_at(secret_b32, now + d * STEP).as_deref() == Some(code))
}

pub fn otpauth_url(secret_b32: &str, email: &str) -> String {
    format!("otpauth://totp/KiteTerm:{email}?secret={secret_b32}&issuer=KiteTerm&algorithm=SHA1&digits=6&period=30")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_sha1_vectors() {
        // RFC 6238 附录 B 的密钥 "12345678901234567890"(8 位码取后 6 位)
        let secret = base32::encode(ALPHABET, b"12345678901234567890");
        assert_eq!(code_at(&secret, 59).unwrap(), "287082");
        assert_eq!(code_at(&secret, 1111111109).unwrap(), "081804");
        assert_eq!(code_at(&secret, 1234567890).unwrap(), "005924");
    }

    #[test]
    fn verify_window_and_format() {
        let secret = new_secret();
        assert_eq!(secret.len(), 32);
        let now = 1_700_000_000;
        let c = code_at(&secret, now).unwrap();
        assert!(verify(&secret, &c, now));
        assert!(verify(&secret, &c, now + 29));
        assert!(verify(&secret, &c, now - 30));
        assert!(!verify(&secret, &c, now + 90));
        assert!(!verify(&secret, "12345", now));
        assert!(!verify(&secret, "abcdef", now));
        assert!(otpauth_url(&secret, "a@b.c").starts_with("otpauth://totp/KiteTerm:a@b.c?secret="));
    }
}
```

- [ ] **Step 2: 登录流程**

`Credentials` 加 `#[serde(default)] totp: Option<String>`。`login()` SELECT 再加 `totp_secret, totp_enabled`;口令校验通过、登录开关检查通过后:

```rust
if totp_enabled != 0 {
    let Some(code) = body.totp.as_deref().filter(|c| !c.trim().is_empty()) else {
        return Err(err(StatusCode::UNAUTHORIZED, "TOTP_REQUIRED", "请输入动态验证码"));
    };
    if !totp::verify(totp_secret.as_deref().unwrap_or(""), code, now_ts()) {
        return Err(err(StatusCode::UNAUTHORIZED, "TOTP_INVALID", "动态验证码错误"));
    }
}
let (token, expires_at) = issue_token(&conn, &user_id)?;
// 多设备登录关闭:新登录挤掉其它设备的令牌
if !features::raw(&conn, "multi_device") {
    conn.execute("DELETE FROM tokens WHERE user_id = ?1 AND token != ?2", params![user_id, token]).map_err(internal)?;
}
let totp_setup_required = features::raw(&conn, "totp_required") && totp_enabled == 0;
Ok(Json(json!({
    "token": token, "expiresAt": expires_at, "totpSetupRequired": totp_setup_required,
    "user": { "id": user_id, "email": email, "syncEnabled": sync_enabled != 0,
              "emailVerified": email_verified != 0, "totpEnabled": totp_enabled != 0 },
})))
```
注册响应同样加 `"totpEnabled": false`、`"totpSetupRequired": features::raw(&conn, "totp_required")`。

`sync_credentials()` 在邮箱验证检查后加:
```rust
if features::raw(&conn, "totp_required") && !user.totp_enabled {
    return Err(err(StatusCode::FORBIDDEN, "TOTP_SETUP_REQUIRED", "平台要求绑定双因素认证后才能同步"));
}
```

- [ ] **Step 3: 2FA 接口**

```rust
/// 生成待确认的密钥(不启用);再次调用会换新密钥
async fn totp_setup(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    let user = authenticate(&state, &headers)?;
    if user.totp_enabled {
        return Err(err(StatusCode::CONFLICT, "TOTP_ALREADY_ENABLED", "已绑定双因素认证,如需更换请先解绑"));
    }
    let secret = totp::new_secret();
    let conn = state.db.lock().unwrap();
    conn.execute("UPDATE users SET totp_secret = ?2 WHERE id = ?1", params![user.id, secret]).map_err(internal)?;
    Ok(Json(json!({ "secret": secret, "otpauthUrl": totp::otpauth_url(&secret, &user.email) })))
}

fn user_totp_secret(conn: &Connection, user_id: &str) -> Result<Option<String>, ApiError> {
    conn.query_row("SELECT totp_secret FROM users WHERE id = ?1", [user_id], |r| r.get::<_, Option<String>>(0))
        .map_err(internal)
}

/// 用当前验证码确认绑定
async fn totp_enable(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(body): Json<CodeBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let Some(secret) = user_totp_secret(&conn, &user.id)? else {
        return Err(err(StatusCode::BAD_REQUEST, "TOTP_NOT_PENDING", "请先生成密钥"));
    };
    if !totp::verify(&secret, &body.code, now_ts()) {
        return Err(err(StatusCode::BAD_REQUEST, "TOTP_INVALID", "动态验证码错误"));
    }
    conn.execute("UPDATE users SET totp_enabled = 1 WHERE id = ?1", [&user.id]).map_err(internal)?;
    Ok(Json(json!({ "ok": true, "totpEnabled": true })))
}

/// 解绑须验证码;平台强制 2FA 时不允许解绑
async fn totp_disable(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(body): Json<CodeBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    if features::raw(&conn, "totp_required") {
        return Err(err(StatusCode::FORBIDDEN, "TOTP_ENFORCED", "平台强制双因素认证,不能解绑"));
    }
    let secret = user_totp_secret(&conn, &user.id)?.unwrap_or_default();
    if !user.totp_enabled || !totp::verify(&secret, &body.code, now_ts()) {
        return Err(err(StatusCode::BAD_REQUEST, "TOTP_INVALID", "动态验证码错误"));
    }
    conn.execute("UPDATE users SET totp_enabled = 0, totp_secret = NULL WHERE id = ?1", [&user.id]).map_err(internal)?;
    Ok(Json(json!({ "ok": true, "totpEnabled": false })))
}

/// 管理员重置(用户丢了验证器):清掉密钥并吊销令牌,下次登录重新绑定
async fn admin_reset_totp(State(state): State<Arc<AppState>>, Path(email): Path<String>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let email = email.trim().to_lowercase();
    let n = conn.execute("UPDATE users SET totp_enabled = 0, totp_secret = NULL WHERE email = ?1", [&email]).map_err(internal)?;
    if n == 0 { return Err(err(StatusCode::NOT_FOUND, "not_found", "用户不存在")); }
    conn.execute("DELETE FROM tokens WHERE user_id IN (SELECT id FROM users WHERE email = ?1)", [&email]).map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}
```

路由:`/api/2fa/setup` post、`/api/2fa/enable` post、`/api/2fa/disable` post、`/api/admin/users/{email}/2fa/reset` post。`mod totp;`。

- [ ] **Step 4: 测试与 curl**

Run: `cd server && cargo test 2>&1 | tail -3` → 通过(含 rfc6238 向量)。

```bash
T=$(curl -s -X POST http://127.0.0.1:8799/api/login -H 'content-type: application/json' -d '{"email":"lin@vela.dev","password":"Passw0rd!Strong#lin"}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')
S=$(curl -s -X POST http://127.0.0.1:8799/api/2fa/setup -H "authorization: Bearer $T" | python3 -c 'import sys,json;print(json.load(sys.stdin)["secret"])')
# 用 python 算当前码(oathtool 亦可):
CODE=$(python3 - <<PY
import base64,hmac,hashlib,struct,time
k=base64.b32decode("$S"+"="*((8-len("$S")%8)%8)); c=int(time.time())//30
h=hmac.new(k,struct.pack(">Q",c),hashlib.sha1).digest(); o=h[19]&15
print("%06d"%((struct.unpack(">I",h[o:o+4])[0]&0x7fffffff)%1000000))
PY
)
curl -s -X POST http://127.0.0.1:8799/api/2fa/enable -H "authorization: Bearer $T" -H 'content-type: application/json' -d "{\"code\":\"$CODE\"}"   # ok
curl -s -X POST http://127.0.0.1:8799/api/login -H 'content-type: application/json' -d '{"email":"lin@vela.dev","password":"Passw0rd!Strong#lin"}'   # 401 TOTP_REQUIRED
# 多设备关闭:先关开关再登录两次,活跃令牌应只剩 1 个
curl -s -X POST http://127.0.0.1:8799/api/admin/settings -H 'x-admin-key: test-admin-key' -H 'content-type: application/json' -d '{"featureMultiDevice":false}'
```

- [ ] **Step 5: Commit(需用户允许)**

```bash
git add server/Cargo.toml server/Cargo.lock server/src/totp.rs server/src/main.rs
git commit -m "feat(server): TOTP 双因素认证(可强制)与单设备登录策略"
```

---

### Task 5: 管理控制台三页签重写(用户与同步 / S3 存储 / 功能开关)

**Files:**
- Modify: `server/src/admin.html`(保留现有 token、`.btn/.input/.badge/.switch/.seg/.pill/.card/.stat/table/.toast` 样式与 `api()/toast()/fmtTime()/fmtBytes()/render()/toggleSync()/revoke()/loadUsage()/setRegMode()/toggleDefaultSync()/loadInvites()/createInvite()/copyInvite()/disableInvite()`;重排主界面为页签;新增存储页与功能页)

**Interfaces:**
- Consumes:Task 1 的 `/api/admin/settings`(`featureXxx` 键)、Task 2 的 `/api/admin/storage`(GET/POST)与 `/api/admin/storage/test`、Task 3/4 的 `emailVerified`/`totpEnabled` 字段与 `users/{email}/verify`、`users/{email}/2fa/reset`。

- [ ] **Step 1: 新增样式(追加到 `<style>` 末尾)**

```css
  .tabs{display:inline-flex;gap:4px;background:var(--card);border:1px solid var(--border);border-radius:12px;padding:4px;margin:18px 0}
  .tab{display:flex;align-items:center;gap:8px;padding:7px 16px;border-radius:9px;font-size:13px;font-weight:600;cursor:pointer;color:var(--muted);transition:all .14s}
  .tab:hover{color:var(--text)}
  .tab[data-active="true"]{background:var(--accent-subtle);color:var(--accent-text)}
  .tab svg{flex:none}
  .form-grid{display:grid;grid-template-columns:2fr 1fr;gap:14px 16px}
  .field label{display:block;font-size:11.5px;font-weight:600;color:var(--muted);margin-bottom:6px}
  .field .input{width:100%}
  .split{display:flex;align-items:center;gap:12px;margin-top:8px;padding-top:14px;border-top:1px solid var(--border)}
  .split .title{font-weight:600;font-size:13px;color:var(--text-strong)}
  .split .desc{font-size:11.5px;color:var(--faint)}
  .feat-grid{display:grid;grid-template-columns:1fr 1fr;gap:16px;align-items:start}
  .feat-card{padding:8px 20px}
  .feat-head{display:flex;align-items:center;gap:8px;padding:14px 0 6px}
  .feat-head .name{font-size:13px;font-weight:700;color:var(--text-strong)}
  .feat-row{display:flex;align-items:center;gap:12px;padding:12px 0;border-top:1px solid var(--border)}
  .feat-row[data-dim="true"]{opacity:.45}
  .icon-chip{width:34px;height:34px;flex:none;border-radius:10px;background:var(--accent-subtle);color:var(--accent-text);display:flex;align-items:center;justify-content:center}
  .badge.neutral{background:var(--sunken);color:var(--muted)}
  .probe{font-size:12px;line-height:1.7;margin-top:12px;color:var(--muted)}
  .probe .ok{color:var(--success-t)} .probe .bad{color:var(--danger-t)}
```

- [ ] **Step 2: 主界面结构**

把 `#main-view` 内的标题栏保留,其后替换为:

```html
    <div class="tabs" id="tabs">
      <div class="tab" data-tab="users" onclick="switchTab('users')">用户与同步</div>
      <div class="tab" data-tab="storage" onclick="switchTab('storage')">S3 存储</div>
      <div class="tab" data-tab="features" onclick="switchTab('features')">功能开关</div>
    </div>

    <!-- ===== 用户与同步 ===== -->
    <div id="tab-users">
      （原有 .stats 三张统计卡 + 用户表卡 + .muted-note,原样搬进来;删除原「功能开关」与「注册策略」两张卡)
    </div>

    <!-- ===== S3 存储 ===== -->
    <div id="tab-storage" class="hidden">
      <div class="card" style="padding:20px 22px;margin-bottom:16px">
        <div style="display:flex;align-items:center;gap:10px;margin-bottom:16px">
          <div class="icon-chip"><svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14a9 3 0 0 0 18 0V5"/><path d="M3 12a9 3 0 0 0 18 0"/></svg></div>
          <div style="flex:1;line-height:1.3">
            <div style="font-weight:700;font-size:14.5px;color:var(--text-strong)">S3 兼容存储</div>
            <div style="font-size:12px;color:var(--faint)">同步数据的唯一后端;支持 AWS S3 / Cloudflare R2 / MinIO</div>
          </div>
          <span class="badge neutral" id="s3-state"><span class="dot"></span>—</span>
        </div>
        <div class="form-grid">
          <div class="field"><label>Endpoint</label><input class="input" id="s3-endpoint" placeholder="https://s3.example.com"></div>
          <div class="field"><label>Region</label><input class="input" id="s3-region" placeholder="us-east-1"></div>
          <div class="field"><label>Bucket</label><input class="input" id="s3-bucket" placeholder="vela-sync"></div>
          <div class="field"><label>路径前缀(可选)</label><input class="input" id="s3-prefix" placeholder="prod/"></div>
          <div class="field"><label>Access Key ID</label><input class="input" id="s3-ak" placeholder="AKIA…"></div>
          <div class="field"><label>Secret Access Key</label><input class="input" id="s3-sk" type="password" placeholder="留空 = 沿用已保存的"></div>
        </div>
        <div class="split">
          <div style="flex:1"><div class="title">Path-style 寻址</div><div class="desc">MinIO / 自建网关需开启;AWS S3 / R2 保持关闭</div></div>
          <button class="switch" id="s3-path" onclick="flip('s3-path')"><span class="thumb"></span></button>
        </div>
        <div class="split">
          <div style="flex:1"><div class="title">STS 限权凭据下发</div><div class="desc">为每个用户签发仅限 <code>users/{uid}/*</code> 前缀的临时凭据(1h);关闭则回退共享凭据</div></div>
          <div class="seg" id="s3-sts">
            <div class="seg-item" data-mode="auto" onclick="setSts('auto')">自动回退</div>
            <div class="seg-item" data-mode="require" onclick="setSts('require')">强制</div>
            <div class="seg-item" data-mode="off" onclick="setSts('off')">关闭</div>
          </div>
        </div>
        <div id="probe" class="probe hidden"></div>
        <div style="display:flex;gap:10px;margin-top:18px">
          <button class="btn" id="s3-test" onclick="testStorage()">测试连接</button>
          <button class="btn primary" onclick="saveStorage()">保存配置</button>
          <span style="flex:1"></span>
          <span class="mono" id="s3-source"></span>
        </div>
      </div>
      <div class="muted-note" style="margin-top:0">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/></svg>
        <span>建议 IAM 权限最小化:<code>GetObject / PutObject / DeleteObject / ListBucket</code> 且限定该桶;更换桶或凭据不影响已有密文,客户端下次同步自动指向新地址。保存即热生效,无需重启。</span>
      </div>
    </div>

    <!-- ===== 功能开关 ===== -->
    <div id="tab-features" class="hidden">
      <div class="card" style="margin-bottom:16px">
        <div class="toolbar" style="border-bottom:none;gap:14px;flex-wrap:wrap">
          <span style="font-size:13px;font-weight:700;color:var(--text-strong)">门禁总闸</span>
          <span class="pill" id="gating-state">—</span>
          <span style="flex:1"></span>
          <span style="font-size:13px;font-weight:700;color:var(--text-strong)">注册策略</span>
          <div class="seg" id="reg-mode">
            <div class="seg-item" data-mode="open" onclick="setRegMode('open')">开放注册</div>
            <div class="seg-item" data-mode="invite" onclick="setRegMode('invite')">仅邀请码</div>
            <div class="seg-item" data-mode="closed" onclick="setRegMode('closed')">关闭注册</div>
          </div>
        </div>
        <div id="gating-hint" style="border-top:1px solid var(--border);padding:10px 16px;font-size:12px;color:var(--muted);line-height:1.7"></div>
        <div id="invite-section" class="hidden" style="border-top:1px solid var(--border);padding:14px 16px">
          （原有邀请码表单与 #invite-list 原样搬进来）
        </div>
      </div>
      <div class="feat-grid">
        <div class="card feat-card">
          <div class="feat-head"><span class="name">登录与注册</span></div>
          <div id="feat-auth"></div>
        </div>
        <div style="display:flex;flex-direction:column;gap:16px">
          <div class="card feat-card">
            <div class="feat-head"><span class="name">云同步</span></div>
            <div id="feat-sync"></div>
          </div>
          <div class="card feat-card">
            <div class="feat-head"><span class="name">AI 功能</span><span style="flex:1"></span><span class="badge" id="ai-badge"><span class="dot"></span>—</span></div>
            <div id="feat-ai"></div>
          </div>
        </div>
      </div>
      <div class="muted-note">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/></svg>
        <span>开关经 <code>/api/features</code> 下发,客户端启动与每 60 s 拉取一次,变更在下一次拉取内生效。「产品开关」受门禁总闸约束;「安全策略」(注册、邮箱验证、强制 2FA、多设备)始终生效并由服务端拦截。</span>
      </div>
    </div>
```

用户表 `<thead>` 在「用户」后加 `<th>状态</th>`;`render()` 每行加:

```js
      <td>
        <div style="display:flex;gap:6px;flex-wrap:wrap">
          ${u.emailVerified ? "" : `<span class="badge warn"><span class="dot"></span>邮箱未验证</span>`}
          <span class="badge ${u.totpEnabled ? "accent" : "neutral"}"><span class="dot"></span>${u.totpEnabled ? "2FA 已绑定" : "无 2FA"}</span>
        </div>
      </td>
```
操作列在「强制下线」前追加 `${u.emailVerified ? "" : `<button class="btn ghost" style="height:28px" onclick="verifyUser('${u.email}')">标记已验证</button>`}${u.totpEnabled ? `<button class="btn ghost" style="height:28px" onclick="resetTotp('${u.email}')">重置 2FA</button>` : ""}`,`colspan` 改 6。

- [ ] **Step 3: JS**

```js
let TAB = localStorage.getItem("kite_admin_tab") || "users";
let STORAGE = null;      // /api/admin/storage 回包
let S3_FORM = { pathStyle: true, stsMode: "auto" };

function switchTab(t) {
  TAB = t; localStorage.setItem("kite_admin_tab", t);
  document.querySelectorAll("#tabs .tab").forEach((el) => (el.dataset.active = String(el.dataset.tab === t)));
  ["users", "storage", "features"].forEach((k) => $("tab-" + k).classList.toggle("hidden", k !== t));
}
function flip(id) { const b = $(id); b.dataset.on = String(b.dataset.on !== "true"); if (id === "s3-path") S3_FORM.pathStyle = b.dataset.on === "true"; }
function setSts(mode) { S3_FORM.stsMode = mode; document.querySelectorAll("#s3-sts .seg-item").forEach((el) => (el.dataset.active = String(el.dataset.mode === mode))); }

function renderStorage() {
  const s = STORAGE;
  $("s3-endpoint").value = s.endpoint; $("s3-region").value = s.region; $("s3-bucket").value = s.bucket;
  $("s3-prefix").value = s.prefix; $("s3-ak").value = s.accessKey; $("s3-sk").value = "";
  $("s3-sk").placeholder = s.hasSecret ? "留空 = 沿用已保存的" : "必填";
  S3_FORM = { pathStyle: !!s.pathStyle, stsMode: s.stsMode || "auto" };
  $("s3-path").dataset.on = String(S3_FORM.pathStyle); setSts(S3_FORM.stsMode);
  $("s3-state").className = "badge " + (s.configured ? "ok" : "warn");
  $("s3-state").innerHTML = '<span class="dot"></span>' + (s.configured ? "已配置" : "未配置");
  $("s3-source").textContent = s.source === "db" ? "来源:控制台保存" : "来源:环境变量初值(尚未在控制台保存)";
  $("probe").classList.add("hidden");
}
function storageBody() {
  return { endpoint: $("s3-endpoint").value.trim(), region: $("s3-region").value.trim(), bucket: $("s3-bucket").value.trim(),
    prefix: $("s3-prefix").value.trim(), accessKey: $("s3-ak").value.trim(), secretKey: $("s3-sk").value,
    pathStyle: S3_FORM.pathStyle, stsMode: S3_FORM.stsMode };
}
async function testStorage() {
  const btn = $("s3-test"); btn.disabled = true; btn.textContent = "测试中…";
  try {
    const r = await api("/api/admin/storage/test", { method: "POST", body: JSON.stringify(storageBody()) });
    const line = (name, v) => `<div class="${v ? "bad" : "ok"}">${v ? "✗" : "✓"} ${name}${v ? ":" + esc(v) : ""}</div>`;
    $("probe").innerHTML = line("ListBucket", r.list) + line("PutObject", r.put) + line("DeleteObject", r.delete)
      + (S3_FORM.stsMode !== "off" ? line("STS AssumeRole", r.sts) : "") + `<div>耗时 ${r.latencyMs} ms</div>`;
    $("probe").classList.remove("hidden");
    toast(r.ok ? "连接成功:读写与 ListBucket 权限正常 · " + r.latencyMs + "ms" : "连接测试未通过,见下方明细", !r.ok, r.ok ? "check" : "alert-circle");
  } catch (e) { toast("测试失败:" + e.message, true); }
  finally { btn.disabled = false; btn.textContent = "测试连接"; }
}
async function saveStorage() {
  try {
    STORAGE = await api("/api/admin/storage", { method: "POST", body: JSON.stringify(storageBody()) });
    renderStorage(); toast("存储配置已保存并热生效", false, "check"); refresh();
  } catch (e) { toast("保存失败:" + e.message, true); }
}

// ---- 功能开关卡片(数据驱动) ----
const FEATURE_CARDS = {
  "feat-auth": [
    ["featureLogin", "登录", "关闭后已有账户无法登录,注册不受影响", true],
    ["featureEmailVerify", "邮箱验证", "注册后须验证邮箱才能开始同步(未配 SMTP 时验证码打在服务端日志)", false],
    ["featureTotpRequired", "双因素认证(强制)", "所有账户登录必须绑定 TOTP,未绑定者不能同步", false],
    ["featureMultiDevice", "多设备登录", "关闭后同一账户仅允许一台设备在线,新登录挤掉旧令牌", false],
  ],
  "feat-sync": [
    ["featureSync", "云同步(全局)", "关闭后所有客户端同步暂停,数据保留", true],
    ["defaultSyncEnabled", "新用户默认放开同步", "关闭则新注册账户需管理员手动放开", false],
    ["featureKeySync", "允许密钥同步", "关闭后客户端密钥项强制「仅本地」", true],
    ["featureRecordingSync", "允许会话录制同步", "关闭后录制文件不再上传", true],
  ],
  "feat-ai": [
    ["featureAi", "启用 AI 功能(总开关)", "关闭后客户端隐藏全部 AI 入口,子项失效", true],
    ["featureAiCommand", "命令补全与解释", "终端内白话转命令、⌘⇧A 命令栏", true],
    ["featureAiDiagnose", "错误诊断", "命令失败时分析输出给出修复建议(⌘I)", true],
    ["featureAiSummary", "输出摘要", "长输出一键生成摘要(⌘⇧S)", true],
  ],
};

function renderFeatures() {
  const gating = !!SETTINGS.featureGating;
  for (const [box, rows] of Object.entries(FEATURE_CARDS)) {
    $(box).innerHTML = rows.map(([key, label, desc, gated]) => {
      const dim = gated && !gating;
      const sub = box === "feat-ai" && key !== "featureAi" && !SETTINGS.featureAi;
      return `<div class="feat-row" data-dim="${dim || sub}">
        <div style="flex:1;line-height:1.4"><div style="font-weight:600;font-size:13px;color:var(--text-strong)">${label}</div>
        <div style="font-size:11.5px;color:var(--faint)">${desc}${gated ? "" : " · 安全策略,不受总闸影响"}</div></div>
        <button class="switch" data-on="${!!SETTINGS[key]}" onclick="toggleSetting('${key}','${label}')"><span class="thumb"></span></button>
      </div>`;
    }).join("");
  }
  $("ai-badge").className = "badge " + (SETTINGS.featureAi ? "ok" : "neutral");
  $("ai-badge").innerHTML = '<span class="dot"></span>' + (SETTINGS.featureAi ? "已启用" : "已关闭");
}

async function toggleSetting(key, label) {
  try {
    SETTINGS = await api("/api/admin/settings", { method: "POST", body: JSON.stringify({ [key]: !SETTINGS[key] }) });
    renderSettings(); toast(label + ":" + (SETTINGS[key] ? "已开启" : "已关闭"), false, SETTINGS[key] ? "check" : "ban");
  } catch (e) { toast("设置失败:" + e.message, true); }
}
async function verifyUser(email) {
  try { await api(`/api/admin/users/${encodeURIComponent(email)}/verify`, { method: "POST" }); toast("已标记 " + email + " 邮箱为已验证"); refresh(); }
  catch (e) { toast("操作失败:" + e.message, true); }
}
async function resetTotp(email) {
  if (!confirm(`重置 ${email} 的双因素认证?其全部登录令牌将被吊销,下次登录重新绑定。`)) return;
  try { await api(`/api/admin/users/${encodeURIComponent(email)}/2fa/reset`, { method: "POST" }); toast("已重置 " + email + " 的 2FA", false, "log-out"); refresh(); }
  catch (e) { toast("操作失败:" + e.message, true); }
}
```

`renderSettings()` 改为:注册策略 seg + `gating-state` + `gating-hint` + `invite-section` 逻辑保留,删除 `feat-login/feat-sync/feat-ai/default-sync` 四行(旧元素已不存在),末尾调用 `renderFeatures()`。`refresh()` 的 `Promise.all` 增加 `api("/api/admin/storage")` → `STORAGE = storage; renderStorage();`,并在末尾 `switchTab(TAB)`。删除旧的 `toggleFeature()`/`toggleDefaultSync()`(被 `toggleSetting` 取代;`defaultSyncEnabled` 走同一接口)。

- [ ] **Step 4: 浏览器验证**

起实例(`KITE_FEATURE_GATING=on`),Chrome 打开 `http://127.0.0.1:8799/admin`,登录后逐项检查:
- 三个页签可切换,刷新页面后停在上次页签;
- S3 页显示 env 初值、「未在控制台保存」;改 Bucket 点「测试连接」看到 ✗ 明细;点「保存配置」后来源变为「控制台保存」,用户页统计卡 bucket 同步更新;
- 功能页 11 个开关 + 新用户默认同步都能切换并 toast;总闸关时产品开关变暗且说明文案正确;AI 总开关关掉后三个子项变暗;
- 用户页新列显示「邮箱未验证 / 2FA 已绑定」,对应操作按钮可用;
- 亮/暗两套主题无硬编码色残留(`grep -c '#[0-9a-f]\{6\}' admin.html` 只应命中 token 定义区)。

- [ ] **Step 5: Commit(需用户允许)**

```bash
git add server/src/admin.html
git commit -m "feat(console): 按原型重排为三页签 —— 用户与同步 / S3 存储 / 功能开关"
```

---

### Task 6: 客户端功能开关扩字段 + 60 s 轮询 + 设置页展示

**Files:**
- Modify: `desktop/src-tauri/src/features.rs`(`Features` 加字段)
- Modify: `desktop/src/types.ts:168-180`(`Features`)
- Modify: `desktop/src/stores/features.ts`
- Modify: `desktop/src/screens/SettingsScreen.tsx:66-71`(badges 列表)

**Interfaces:**
- Produces:`Features` 新字段 `keySync, recordingSync, aiCommand, aiDiagnose, aiSummary, emailVerify, totpRequired, multiDevice`(TS 与 Rust 同名 camelCase);`useFeature(name)` 接受全部布尔名;`useFeaturesStore.getState().features`。

- [ ] **Step 1: Rust 结构体**

在 `Features` 里 `pub ai: bool,` 之后加:

```rust
    #[serde(default = "default_true")] pub key_sync: bool,
    #[serde(default = "default_true")] pub recording_sync: bool,
    #[serde(default = "default_true")] pub ai_command: bool,
    #[serde(default = "default_true")] pub ai_diagnose: bool,
    #[serde(default = "default_true")] pub ai_summary: bool,
    /// 安全策略:由服务端拦截,客户端只用来提示
    #[serde(default)] pub email_verify: bool,
    #[serde(default)] pub totp_required: bool,
    #[serde(default = "default_true")] pub multi_device: bool,
```
加 `fn default_true() -> bool { true }`;`Features::unknown()` 里对应赋值(前五项与 multi_device 为 true,email_verify/totp_required 为 false)。

- [ ] **Step 2: TS 类型与 store**

`types.ts` 的 `Features` 在 `ai: boolean;` 后加同名八个字段(带一行注释"平台下发,安全策略仅提示")。`stores/features.ts`:

```ts
const UNKNOWN: Features = {
  registration: "open", enforced: false, login: true, sync: true, ai: true,
  keySync: true, recordingSync: true, aiCommand: true, aiDiagnose: true, aiSummary: true,
  emailVerify: false, totpRequired: false, multiDevice: true,
  passwordMinLength: 8, passwordRule: "", known: false,
};

/** 原型约定:变更在客户端下一次心跳(≤60 s)内生效 */
const POLL_MS = 60_000;
let pollTimer: ReturnType<typeof setInterval> | null = null;

// load 末尾(refresh 之后):
    if (!pollTimer) pollTimer = setInterval(() => void useFeaturesStore.getState().refresh(), POLL_MS);

export type FeatureName =
  | "login" | "sync" | "ai" | "keySync" | "recordingSync"
  | "aiCommand" | "aiDiagnose" | "aiSummary" | "emailVerify" | "totpRequired" | "multiDevice";

export function useFeature(name: FeatureName): boolean {
  return useFeaturesStore((s) => s.features[name]);
}

/** AI 子功能:总开关关了子项一律关 */
export function useAiFeature(name: "aiCommand" | "aiDiagnose" | "aiSummary"): boolean {
  return useFeaturesStore((s) => s.features.ai && s.features[name]);
}
```

- [ ] **Step 3: 设置页 badges**

`SettingsScreen.tsx` 的 `items` 改为分组:

```tsx
  const groups: { title: string; items: { label: string; on: boolean }[] }[] = [
    { title: "账户", items: [
      { label: "注册", on: features.registration !== "closed" },
      { label: "登录", on: features.login },
      { label: "邮箱验证", on: features.emailVerify },
      { label: "强制 2FA", on: features.totpRequired },
      { label: "多设备登录", on: features.multiDevice },
    ]},
    { title: "同步", items: [
      { label: "云同步", on: features.sync },
      { label: "密钥同步", on: features.keySync },
      { label: "录制同步", on: features.recordingSync },
    ]},
    { title: "AI", items: [
      { label: "AI 助手", on: features.ai },
      { label: "命令生成", on: features.ai && features.aiCommand },
      { label: "错误诊断", on: features.ai && features.aiDiagnose },
      { label: "输出摘要", on: features.ai && features.aiSummary },
    ]},
  ];
```
渲染改为每组一行 `settings-row`(左侧组名,右侧 badges),文案里的"四项功能"改为"各项功能"。

- [ ] **Step 4: 编译检查**

Run: `cd desktop && pnpm exec tsc -b && cd src-tauri && cargo check 2>&1 | tail -2`
Expected: 无错误。

- [ ] **Step 5: Commit(需用户允许)**

```bash
git add desktop/src-tauri/src/features.rs desktop/src/types.ts desktop/src/stores/features.ts desktop/src/screens/SettingsScreen.tsx
git commit -m "feat(desktop): 功能开关扩到 11 项并每 60s 轮询"
```

---

### Task 7: 客户端账户层 —— 邮箱验证 / TOTP / 账户刷新(Rust + IPC)

**Files:**
- Modify: `desktop/src-tauri/src/account.rs`
- Modify: `desktop/src-tauri/src/commands.rs:388-460`、`desktop/src-tauri/src/lib.rs`(注册命令)
- Modify: `desktop/src/types.ts:182-187`(`AccountInfo`)、`desktop/src/ipc/api.ts`、`desktop/src/ipc/tauri.ts`、`desktop/src/stores/sync.ts`

**Interfaces:**
- Produces(Rust):`AccountInfo { id, email, sync_enabled, email_verified: bool, totp_enabled: bool, totp_setup_required: bool }`;`login(db, vault, email, password, totp: Option<&str>)`;`verify_email(db, vault, code)`、`resend_verification(db, vault)`、`totp_setup(db, vault) -> TotpSetup { secret, otpauth_url }`、`totp_enable(db, vault, code)`、`totp_disable(db, vault, code)`、`refresh(db, vault) -> AccountInfo`(GET /api/me,401 时清本地登录态并返回 `LOGIN_REQUIRED`)。
- Produces(TS `KiteIpc`):`accountLogin(email, password, totp?)`、`accountVerifyEmail(code)`、`accountResendVerification()`、`accountTotpSetup()`、`accountTotpEnable(code)`、`accountTotpDisable(code)`、`accountRefresh()`;`useSyncStore` 新增同名动作 `verifyEmail/resendVerification/totpSetup/totpEnable/totpDisable/refreshAccount`,`login(email, password, totp?)`。
- 错误串形态不变:`"TOTP_REQUIRED: 请输入动态验证码(HTTP 401)"`。

- [ ] **Step 1: account.rs**

`AccountInfo` 加:
```rust
    #[serde(default = "default_true")] pub email_verified: bool,
    #[serde(default)] pub totp_enabled: bool,
    /// 平台强制 2FA 且本账户尚未绑定 —— 同步会被拒,界面要引导绑定
    #[serde(default)] pub totp_setup_required: bool,
```
抽出解析函数(注册与登录共用):
```rust
fn info_from(body: &serde_json::Value) -> AccountInfo {
    let u = &body["user"];
    AccountInfo {
        id: u["id"].as_str().unwrap_or_default().to_string(),
        email: u["email"].as_str().unwrap_or_default().to_string(),
        sync_enabled: u["syncEnabled"].as_bool().unwrap_or(false),
        email_verified: u["emailVerified"].as_bool().unwrap_or(true),
        totp_enabled: u["totpEnabled"].as_bool().unwrap_or(false),
        totp_setup_required: body["totpSetupRequired"].as_bool().unwrap_or(false),
    }
}
fn store_login(db: &Db, vault: &Vault, token: &str, info: &AccountInfo) -> Result<()> {
    db.kv_set(KV_TOKEN, &base64::engine::general_purpose::STANDARD.encode(vault.encrypt(token.as_bytes())?))?;
    db.kv_set(KV_ACCOUNT, &serde_json::to_string(info)?)
}
```
`register`/`login` 改用它们;`login` 签名加 `totp: Option<&str>`,请求体 `"totp": totp.unwrap_or_default()`。

新增(统一的带鉴权 POST 助手):
```rust
async fn authed_post(db: &Db, vault: &Vault, path: &str, body: serde_json::Value) -> Result<serde_json::Value> {
    let t = token(db, vault)?;
    let resp = reqwest::Client::new()
        .post(format!("{}{path}", platform_url(db)))
        .bearer_auth(t).json(&body).send().await
        .map_err(|e| AppError(format!("无法连接管理平台: {e}")))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError("LOGIN_REQUIRED: 登录已失效,请重新登录".into()));
    }
    if !resp.status().is_success() { return Err(api_error(resp).await); }
    resp.json().await.map_err(|e| AppError(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpSetup { pub secret: String, pub otpauth_url: String }

pub async fn verify_email(db: &Db, vault: &Vault, code: &str) -> Result<AccountInfo> {
    authed_post(db, vault, "/api/verify-email", json!({ "code": code })).await?;
    refresh(db, vault).await
}
pub async fn resend_verification(db: &Db, vault: &Vault) -> Result<()> {
    authed_post(db, vault, "/api/verify-email/resend", json!({})).await.map(|_| ())
}
pub async fn totp_setup(db: &Db, vault: &Vault) -> Result<TotpSetup> {
    let v = authed_post(db, vault, "/api/2fa/setup", json!({})).await?;
    serde_json::from_value(v).map_err(|e| AppError(e.to_string()))
}
pub async fn totp_enable(db: &Db, vault: &Vault, code: &str) -> Result<AccountInfo> {
    authed_post(db, vault, "/api/2fa/enable", json!({ "code": code })).await?;
    refresh(db, vault).await
}
pub async fn totp_disable(db: &Db, vault: &Vault, code: &str) -> Result<AccountInfo> {
    authed_post(db, vault, "/api/2fa/disable", json!({ "code": code })).await?;
    refresh(db, vault).await
}

/// GET /api/me 刷新账户状态缓存;令牌失效则清掉本地登录态
pub async fn refresh(db: &Db, vault: &Vault) -> Result<AccountInfo> {
    let t = token(db, vault)?;
    let resp = reqwest::Client::new()
        .get(format!("{}/api/me", platform_url(db))).bearer_auth(t).send().await
        .map_err(|e| AppError(format!("无法连接管理平台: {e}")))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        let conn = db.conn.lock().unwrap();
        conn.execute("DELETE FROM kv WHERE key IN (?1, ?2)", [KV_TOKEN, KV_ACCOUNT])?;
        return Err(AppError("LOGIN_REQUIRED: 登录已失效(可能在其它设备登录或被管理员下线),请重新登录".into()));
    }
    if !resp.status().is_success() { return Err(api_error(resp).await); }
    let v: serde_json::Value = resp.json().await.map_err(|e| AppError(e.to_string()))?;
    let info = AccountInfo {
        id: v["id"].as_str().unwrap_or_default().to_string(),
        email: v["email"].as_str().unwrap_or_default().to_string(),
        sync_enabled: v["syncEnabled"].as_bool().unwrap_or(false),
        email_verified: v["emailVerified"].as_bool().unwrap_or(true),
        totp_enabled: v["totpEnabled"].as_bool().unwrap_or(false),
        totp_setup_required: v["requirements"]["totpRequired"].as_bool().unwrap_or(false)
            && !v["totpEnabled"].as_bool().unwrap_or(false),
    };
    db.kv_set(KV_ACCOUNT, &serde_json::to_string(&info)?)?;
    Ok(info)
}
```
`sync_credentials` 里"同步账户状态缓存"那段保持,只把 `info.sync_enabled = true` 保留。

- [ ] **Step 2: 命令与注册**

`commands.rs`:`account_login` 加参数 `totp: Option<String>` 透传;新增:
```rust
#[tauri::command] pub async fn account_verify_email(state: State<'_, AppState>, code: String) -> Result<crate::account::AccountInfo> { crate::account::verify_email(&state.db, &state.vault, &code).await }
#[tauri::command] pub async fn account_resend_verification(state: State<'_, AppState>) -> Result<()> { crate::account::resend_verification(&state.db, &state.vault).await }
#[tauri::command] pub async fn account_totp_setup(state: State<'_, AppState>) -> Result<crate::account::TotpSetup> { crate::account::totp_setup(&state.db, &state.vault).await }
#[tauri::command] pub async fn account_totp_enable(state: State<'_, AppState>, code: String) -> Result<crate::account::AccountInfo> { crate::account::totp_enable(&state.db, &state.vault, &code).await }
#[tauri::command] pub async fn account_totp_disable(state: State<'_, AppState>, code: String) -> Result<crate::account::AccountInfo> { crate::account::totp_disable(&state.db, &state.vault, &code).await }
#[tauri::command] pub async fn account_refresh(state: State<'_, AppState>) -> Result<crate::account::AccountInfo> { crate::account::refresh(&state.db, &state.vault).await }
```
`lib.rs` 的 `generate_handler!` 追加这六个。

- [ ] **Step 3: TS 契约与 store**

`types.ts` `AccountInfo` 加 `emailVerified: boolean; totpEnabled: boolean; totpSetupRequired: boolean;` 与 `export interface TotpSetup { secret: string; otpauthUrl: string }`。`api.ts`/`tauri.ts` 加上述七个方法(`accountLogin: (email, password, totp) => invoke("account_login", { email, password, totp })` 等)。`stores/sync.ts`:

```ts
  login: async (email, password, totp) => {
    const account = await ipc.accountLogin(email, password, totp);
    set({ account, status: await ipc.getSyncStatus() });
  },
  refreshAccount: async () => {
    try { set({ account: await ipc.accountRefresh() }); }
    catch (e) { if (String(e).includes("LOGIN_REQUIRED")) set({ account: null }); }
  },
  verifyEmail: async (code) => set({ account: await ipc.accountVerifyEmail(code) }),
  resendVerification: () => ipc.accountResendVerification(),
  totpSetup: () => ipc.accountTotpSetup(),
  totpEnable: async (code) => set({ account: await ipc.accountTotpEnable(code) }),
  totpDisable: async (code) => set({ account: await ipc.accountTotpDisable(code) }),
```
`load()` 末尾:`if (account) void get().refreshAccount();`(启动时核对一次,拿到多设备被挤下线/管理员改动的最新状态)。

- [ ] **Step 4: 编译检查**

Run: `cd desktop && pnpm exec tsc -b && cd src-tauri && cargo check 2>&1 | tail -2` → 无错误。

- [ ] **Step 5: Commit(需用户允许)**

```bash
git add desktop/src-tauri/src/account.rs desktop/src-tauri/src/commands.rs desktop/src-tauri/src/lib.rs desktop/src/types.ts desktop/src/ipc desktop/src/stores/sync.ts
git commit -m "feat(desktop): 账户层接入邮箱验证、TOTP 与状态刷新"
```

---

### Task 8: 账户界面 —— 登录时 TOTP、邮箱验证卡、2FA 绑定/解绑卡(含二维码)

**Files:**
- Modify: `desktop/package.json`(加 `qrcode` + `@types/qrcode`)
- Modify: `desktop/src/screens/sync/AccountCard.tsx`
- Modify: `desktop/src/screens/sync/AccountSummary.tsx`
- Create: `desktop/src/screens/sync/TotpCard.tsx`
- Create: `desktop/src/screens/sync/EmailVerifyCard.tsx`

**Interfaces:**
- Consumes:Task 7 的 store 动作与 `AccountInfo` 字段;Task 6 的 `features.totpRequired / emailVerify`。

- [ ] **Step 1: 装依赖**

Run: `cd desktop && pnpm add qrcode && pnpm add -D @types/qrcode`

- [ ] **Step 2: AccountCard 登录支持动态码**

新增状态 `const [totp, setTotp] = useState(""); const [needTotp, setNeedTotp] = useState(false);`。登录调用改为:
```tsx
const doLogin = () =>
  run("login", async () => {
    try {
      await login(email.trim(), password, needTotp ? totp.trim() : undefined);
    } catch (e) {
      if (String(e).includes("TOTP_REQUIRED")) { setNeedTotp(true); setError("该账户已绑定双因素认证,请输入验证器里的 6 位动态码"); return; }
      throw e;
    }
  });
```
表单在密码之后、仅 `!isRegister && needTotp` 时渲染:
```tsx
<TextInput label="动态验证码" placeholder="6 位数字" value={totp} inputMode="numeric" maxLength={6}
  onChange={(e) => setTotp(e.target.value.replace(/\D/g, ""))}
  onKeyDown={(e) => e.key === "Enter" && totp.length === 6 && void doLogin()} />
```
登录按钮 `disabled` 追加 `|| (needTotp && totp.length !== 6)`,`onClick={() => void doLogin()}`;两处原 `run("login", () => login(...))` 都改为 `doLogin()`。

- [ ] **Step 3: EmailVerifyCard**

```tsx
import { useState } from "react";
import { MailCheck } from "lucide-react";
import { Button, TextInput } from "@/components/ui";
import { useSyncStore } from "@/stores/sync";

/** 平台开启邮箱验证且本账户未验证:同步会被拒(EMAIL_UNVERIFIED),在这里输码 */
export function EmailVerifyCard() {
  const { verifyEmail, resendVerification } = useSyncStore();
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState<"verify" | "resend" | null>(null);
  const [msg, setMsg] = useState<string | null>(null);

  const act = async (kind: "verify" | "resend", fn: () => Promise<unknown>, ok: string) => {
    setBusy(kind); setMsg(null);
    try { await fn(); setMsg(ok); }
    catch (e) { setMsg(String(e).replace(/^(?:Error:\s*)?[A-Z_]+:\s*/, "").replace(/\(HTTP \d+\)\s*$/, "")); }
    finally { setBusy(null); }
  };

  return (
    <div className="card pad-card mb-16">
      <div className="row gap-8 mb-4">
        <MailCheck size={15} />
        <span className="fs-md w-bold t-strong">验证邮箱</span>
      </div>
      <div className="fs-sm t-muted lh-relaxed mb-12">
        平台要求验证邮箱后才能开始同步。验证码已发送到你的邮箱(10 分钟内有效);若平台未配置邮件服务,请联系管理员获取或直接标记。
      </div>
      <div className="row gap-8">
        <TextInput placeholder="6 位验证码" value={code} inputMode="numeric" maxLength={6}
          onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))} style={{ width: 160 }} />
        <Button variant="primary" loading={busy === "verify"} disabled={code.length !== 6 || busy !== null}
          onClick={() => void act("verify", () => verifyEmail(code), "邮箱已验证")}>验证</Button>
        <Button variant="ghost" loading={busy === "resend"} disabled={busy !== null}
          onClick={() => void act("resend", resendVerification, "已重新发送")}>重发验证码</Button>
      </div>
      {msg && <div className="fs-sm t-muted mt-8">{msg}</div>}
    </div>
  );
}
```

- [ ] **Step 4: TotpCard**

```tsx
import { useEffect, useState } from "react";
import { ShieldCheck } from "lucide-react";
import QRCode from "qrcode";
import { Badge, Button, TextInput } from "@/components/ui";
import { useFeaturesStore } from "@/stores/features";
import { useSyncStore } from "@/stores/sync";
import type { AccountInfo, TotpSetup } from "@/types";

/** 双因素认证:未绑定 → 生成密钥/二维码 → 输码确认;已绑定 → 输码解绑(平台强制时不可解绑) */
export function TotpCard({ account }: { account: AccountInfo }) {
  const { totpSetup, totpEnable, totpDisable } = useSyncStore();
  const enforced = useFeaturesStore((s) => s.features.totpRequired);
  const [setup, setSetup] = useState<TotpSetup | null>(null);
  const [qr, setQr] = useState<string>("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  useEffect(() => {
    if (!setup) { setQr(""); return; }
    QRCode.toDataURL(setup.otpauthUrl, { margin: 1, width: 160 }).then(setQr).catch(() => setQr(""));
  }, [setup]);

  const run = async (fn: () => Promise<unknown>, ok: string) => {
    setBusy(true); setMsg(null);
    try { await fn(); setMsg(ok); setCode(""); }
    catch (e) { setMsg(String(e).replace(/^(?:Error:\s*)?[A-Z_]+:\s*/, "").replace(/\(HTTP \d+\)\s*$/, "")); }
    finally { setBusy(false); }
  };

  return (
    <div className="card pad-card mb-16">
      <div className="row gap-8 mb-4">
        <ShieldCheck size={15} />
        <span className="fs-md w-bold t-strong">双因素认证</span>
        <span className="spacer" />
        <Badge color={account.totpEnabled ? "success" : enforced ? "danger" : "neutral"} dot>
          {account.totpEnabled ? "已绑定" : enforced ? "平台强制,尚未绑定" : "未绑定"}
        </Badge>
      </div>
      {!account.totpEnabled && enforced && (
        <div className="alert alert-warning mb-12">平台要求所有账户绑定双因素认证,绑定前无法同步。</div>
      )}

      {account.totpEnabled ? (
        <div className="row gap-8">
          <span className="fs-sm t-muted grow">登录时需要验证器里的 6 位动态码。{enforced ? "平台强制开启,不能解绑。" : ""}</span>
          {!enforced && (
            <>
              <TextInput placeholder="动态码" value={code} inputMode="numeric" maxLength={6}
                onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))} style={{ width: 120 }} />
              <Button variant="ghost" loading={busy} disabled={code.length !== 6}
                onClick={() => void run(() => totpDisable(code), "已解绑")}>解绑</Button>
            </>
          )}
        </div>
      ) : !setup ? (
        <Button variant="primary" loading={busy}
          onClick={() => void run(async () => setSetup(await totpSetup()), "用验证器扫码,然后输入动态码确认")}>
          开始绑定
        </Button>
      ) : (
        <div className="row gap-16" style={{ alignItems: "flex-start" }}>
          {qr && <img src={qr} alt="otpauth 二维码" width={160} height={160} className="qr" />}
          <div className="col gap-8 grow">
            <div className="fs-sm t-muted lh-relaxed">
              用 Google Authenticator / 1Password 等扫码,或手动输入密钥:
              <div className="mono fs-sm t-strong" style={{ wordBreak: "break-all" }}>{setup.secret}</div>
            </div>
            <div className="row gap-8">
              <TextInput placeholder="6 位动态码" value={code} inputMode="numeric" maxLength={6}
                onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))} style={{ width: 140 }} />
              <Button variant="primary" loading={busy} disabled={code.length !== 6}
                onClick={() => void run(async () => { await totpEnable(code); setSetup(null); }, "已绑定双因素认证")}>确认绑定</Button>
              <Button variant="ghost" onClick={() => setSetup(null)}>取消</Button>
            </div>
          </div>
        </div>
      )}
      {msg && <div className="fs-sm t-muted mt-8">{msg}</div>}
    </div>
  );
}
```
`utilities.css` 加 `.qr{border-radius:var(--radius-md);background:#fff;padding:4px}`(二维码底色必须是白,这是唯一允许的固定色,注释说明)。

- [ ] **Step 5: AccountSummary 组装**

`AccountSummary` 返回改为 Fragment:主卡 + `{emailVerify && !account.emailVerified && <EmailVerifyCard />}` + `<TotpCard account={account} />`。主卡 badge 区加 `{!account.emailVerified && <Badge color="warning" dot>邮箱未验证</Badge>}`;`emailVerify` 取自 `useFeaturesStore((s) => s.features.emailVerify)`。

- [ ] **Step 6: 编译与手工走查**

Run: `cd desktop && pnpm exec tsc -b`。
`pnpm tauri dev` 连本地平台:开邮箱验证注册 → 出现验证卡 → 输日志里的码 → 卡消失;开始绑定 → 扫码 → 确认 → badge 变「已绑定」;退出再登录 → 出现动态码输入。

- [ ] **Step 7: Commit(需用户允许)**

```bash
git add desktop/package.json desktop/pnpm-lock.yaml desktop/src/screens/sync desktop/src/theme/utilities.css
git commit -m "feat(desktop): 登录动态码、邮箱验证卡、2FA 绑定/解绑卡(二维码)"
```

---

### Task 9: 密钥同步开关 + 会话录制同步(Rust 同步层 + 界面)

**Files:**
- Modify: `desktop/src-tauri/src/models.rs:177-182`(`SyncScopes` 加 `recordings`)
- Modify: `desktop/src-tauri/src/db.rs`(新增 `set_recording_sync`、`recording_exists`、`insert_recording_with_id`、`list_recordings` 返回 path)
- Modify: `desktop/src-tauri/src/sync.rs`(`scope_allows` 看开关;新增 `sync_recordings`)
- Modify: `desktop/src-tauri/src/commands.rs`(`set_key_sync` 门禁;新增 `set_recording_sync`)、`lib.rs`
- Modify: `desktop/src/types.ts`(`SyncConfig.scopes` 加 `recordings`)、`ipc/api.ts`、`ipc/tauri.ts`、`stores/data.ts`
- Modify: `desktop/src/screens/sync/ScopeCard.tsx`、`desktop/src/screens/terminal/RightDock.tsx`、`desktop/src/screens/KeysScreen.tsx`

**Interfaces:**
- Produces:`ipc.setRecordingSync(id, enabled)`;`useDataStore().setRecordingSync`;桶内对象 `{prefix}/recordings/{id}.cast`(主密钥加密,AAD=对象键)与 `{prefix}/recordings/{id}.meta`(加密 JSON `{id, sessionHost, name, durationMs, startedAt}`);错误码 `KEY_SYNC_DISABLED`、`RECORDING_SYNC_DISABLED`。

- [ ] **Step 1: 模型与 DB**

`SyncScopes` 加 `#[serde(default)] pub recordings: bool`(默认 false:录制大,用户显式开)。`get_config` 的默认值里加 `recordings: false`。

`db.rs` 新增:
```rust
    pub fn set_recording_sync(&self, id: &str, enabled: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE recordings SET sync_enabled = ?2 WHERE id = ?1", params![id, enabled as i64])?;
        Ok(())
    }
    pub fn recording_exists(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row("SELECT 1 FROM recordings WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
    }
    /// 从云端拉下来的录制:沿用远端 id,标记为已同步
    pub fn insert_recording_with_id(&self, id: &str, session_host: &str, name: &str, path: &str, duration_ms: u64, started_at: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO recordings (id, session_host, name, path, duration_ms, started_at, sync_enabled)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
            params![id, session_host, name, path, duration_ms as i64, started_at],
        )?;
        Ok(())
    }
    /// (id, path) —— 同步上传用
    pub fn recording_paths(&self) -> Result<Vec<(String, String, bool)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, path, sync_enabled FROM recordings")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
```

- [ ] **Step 2: sync.rs**

`scope_allows` 改签名 `fn scope_allows(cfg: &SyncConfig, flags: &crate::features::Features, entity: &str) -> bool`,`"key" => cfg.scopes.keys && flags.key_sync`;`sync_once` 开头 `let flags = crate::features::cached(db);`,所有调用点传 `&flags`(`build_snapshot`/`pull_snapshot`/`maybe_compact` 也补参数)。

在 `maybe_compact` 之前插入 `sync_recordings(db, &bucket, &cipher, &cfg, &flags, &prefix).await?;`:

```rust
/// 录制文件同步:与 oplog 无关的独立推拉。
/// 推:本地 sync_enabled 且未上传过(kv rec_pushed_{id})的 .cast + .meta;
/// 拉:桶里有 .meta 而本地没有该 id 的,下载 .cast 到 data_dir/recordings/ 并入库。
async fn sync_recordings(
    db: &Db, bucket: &Bucket, cipher: &XChaCha20Poly1305, cfg: &SyncConfig,
    flags: &crate::features::Features, prefix: &str,
) -> Result<()> {
    if !cfg.scopes.recordings || !flags.recording_sync {
        return Ok(());
    }
    let dir = db.data_dir.join("recordings");
    std::fs::create_dir_all(&dir)?;
    let metas: std::collections::HashMap<String, (String, String, u64, String)> = db
        .list_recordings()?
        .into_iter()
        .map(|r| (r.id, (r.session_host, r.name, r.duration_ms, r.started_at)))
        .collect();
    for (id, path, on) in db.recording_paths()? {
        let mark = format!("rec_pushed_{id}");
        if !on || db.kv_get(&mark)?.is_some() { continue; }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let cast_key = format!("{prefix}/recordings/{id}.cast");
        let meta_key = format!("{prefix}/recordings/{id}.meta");
        let (host, name, dur, started) = metas.get(&id).cloned().unwrap_or_default();
        let meta = serde_json::json!({ "id": id, "sessionHost": host, "name": name, "durationMs": dur, "startedAt": started });
        bucket.put_object(&cast_key, &seal(cipher, &bytes, &cast_key)?).await
            .map_err(|e| AppError(format!("上传录制 {id} 失败: {e}")))?;
        bucket.put_object(&meta_key, &seal(cipher, &serde_json::to_vec(&meta)?, &meta_key)?).await
            .map_err(|e| AppError(format!("上传录制元数据 {id} 失败: {e}")))?;
        db.kv_set(&mark, "1")?;
    }
    let list_prefix = format!("{prefix}/recordings/");
    let listing = bucket.list(list_prefix.clone(), None).await
        .map_err(|e| AppError(format!("列举录制失败: {e}")))?;
    for page in listing {
        for obj in page.contents {
            let Some(id) = obj.key.strip_prefix(&list_prefix).and_then(|k| k.strip_suffix(".meta")) else { continue };
            if db.recording_exists(id)? { continue; }
            let meta_resp = bucket.get_object(&obj.key).await.map_err(|e| AppError(e.to_string()))?;
            let Ok(meta) = unseal(cipher, meta_resp.bytes(), &obj.key)
                .and_then(|p| serde_json::from_slice::<serde_json::Value>(&p).map_err(Into::into)) else { continue };
            let cast_key = format!("{list_prefix}{id}.cast");
            let cast_resp = bucket.get_object(&cast_key).await.map_err(|e| AppError(e.to_string()))?;
            let Ok(plain) = unseal(cipher, cast_resp.bytes(), &cast_key) else { continue };
            let local = dir.join(format!("{id}.cast"));
            std::fs::write(&local, plain)?;
            db.insert_recording_with_id(
                id,
                meta["sessionHost"].as_str().unwrap_or("远端"),
                meta["name"].as_str().unwrap_or("云端录制"),
                &local.to_string_lossy(),
                meta["durationMs"].as_u64().unwrap_or(0),
                meta["startedAt"].as_str().unwrap_or(""),
            )?;
            db.kv_set(&format!("rec_pushed_{id}"), "1")?;
        }
    }
    Ok(())
}
```

- [ ] **Step 3: 命令**

`set_key_sync` 开头:
```rust
    if enabled && !crate::features::cached(&state.db).key_sync {
        return Err(AppError("KEY_SYNC_DISABLED: 平台已禁止密钥同步,该密钥只能保留在本机".into()));
    }
```
新增:
```rust
#[tauri::command]
pub fn set_recording_sync(state: State<AppState>, id: String, enabled: bool) -> Result<()> {
    if enabled && !crate::features::cached(&state.db).recording_sync {
        return Err(AppError("RECORDING_SYNC_DISABLED: 平台已禁止录制同步".into()));
    }
    state.db.set_recording_sync(&id, enabled)
}
```
注册到 `lib.rs`。

- [ ] **Step 4: 前端**

`types.ts` `SyncConfig.scopes` 加 `recordings: boolean`;`AccountSummary` 里那处硬编码 `scopes` 默认值加 `recordings: false`。`api.ts`/`tauri.ts` 加 `setRecordingSync(id, enabled)`。`stores/data.ts` 加:
```ts
  setRecordingSync: async (id, enabled) => {
    await ipc.setRecordingSync(id, enabled);
    set({ recordings: get().recordings.map((r) => (r.id === id ? { ...r, syncEnabled: enabled } : r)) });
  },
```
`ScopeCard.tsx`:`SCOPES` 加 `{ key: "recordings", label: "会话录制", desc: "asciicast 文件经主密码加密后上传;文件较大,建议按需开启" }`;渲染时按平台开关禁用:
```tsx
  const flags = useFeaturesStore((s) => s.features);
  const blocked = (key: string) => (key === "keys" && !flags.keySync) || (key === "recordings" && !flags.recordingSync);
  // 行内:{blocked(s.key) && <Badge color="warning">平台已禁止</Badge>},Switch 的 on 取 config.scopes[key] && !blocked(key),onChange 在 blocked 时不调用
```
`RightDock.tsx` 录制列表每行在时长之前加同步开关(`Switch` 尺寸沿用组件默认):
```tsx
  const recSyncOn = useFeaturesStore((s) => s.features.recordingSync);
  const setRecordingSync = useDataStore((s) => s.setRecordingSync);
  // 行内:
  <span title={recSyncOn ? (r.syncEnabled ? "随云同步上传" : "仅本地") : "平台已禁止录制同步"} style={{ opacity: recSyncOn ? 1 : 0.45 }}>
    <Switch on={r.syncEnabled && recSyncOn} onChange={(on) => recSyncOn && void setRecordingSync(r.id, on)} />
  </span>
```
脚注文案改为 `录制以 asciicast 格式保存;打开开关后随云同步上传(需在同步范围里启用「会话录制」)。`。

`KeysScreen.tsx` 的「已同步 / 仅本地」badge 改成可点击切换:
```tsx
  const keySyncOn = useFeaturesStore((s) => s.features.keySync);
  const setKeySync = useDataStore((s) => s.setKeySync);
  // badge 外包一层:
  <span className="clickable" title={keySyncOn ? "点击切换是否随云同步" : "平台已禁止密钥同步"}
    onClick={() => keySyncOn && void setKeySync(k.id, !k.syncEnabled)}>
    <Badge color={k.syncEnabled && keySyncOn ? "success" : "neutral"} dot>{k.syncEnabled && keySyncOn ? "已同步" : "仅本地"}</Badge>
  </span>
```

- [ ] **Step 5: 验证**

Run: `cd desktop && pnpm exec tsc -b && cd src-tauri && cargo check 2>&1 | tail -2`。
集成:`cargo run --example sync_check`(已有,走 MinIO)应仍通过;在 App 里录一段、打开录制同步与范围开关、同步一次,MinIO 控制台看到 `users/{id}/recordings/{id}.cast/.meta`;第二台设备(或删本地行后)同步能拉回并可回放。

- [ ] **Step 6: Commit(需用户允许)**

```bash
git add desktop/src-tauri/src desktop/src
git commit -m "feat(desktop): 密钥同步受平台开关约束;会话录制加密同步(推拉+元数据)"
```

---

### Task 10: AI 子功能 —— 输出摘要模式、错误诊断提示、入口门禁

**Files:**
- Modify: `desktop/src-tauri/src/ai.rs`(`AskMode::Summary`)
- Create: `desktop/src/lib/errorDetect.ts`、`desktop/src/lib/errorDetect.test.ts`
- Modify: `desktop/package.json`(test 脚本加 errorDetect)
- Modify: `desktop/src/stores/ai.ts`(`summarize`)、`desktop/src/stores/session.ts`(错误提示状态)
- Modify: `desktop/src/screens/TerminalScreen.tsx`、`desktop/src/components/AiDock.tsx`

**Interfaces:**
- Produces:`looksLikeError(lines: string[]): boolean`;`useSessionStore().errorHint: { sessionId, snippet } | null` + `clearErrorHint()`;`useAiStore().summarize(terminalContext)`;`ai_ask` 的 `mode` 新增 `"summary"`;快捷键 ⌘⇧S。

- [ ] **Step 1: 失败测试 —— errorDetect**

`desktop/src/lib/errorDetect.test.ts`:
```ts
/**
 * 终端错误识别用例:node --experimental-strip-types src/lib/errorDetect.test.ts
 */
import { looksLikeError } from "./errorDetect.ts";

let failed = 0;
function expect(lines: string[], want: boolean, why: string) {
  const got = looksLikeError(lines);
  if (got !== want) { failed++; console.error(`  ✗ ${JSON.stringify(lines)} → ${got},应为 ${want}(${why})`); }
}

expect(["zsh: command not found: kubeclt", "$ "], true, "command not found");
expect(["bash: /etc/shadow: Permission denied"], true, "permission denied");
expect(["ls: cannot access 'nope': No such file or directory"], true, "no such file");
expect(["error[E0425]: cannot find value `x` in this scope"], true, "编译器 error");
expect(["fatal: not a git repository (or any of the parent directories): .git"], true, "git fatal");
expect(["Traceback (most recent call last):", "  File \"a.py\", line 1", "NameError: name 'x' is not defined"], true, "python traceback");
expect(["E: Unable to locate package foo"], true, "apt E:");
expect(["total 8", "drwxr-xr-x 2 u u 4096 Sep 6 10:00 ."], false, "普通 ls 输出");
expect(["Building... done", "$ "], false, "正常完成");
expect(["Error handling is described in chapter 3"], false, "句中提到 Error 不算");
expect([], false, "空");

if (failed) { console.error(`${failed} 个用例失败`); process.exit(1); }
console.log("errorDetect: 全部通过");
```
Run: `cd desktop && node --experimental-strip-types src/lib/errorDetect.test.ts` → 失败(模块不存在)。

- [ ] **Step 2: 实现**

`desktop/src/lib/errorDetect.ts`:
```ts
/** 常见失败输出的行首/行内模式 —— 只看最近几行,命中即提示「AI 诊断」 */
const PATTERNS: RegExp[] = [
  /command not found/i,
  /permission denied/i,
  /no such file or directory/i,
  /^\s*(error|fatal|panic)(\[[A-Z0-9]+\])?\s*:/i,
  /^\s*[A-Za-z]+Error:/,           // NameError: / TypeError:
  /^\s*E:\s/,                      // apt
  /^\s*npm ERR!/,
  /Traceback \(most recent call last\)/,
  /segmentation fault/i,
  /connection refused/i,
  /exit(ed)? (with )?(code|status) [1-9]\d*/i,
];

export function looksLikeError(lines: string[]): boolean {
  const recent = lines.slice(-6).map((l) => l.trimEnd()).filter((l) => l.length > 0);
  return recent.some((l) => PATTERNS.some((p) => p.test(l)));
}
```
`package.json` 的 `test` 脚本追加 `&& node --experimental-strip-types src/lib/errorDetect.test.ts`。
Run 测试 → 全部通过。

- [ ] **Step 3: 会话 store 检测**

`stores/session.ts`:`SessionState` 加 `errorHint: { sessionId: string; snippet: string } | null; diagnoseEnabled: boolean; setDiagnoseEnabled: (on: boolean) => void; clearErrorHint: () => void;`。`wire()` 的 `onSessionData` 回调改为:
```ts
    let debounce: ReturnType<typeof setTimeout> | null = null;
    ipc.onSessionData((id, data) => {
      terminals.get(id)?.write(data);
      if (!get().diagnoseEnabled || id !== get().activeId) return;
      if (debounce) clearTimeout(debounce);
      debounce = setTimeout(() => {
        const ctx = getTerminalContext(id, 8);
        const lines = ctx.split("\n");
        if (looksLikeError(lines)) set({ errorHint: { sessionId: id, snippet: lines.slice(-3).join("\n") } });
      }, 350);
    });
```
`runCommand`/用户按回车(`term.onData` 里 `data === "\r"` 分支末尾)时 `set({ errorHint: null })`,避免旧提示挂着。

- [ ] **Step 4: AI 摘要模式**

`ai.rs`:`AskMode` 加 `Summary`,`parse` 加 `"summary" => AskMode::Summary`,提示词:
```rust
            AskMode::Summary => {
                "你是 KiteTerm(桌面 SSH 终端)内置的终端助手。用户会给你一段较长的终端输出(日志、diff、构建输出等),\
                 请用中文给出结构化摘要:先一句话总结,再用 3-6 个要点列出关键信息(错误、警告、数字指标、变更点),\
                 最后若有需要关注的问题给出一条建议。不要逐行复述,总长度控制在 200 字内。"
            }
```
`commands.rs` 的 `ai_ask` 注释补 `summary`。`stores/ai.ts` 加:
```ts
  summarize: async (terminalContext) => {
    const ask = "总结一下上面这段终端输出。";
    const next = [...get().messages, { role: "user" as const, text: ask }];
    set({ messages: next, busy: true });
    try {
      const reply = await ipc.aiAsk([{ role: "user", content: ask }], terminalContext, "summary");
      set({ messages: [...next, { role: "assistant", text: reply }] });
    } catch (e) {
      set({ messages: [...next, { role: "assistant", text: friendly(e) }] });
    } finally { set({ busy: false }); }
  },
```

- [ ] **Step 5: TerminalScreen 门禁与入口**

```tsx
  const aiOn = useFeaturesStore((s) => s.features.ai);
  const cmdOn = useAiFeature("aiCommand");
  const diagOn = useAiFeature("aiDiagnose");
  const sumOn = useAiFeature("aiSummary");
  const summarize = useAiStore((s) => s.summarize);
  const errorHint = useSessionStore((s) => s.errorHint);
  const clearErrorHint = useSessionStore((s) => s.clearErrorHint);
  const setDiagnoseEnabled = useSessionStore((s) => s.setDiagnoseEnabled);
  useEffect(() => { setNlEnabled(nlSetting && cmdOn); }, [nlSetting, cmdOn, setNlEnabled]);
  useEffect(() => { setDiagnoseEnabled(diagOn); }, [diagOn, setDiagnoseEnabled]);

  const summarizeOutput = () => {
    if (!active) return;
    setRightTab("ai");
    void summarize(getTerminalContext(active.id, 400));
  };
```
快捷键:⌘⇧A 条件 `cmdOn`;⌘I 条件 `diagOn`;新增 `else if (sumOn && mod && e.shiftKey && (e.key === "s" || e.key === "S") && active) { e.preventDefault(); summarizeOutput(); }`;依赖数组换成 `[active, close, setPalette, cmdOn, diagOn, sumOn]`。
标签栏:`{aiOn && (...Sparkles...)}` 改 `cmdOn`;解释按钮改 `diagOn && active`;新增摘要按钮(`sumOn && active`,图标 `AlignLeft`,title「AI 摘要最近输出(⌘⇧S)」)。`AiCommandBar` 两处与 `AiFab` 改 `cmdOn`。
终端表面里、`XtermPane` 之后加错误提示条:
```tsx
              {diagOn && errorHint && errorHint.sessionId === active.id && (
                <div className="error-hint">
                  <AlertTriangle size={13} />
                  <span className="grow ellipsis">检测到命令失败</span>
                  <span className="clickable t-accent" onClick={() => { clearErrorHint(); explainOutput(); }}>AI 诊断</span>
                  <span className="clickable t-faint" onClick={clearErrorHint}>忽略</span>
                </div>
              )}
```
`components.css` 加:
```css
.error-hint{position:absolute;left:12px;bottom:12px;display:flex;align-items:center;gap:8px;max-width:420px;padding:6px 10px;border-radius:var(--radius-md);background:var(--danger-subtle);color:var(--danger-text);font-size:12px;box-shadow:var(--shadow-md)}
```
`AiDock.tsx` 空态提示按开关拼接:`{diagOn && "⌘I 解释最近输出"}{sumOn && " · ⌘⇧S 摘要长输出"}{cmdOn && " · ⌘⇧A 生成命令"}`(从 `useAiFeature` 取)。

- [ ] **Step 6: 验证**

Run: `cd desktop && pnpm test && pnpm exec tsc -b && cd src-tauri && cargo check 2>&1 | tail -2`。
App 内:敲 `nosuchcmd` 回车 → 左下出现「检测到命令失败 · AI 诊断」;⌘⇧S 出摘要;控制台关掉「命令补全与解释」等 60 s 后 Sparkles 按钮、⌘⇧A、白话拦截全部消失。

- [ ] **Step 7: Commit(需用户允许)**

```bash
git add desktop/src desktop/src-tauri/src/ai.rs desktop/package.json
git commit -m "feat(desktop): AI 输出摘要、失败输出自动提示诊断,三项子功能受平台开关约束"
```

---

### Task 11: 文档

**Files:**
- Modify: `README.md`(管理平台段:环境变量表、管理 API 列表、功能开关说明)
- Modify: `server/src/main.rs` 头注释

- [ ] **Step 1: README**

管理 API 列表补:`GET|POST /api/admin/storage`、`POST /api/admin/storage/test`、`POST /api/admin/users/{email}/verify`、`POST /api/admin/users/{email}/2fa/reset`;账户 API 补 `POST /api/verify-email`、`POST /api/verify-email/resend`、`POST /api/2fa/setup|enable|disable`、`GET /api/me`。环境变量表补 `S3_PREFIX`、`S3_PATH_STYLE`、`KITE_SMTP_HOST/PORT/USER/PASS/FROM`,并注明 `S3_*` 仅作初值。功能开关一节改为 11 项两类的表格(产品开关 / 安全策略),并写明客户端 60 s 轮询。已知简化清单补:"S3 secret 明文存 SQLite(与环境变量同强度)"、"TOTP 密钥明文存库"、"未配 SMTP 时验证码打日志"。

- [ ] **Step 2: Commit(需用户允许)**

```bash
git add README.md server/src/main.rs
git commit -m "docs: 管理平台存储配置、邮箱验证/2FA、11 项功能开关说明"
```

---

## 自检记录

- 原型覆盖:三页签(T5)、S3 表单六字段 + Path-style + STS + 测试/保存 + 状态徽章 + IAM 脚注(T2/T5)、登录与注册 4 项(开放注册=注册策略 T5;登录 T1;邮箱验证 T3/T8;强制 2FA T4/T8;多设备 T4)、云同步 3 项(全局 T1;密钥同步 T9;录制同步 T9)、AI 总开关 + 3 子项(T1/T10)、用户页(已完成于本会话前半段)、功能页脚注的 60 s 生效(T6)。
- 类型一致性:`features::raw/effective/set/payload/admin_payload`(T1)被 T2–T4 使用;`StorageConfig.user_prefix/bucket/public_json`(T2)被 T2 handlers 与 T5 使用;`AccountInfo.totp_setup_required`(T7)被 T8 使用;`useAiFeature`(T6)被 T10 使用;`ipc.setRecordingSync`(T9)与 store 同名。
- 占位扫描:无 TBD/TODO;T5 两处「原样搬进来」指向现有文件中已存在的完整片段。
