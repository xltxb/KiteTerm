//! KiteTerm 管理平台
//!
//! 职责:账户注册/登录(Argon2id 口令哈希 + 不透明令牌)、同步授权开关、
//! 平台持有的 S3 桶凭据下发(按用户前缀 users/{user_id} 隔离)。
//! 平台不接触用户主密码 —— 桶内仍是客户端主密钥加密的密文,平台零知识。
//!
//! 配置(环境变量):
//!   KITE_LISTEN       默认 127.0.0.1:8787
//!   KITE_DB           默认 ./kite-server.db
//!   KITE_ADMIN_KEY    管理接口密钥(必填)
//!   KITE_REGISTRATION 注册策略初值 open|invite|closed(默认 open;之后以管理后台设置为准)
//!   KITE_FEATURE_GATING 功能门禁总闸 on|off(默认 off;关闭时产品开关全部失效,安全策略类不受影响)
//!   KITE_REGISTER_LIMIT 同一 IP 每小时注册尝试上限(默认 5)
//!   S3_ENDPOINT / S3_BUCKET / S3_REGION / S3_ACCESS_KEY / S3_SECRET_KEY / S3_PREFIX / S3_PATH_STYLE / KITE_STS
//!       —— 仅作存储配置初值;控制台「S3 存储」页保存后以 DB 为准,不再读环境变量。
//!       KITE_STS 取值 auto|require|off(默认 auto),控制 STS 限权凭据的下发策略。
//!   KITE_SMTP_HOST / KITE_SMTP_PORT / KITE_SMTP_USER / KITE_SMTP_PASS / KITE_SMTP_FROM
//!       —— 邮箱验证码发信配置;未设置 KITE_SMTP_HOST 时验证码只打进服务端日志。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::json;

mod features;
mod mail;
mod storage;
mod sts;
mod totp;

struct PlatformConfig {
    admin_key: String,
    /// 注册策略初值(仅在 DB 尚无设置时生效)
    registration_default: String,
    /// 功能门禁总闸(KITE_FEATURE_GATING,默认 off)。
    ///
    /// off:8 项产品开关(login/sync/ai/key_sync/recording_sync/ai_command/
    ///     ai_diagnose/ai_summary)全部失效 —— 客户端不隐藏任何入口,服务端
    ///     也不据此拦截。平台只当账户与凭据的中枢,不管功能可见性。
    ///     安全策略类开关(email_verify/totp_required/multi_device)不受总闸
    ///     影响,总闸关闭时依旧按各自设置生效。
    /// on: 8 项产品开关生效,由管理控制台逐项开关并下发。
    ///
    /// 之所以做成部署配置而不是控制台开关:它决定的是这套管控要不要存在,
    /// 属于部署形态,不该被日常运维误点。
    feature_gating: bool,
    /// 验证码发信配置;None 时验证码只打进服务端日志
    smtp: Option<mail::Smtp>,
}

struct AppState {
    db: Mutex<Connection>,
    cfg: PlatformConfig,
    /// S3 存储配置:运行期可热改(管理控制台保存后立即生效,无需重启)
    storage: std::sync::RwLock<storage::StorageConfig>,
    /// 当前生效配置的来源:"db" | "env",供控制台展示
    storage_source: Mutex<&'static str>,
    /// 通用滑动窗口限流:`"{bucket}:{key}"` → 最近命中的时间戳。
    /// bucket 区分场景(register / totp / verify_email 等),key 是该场景内的限流对象
    /// (IP、`用户id:IP` 或用户 id)。
    rate_hits: Mutex<HashMap<String, Vec<i64>>>,
}

/// 注册限流:同一 IP 在窗口内的注册尝试上限(KITE_REGISTER_LIMIT 可调)
const REGISTER_WINDOW_SECS: i64 = 3600;
const REGISTER_MAX_PER_WINDOW: usize = 5;
const PASSWORD_MIN_LEN: usize = 8;
const PASSWORD_MAX_LEN: usize = 128;

type ApiError = (StatusCode, Json<serde_json::Value>);

fn err(status: StatusCode, code: &str, msg: &str) -> ApiError {
    (status, Json(json!({ "error": { "code": code, "message": msg } })))
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    err(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string())
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

// ---- 注册校验 ----

/// 邮箱基本结构校验:单个 @、本地段与域名段非空、域名含点且各段合法、无空白
fn valid_email(email: &str) -> bool {
    if email.len() > 254 || email.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return false;
    }
    let (local, domain) = (parts[0], parts[1]);
    if local.is_empty() || local.len() > 64 || domain.is_empty() || domain.len() > 255 {
        return false;
    }
    if !domain.contains('.') {
        return false;
    }
    domain
        .split('.')
        .all(|seg| !seg.is_empty() && !seg.starts_with('-') && !seg.ends_with('-')
            && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// 口令强度:长度 + 至少两类字符 + 排除常见弱口令。返回 Some(原因) 表示不合格。
fn password_issue(pw: &str) -> Option<String> {
    let len = pw.chars().count();
    if len < PASSWORD_MIN_LEN {
        return Some(format!("密码至少 {PASSWORD_MIN_LEN} 位"));
    }
    if len > PASSWORD_MAX_LEN {
        return Some(format!("密码最多 {PASSWORD_MAX_LEN} 位"));
    }
    let has_letter = pw.chars().any(|c| c.is_alphabetic());
    let has_digit = pw.chars().any(|c| c.is_numeric());
    let has_symbol = pw.chars().any(|c| !c.is_alphanumeric() && !c.is_whitespace());
    let kinds = [has_letter, has_digit, has_symbol].iter().filter(|k| **k).count();
    if kinds < 2 {
        return Some("密码需包含字母、数字、符号中的至少两类".into());
    }
    if pw.chars().collect::<std::collections::HashSet<_>>().len() < 4 {
        return Some("密码字符过于单一".into());
    }
    const WEAK: &[&str] = &[
        "password", "password1", "password123", "12345678", "123456789", "1234567890",
        "qwerty123", "admin123", "abc12345", "iloveyou", "letmein1", "welcome1",
    ];
    let lower = pw.to_lowercase();
    if WEAK.contains(&lower.as_str()) {
        return Some("密码过于常见,请更换".into());
    }
    None
}

// ---- 平台设置(DB 持久化,管理后台可改)----

pub(crate) fn setting_get(conn: &Connection, key: &str, default: &str) -> String {
    conn.query_row(
        "SELECT value FROM platform_settings WHERE key = ?1",
        [key],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or_else(|| default.to_string())
}

pub(crate) fn setting_set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO platform_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = ?2",
        params![key, value],
    )?;
    Ok(())
}

/// 客户端 IP:优先反向代理头,回退连接地址
fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| headers.get("x-real-ip").and_then(|v| v.to_str().ok()).map(String::from))
        .unwrap_or_else(|| peer.map(|p| p.ip().to_string()).unwrap_or_else(|| "unknown".into()))
}

fn register_limit() -> usize {
    std::env::var("KITE_REGISTER_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(REGISTER_MAX_PER_WINDOW)
}

/// 通用滑动窗口限流(窗口固定 3600 s);超限返回 false。
/// `bucket` 区分场景(register/totp/verify_email),`key` 是该场景内的限流对象。
fn rate_limit_ok(state: &AppState, bucket: &str, key: &str, max: usize) -> bool {
    let now = now_ts();
    let mut hits = state.rate_hits.lock().unwrap();
    let entry = hits.entry(format!("{bucket}:{key}")).or_default();
    entry.retain(|t| now - *t < REGISTER_WINDOW_SECS);
    if entry.len() >= max {
        return false;
    }
    entry.push(now);
    // 顺带清理其它键的过期记录,避免无限增长
    if hits.len() > 1024 {
        hits.retain(|_, v| {
            v.retain(|t| now - *t < REGISTER_WINDOW_SECS);
            !v.is_empty()
        });
    }
    true
}

fn new_invite_code() -> String {
    // 去掉易混字符(0/O/1/I/L),便于口头/手抄传达
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let mut raw = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut raw);
    let body: String = raw
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect();
    format!("{}-{}-{}", &body[0..4], &body[4..8], &body[8..12])
}

/// 签发登录令牌(注册与登录共用)
fn issue_token(conn: &Connection, user_id: &str) -> Result<(String, i64), ApiError> {
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let expires_at = now_ts() + 30 * 24 * 3600;
    conn.execute(
        "INSERT INTO tokens (token, user_id, expires_at) VALUES (?1, ?2, ?3)",
        params![token, user_id, expires_at],
    )
    .map_err(internal)?;
    Ok((token, expires_at))
}

// ---- 数据模型 ----

#[derive(Deserialize)]
struct Credentials {
    email: String,
    password: String,
    /// 仅 invite 模式需要
    #[serde(default)]
    invite_code: Option<String>,
    /// 已绑定双因素认证的账户登录时必填
    #[serde(default)]
    totp: Option<String>,
}

struct AuthedUser {
    id: String,
    email: String,
    sync_enabled: bool,
    email_verified: bool,
    totp_enabled: bool,
}

fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<AuthedUser, ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "unauthorized", "缺少令牌"))?;
    let conn = state.db.lock().unwrap();
    let row: Option<(String, String, i64, i64, i64, i64)> = conn
        .query_row(
            "SELECT u.id, u.email, u.sync_enabled, u.email_verified, u.totp_enabled, t.expires_at
             FROM tokens t JOIN users u ON u.id = t.user_id WHERE t.token = ?1",
            [token],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()
        .map_err(internal)?;
    match row {
        Some((id, email, sync_enabled, email_verified, totp_enabled, expires_at)) if expires_at > now_ts() => Ok(AuthedUser {
            id,
            email,
            sync_enabled: sync_enabled != 0,
            email_verified: email_verified != 0,
            totp_enabled: totp_enabled != 0,
        }),
        Some(_) => Err(err(StatusCode::UNAUTHORIZED, "token_expired", "令牌已过期,请重新登录")),
        None => Err(err(StatusCode::UNAUTHORIZED, "unauthorized", "令牌无效")),
    }
}

// ---- handlers ----

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "service": "kite-server" }))
}

/// 全局功能开关。注册用三档 registration_mode 表达(closed 即关),
/// 其余是布尔;默认全开 —— 平台没显式关过的功能就是可用的。
/// 具体注册表见 `features` 模块。
/// 客户端据此决定哪些入口可见。**免鉴权** —— 客户端未登录时也要能拿到,
/// 否则「登录开关」自身就没法下发。
async fn features(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let conn = state.db.lock().unwrap();
    let mut body = features::payload(&conn, state.cfg.feature_gating);
    // 注册策略是平台自己的准入政策,与功能门禁无关,总闸关掉也照样生效
    body.insert("registration".into(), json!(setting_get(&conn, "registration_mode", &state.cfg.registration_default)));
    body.insert("enforced".into(), json!(state.cfg.feature_gating));
    body.insert("passwordMinLength".into(), json!(PASSWORD_MIN_LEN));
    body.insert("passwordRule".into(), json!("至少 8 位,且包含字母、数字、符号中的至少两类"));
    Ok(Json(serde_json::Value::Object(body)))
}

/// 公开的注册策略:客户端据此决定是否展示邀请码输入、以及口令规则提示
async fn register_policy(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let conn = state.db.lock().unwrap();
    let mode = setting_get(&conn, "registration_mode", &state.cfg.registration_default);
    Ok(Json(json!({
        "mode": mode,
        "passwordMinLength": PASSWORD_MIN_LEN,
        "passwordRule": "至少 8 位,且包含字母、数字、符号中的至少两类",
    })))
}

async fn register(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    // 限流放在最前:注册接口会通过 email_taken 泄露邮箱是否已注册,
    // 必须先压制暴力枚举,再做其余校验
    let ip = client_ip(&headers, Some(peer));
    if !rate_limit_ok(&state, "register", &ip, register_limit()) {
        return Err(err(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "注册过于频繁,请稍后再试",
        ));
    }

    let email = body.email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(err(StatusCode::BAD_REQUEST, "invalid_email", "邮箱格式不正确"));
    }
    if let Some(reason) = password_issue(&body.password) {
        return Err(err(StatusCode::BAD_REQUEST, "weak_password", &reason));
    }

    let conn = state.db.lock().unwrap();
    let mode = setting_get(&conn, "registration_mode", &state.cfg.registration_default);
    if mode == "closed" {
        return Err(err(StatusCode::FORBIDDEN, "registration_closed", "平台当前未开放注册"));
    }

    // 邀请码:invite 模式必填并校验有效性(未停用 / 未过期 / 未用尽)
    let invite = body.invite_code.as_deref().map(str::trim).filter(|c| !c.is_empty());
    if mode == "invite" {
        let Some(code) = invite else {
            return Err(err(StatusCode::BAD_REQUEST, "invite_required", "该平台需要邀请码才能注册"));
        };
        let row: Option<(i64, i64, i64, i64)> = conn
            .query_row(
                "SELECT max_uses, used_count, expires_at, disabled FROM invites WHERE code = ?1",
                [code],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(internal)?;
        let Some((max_uses, used_count, expires_at, disabled)) = row else {
            return Err(err(StatusCode::BAD_REQUEST, "invite_invalid", "邀请码无效"));
        };
        if disabled != 0 {
            return Err(err(StatusCode::BAD_REQUEST, "invite_invalid", "邀请码已停用"));
        }
        if expires_at != 0 && expires_at < now_ts() {
            return Err(err(StatusCode::BAD_REQUEST, "invite_expired", "邀请码已过期"));
        }
        if max_uses != 0 && used_count >= max_uses {
            return Err(err(StatusCode::BAD_REQUEST, "invite_exhausted", "邀请码使用次数已用尽"));
        }
    }

    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(internal)?
        .to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let default_sync = setting_get(&conn, "default_sync_enabled", "1");
    // 邮箱验证开关开着时,新用户先落地为未验证,注册成功后即发验证码
    let need_verify = features::raw(&conn, "email_verify");
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO users (id, email, password_hash, sync_enabled, created_at, email_verified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, email, hash, if default_sync == "1" { 1 } else { 0 }, now_ts(), if need_verify { 0 } else { 1 }],
        )
        .map_err(internal)?;
    if inserted == 0 {
        return Err(err(StatusCode::CONFLICT, "email_taken", "该邮箱已注册"));
    }

    // 记账邀请码用量(注册成功后才消费)
    if mode == "invite" {
        if let Some(code) = invite {
            conn.execute("UPDATE invites SET used_count = used_count + 1 WHERE code = ?1", [code])
                .map_err(internal)?;
            conn.execute(
                "INSERT INTO invite_uses (code, user_id, used_at) VALUES (?1, ?2, ?3)",
                params![code, id, now_ts()],
            )
            .map_err(internal)?;
        }
    }

    // 注册即登录:直接签发令牌,免去客户端再发一次登录请求
    let (token, expires_at) = issue_token(&conn, &id)?;
    let sync_enabled = default_sync == "1";

    // ---- 邮箱验证 ----
    if need_verify {
        let code = mail::new_code();
        conn.execute(
            "INSERT INTO email_codes (user_id, code, expires_at, sent_at, attempts) VALUES (?1, ?2, ?3, ?4, 0)
             ON CONFLICT(user_id) DO UPDATE SET code=?2, expires_at=?3, sent_at=?4, attempts=0",
            params![id, code, now_ts() + 600, now_ts()],
        ).map_err(internal)?;
        // 发信不借用 conn(MutexGuard),spawn 前先克隆出去,避免把锁跨 await 持有
        let smtp = state.cfg.smtp.clone();
        let to = email.clone();
        tokio::spawn(async move {
            if let Err(e) = mail::send_code(smtp.as_ref(), &to, &code).await { eprintln!("[mail] {e}"); }
        });
    }

    let totp_setup_required = features::raw(&conn, "totp_required");
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "userId": id,
            "token": token,
            "expiresAt": expires_at,
            "totpSetupRequired": totp_setup_required,
            "user": { "id": id, "email": email, "syncEnabled": sync_enabled, "emailVerified": !need_verify, "totpEnabled": false },
        })),
    ))
}

async fn login(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let email = body.email.trim().to_lowercase();
    let conn = state.db.lock().unwrap();
    let row: Option<(String, String, i64, i64, Option<String>, i64)> = conn
        .query_row(
            "SELECT id, password_hash, sync_enabled, email_verified, totp_secret, totp_enabled FROM users WHERE email = ?1",
            [&email],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()
        .map_err(internal)?;
    let Some((user_id, hash, sync_enabled, email_verified, totp_secret, totp_enabled)) = row else {
        return Err(err(StatusCode::UNAUTHORIZED, "bad_credentials", "邮箱或密码错误"));
    };
    let parsed = PasswordHash::new(&hash).map_err(internal)?;
    if Argon2::default()
        .verify_password(body.password.as_bytes(), &parsed)
        .is_err()
    {
        return Err(err(StatusCode::UNAUTHORIZED, "bad_credentials", "邮箱或密码错误"));
    }
    // 开关校验放在口令校验之后:先把凭据不对的挡掉,避免通过错误信息区分
    // 「密码错」和「登录已关」而泄露邮箱是否存在
    if !features::effective(&conn, state.cfg.feature_gating, "login") {
        return Err(err(StatusCode::FORBIDDEN, "LOGIN_DISABLED", "平台当前已关闭登录"));
    }
    if totp_enabled != 0 {
        // 猜测节流:同一账户 + 同一 IP 每小时最多尝试 10 次,防暴力破解 6 位动态验证码
        let ip = client_ip(&headers, Some(peer));
        if !rate_limit_ok(&state, "totp", &format!("{user_id}:{ip}"), 10) {
            return Err(err(
                StatusCode::TOO_MANY_REQUESTS,
                "TOTP_RATE_LIMITED",
                "动态验证码尝试过于频繁,请稍后再试",
            ));
        }
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
}

async fn logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    if let Some(token) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        let conn = state.db.lock().unwrap();
        conn.execute("DELETE FROM tokens WHERE token = ?1", [token]).map_err(internal)?;
    }
    Ok(Json(json!({ "ok": true })))
}

async fn me(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    Ok(Json(json!({
        "id": user.id, "email": user.email, "syncEnabled": user.sync_enabled,
        "emailVerified": user.email_verified, "totpEnabled": user.totp_enabled,
        "requirements": {
            "emailVerify": features::raw(&conn, "email_verify"),
            "totpRequired": features::raw(&conn, "totp_required"),
        },
    })))
}

/// 平台管控的同步凭据:登录且账户同步已放开时下发。
/// 优先经 STS 签发按 users/{id}/* 收敛的限权临时凭据(1h);
/// STS 不可用时回退共享凭据 + 前缀约定(KITE_STS=require 则直接报错)。
async fn sync_credentials(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let cfg = state.storage.read().unwrap().clone();
    if !cfg.is_configured() {
        return Err(err(
            StatusCode::SERVICE_UNAVAILABLE,
            "STORAGE_UNCONFIGURED",
            "平台尚未配置存储,请联系管理员",
        ));
    }
    {
        let conn = state.db.lock().unwrap();
        // 顺序:平台总闸 → 用户级开关 → 邮箱验证 → 强制 2FA —— 越靠前的检查越该
        // 先挡下,避免被停用账户走到后面的检查里泄露「平台还要求什么」
        if !features::effective(&conn, state.cfg.feature_gating, "sync") {
            return Err(err(
                StatusCode::FORBIDDEN,
                "SYNC_DISABLED",
                "平台当前已关闭云同步",
            ));
        }
        if !user.sync_enabled {
            return Err(err(
                StatusCode::FORBIDDEN,
                "SYNC_DISABLED",
                "该账户的云同步已被平台停用,请联系管理员",
            ));
        }
        if features::raw(&conn, "email_verify") && !user.email_verified {
            return Err(err(StatusCode::FORBIDDEN, "EMAIL_UNVERIFIED", "请先完成邮箱验证"));
        }
        if features::raw(&conn, "totp_required") && !user.totp_enabled {
            return Err(err(StatusCode::FORBIDDEN, "TOTP_SETUP_REQUIRED", "平台要求绑定双因素认证后才能同步"));
        }
    }
    let prefix = cfg.user_prefix(&user.id);
    if cfg.sts_mode != "off" {
        match sts::assume_role_for_user(
            &cfg.endpoint,
            &cfg.region,
            &cfg.access_key,
            &cfg.secret_key,
            &cfg.bucket,
            &cfg.prefix,
            &user.id,
            3600,
        )
        .await
        {
            Ok(t) => {
                return Ok(Json(json!({
                    "endpoint": cfg.endpoint,
                    "bucket": cfg.bucket,
                    "region": cfg.region,
                    "accessKey": t.access_key,
                    "secretKey": t.secret_key,
                    "sessionToken": t.session_token,
                    "expiration": t.expiration,
                    "prefix": prefix,
                    "pathStyle": cfg.path_style,
                })));
            }
            Err(e) if cfg.sts_mode == "require" => {
                return Err(err(StatusCode::BAD_GATEWAY, "sts_error", &e));
            }
            Err(e) => eprintln!("[server] STS 失败,回退共享凭据: {e}"),
        }
    }
    Ok(Json(json!({
        "endpoint": cfg.endpoint,
        "bucket": cfg.bucket,
        "region": cfg.region,
        "accessKey": cfg.access_key,
        "secretKey": cfg.secret_key,
        "prefix": prefix,
        "pathStyle": cfg.path_style,
    })))
}

// ---- 邮箱验证 ----

#[derive(Deserialize)]
struct CodeBody {
    code: String,
}

async fn verify_email(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = authenticate(&state, &headers)?;
    // 猜测节流:同一账户每小时最多尝试 10 次,与下面按次数删码互为补充
    if !rate_limit_ok(&state, "verify_email", &user.id, 10) {
        return Err(err(
            StatusCode::TOO_MANY_REQUESTS,
            "CODE_RATE_LIMITED",
            "验证码尝试过于频繁,请稍后再试",
        ));
    }
    let conn = state.db.lock().unwrap();
    let row: Option<(String, i64, i64)> = conn
        .query_row(
            "SELECT code, expires_at, attempts FROM email_codes WHERE user_id = ?1",
            [&user.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional().map_err(internal)?;
    let Some((code, expires_at, attempts)) = row else {
        return Err(err(StatusCode::BAD_REQUEST, "CODE_INVALID", "验证码不存在,请重新发送"));
    };
    if expires_at < now_ts() {
        return Err(err(StatusCode::BAD_REQUEST, "CODE_EXPIRED", "验证码已过期,请重新发送"));
    }
    if code != body.code.trim() {
        let attempts = attempts + 1;
        if attempts >= 5 {
            // 错误次数用尽:直接把码删掉,逼用户重新发送,而不是无限期让人猜
            conn.execute("DELETE FROM email_codes WHERE user_id = ?1", [&user.id]).map_err(internal)?;
            return Err(err(StatusCode::BAD_REQUEST, "CODE_INVALID", "验证码错误次数过多,请重新发送"));
        }
        conn.execute("UPDATE email_codes SET attempts = ?1 WHERE user_id = ?2", params![attempts, &user.id]).map_err(internal)?;
        return Err(err(StatusCode::BAD_REQUEST, "CODE_INVALID", "验证码错误"));
    }
    conn.execute("UPDATE users SET email_verified = 1 WHERE id = ?1", [&user.id]).map_err(internal)?;
    conn.execute("DELETE FROM email_codes WHERE user_id = ?1", [&user.id]).map_err(internal)?;
    Ok(Json(json!({ "ok": true, "emailVerified": true })))
}

async fn resend_verification(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
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
            "INSERT INTO email_codes (user_id, code, expires_at, sent_at, attempts) VALUES (?1, ?2, ?3, ?4, 0)
             ON CONFLICT(user_id) DO UPDATE SET code=?2, expires_at=?3, sent_at=?4, attempts=0",
            params![user.id, code, now_ts() + 600, now_ts()],
        ).map_err(internal)?;
    }
    mail::send_code(state.cfg.smtp.as_ref(), &user.email, &code).await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, "MAIL_ERROR", &e))?;
    Ok(Json(json!({ "ok": true, "emailVerified": false })))
}

/// 管理员手工标记已验证(没有邮件服务的部署)
async fn admin_verify_user(
    State(state): State<Arc<AppState>>,
    Path(email): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let email = email.trim().to_lowercase();
    let n = conn.execute("UPDATE users SET email_verified = 1 WHERE email = ?1", [&email]).map_err(internal)?;
    if n == 0 { return Err(err(StatusCode::NOT_FOUND, "not_found", "用户不存在")); }
    // 顺带清掉待验证的验证码 —— 已被管理员手工标记,残留的码/尝试次数没有意义
    conn.execute(
        "DELETE FROM email_codes WHERE user_id IN (SELECT id FROM users WHERE email = ?1)",
        [&email],
    ).map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let key = headers.get("x-admin-key").and_then(|v| v.to_str().ok()).unwrap_or("");
    if key.is_empty() || key != state.cfg.admin_key {
        return Err(err(StatusCode::UNAUTHORIZED, "unauthorized", "管理密钥错误"));
    }
    Ok(())
}

// ---- 双因素认证 ----

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

async fn admin_stats(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let cfg = state.storage.read().unwrap().clone();
    let conn = state.db.lock().unwrap();
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)).map_err(internal)?;
    let enabled: i64 = conn
        .query_row("SELECT COUNT(*) FROM users WHERE sync_enabled = 1", [], |r| r.get(0))
        .map_err(internal)?;
    let tokens: i64 = conn
        .query_row("SELECT COUNT(*) FROM tokens WHERE expires_at > ?1", [now_ts()], |r| r.get(0))
        .map_err(internal)?;
    Ok(Json(json!({
        "users": total,
        "syncEnabled": enabled,
        "activeTokens": tokens,
        "bucket": cfg.bucket,
        "endpoint": cfg.endpoint,
        // 控制台据此显示凭据模式徽章(STS 限权 / 共享回退)
        "stsMode": cfg.sts_mode,
        "storageConfigured": cfg.is_configured(),
    })))
}

/// 强制下线:吊销该用户全部令牌
async fn admin_revoke(
    State(state): State<Arc<AppState>>,
    Path(email): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let n = conn
        .execute(
            "DELETE FROM tokens WHERE user_id IN (SELECT id FROM users WHERE email = ?1)",
            [email.trim().to_lowercase()],
        )
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true, "revoked": n })))
}

/// 该用户前缀下的对象数与字节数
async fn admin_usage(
    State(state): State<Arc<AppState>>,
    Path(email): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let user_id: Option<String> = {
        let conn = state.db.lock().unwrap();
        conn.query_row(
            "SELECT id FROM users WHERE email = ?1",
            [email.trim().to_lowercase()],
            |r| r.get(0),
        )
        .optional()
        .map_err(internal)?
    };
    let Some(user_id) = user_id else {
        return Err(err(StatusCode::NOT_FOUND, "not_found", "用户不存在"));
    };
    let cfg = state.storage.read().unwrap().clone();
    let bucket = cfg.bucket().map_err(|e| err(StatusCode::BAD_GATEWAY, "S3_ERROR", &e))?;
    let pages = bucket
        .list(format!("{}/", cfg.user_prefix(&user_id)), None)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, "S3_ERROR", &e.to_string()))?;
    let (mut objects, mut bytes) = (0u64, 0u64);
    for page in pages {
        for obj in page.contents {
            objects += 1;
            bytes += obj.size;
        }
    }
    Ok(Json(json!({ "objects": objects, "bytes": bytes })))
}

async fn admin_ui() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("admin.html"))
}

// ---- S3 存储配置(管理接口)----

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
        return Err(err(StatusCode::BAD_REQUEST, "INVALID_ENDPOINT", "Endpoint 须以 http:// 或 https:// 开头"));
    }
    if cfg.bucket.is_empty() || cfg.access_key.is_empty() || cfg.secret_key.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "INVALID_STORAGE", "Bucket、Access Key、Secret 不能为空"));
    }
    if !storage::prefix_valid(&cfg.prefix) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "INVALID_STORAGE",
            "路径前缀只能包含字母、数字、. _ - /,且不能含 ..",
        ));
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

// ---- 注册策略与邀请码(管理接口)----

async fn admin_get_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    Ok(Json(settings_payload(&conn, &state)))
}

/// 管理接口与公开接口共用同一份取值,避免两边漂移
fn settings_payload(conn: &Connection, state: &AppState) -> serde_json::Value {
    let mut m = features::admin_payload(conn);
    m.insert("registrationMode".into(), json!(setting_get(conn, "registration_mode", &state.cfg.registration_default)));
    m.insert("defaultSyncEnabled".into(), json!(setting_get(conn, "default_sync_enabled", "1") == "1"));
    m.insert("featureGating".into(), json!(state.cfg.feature_gating));
    serde_json::Value::Object(m)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminSettingsBody {
    registration_mode: Option<String>,
    default_sync_enabled: Option<bool>,
    /// featureLogin / featureKeySync / … 任意子集
    #[serde(flatten)]
    features: HashMap<String, serde_json::Value>,
}

async fn admin_set_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<AdminSettingsBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    if let Some(mode) = body.registration_mode {
        if !["open", "invite", "closed"].contains(&mode.as_str()) {
            return Err(err(StatusCode::BAD_REQUEST, "invalid_mode", "注册策略只能是 open/invite/closed"));
        }
        setting_set(&conn, "registration_mode", &mode).map_err(internal)?;
    }
    if let Some(on) = body.default_sync_enabled {
        setting_set(&conn, "default_sync_enabled", if on { "1" } else { "0" }).map_err(internal)?;
    }
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
            return Err(err(StatusCode::BAD_REQUEST, "UNKNOWN_FEATURE", &format!("未知功能开关 {key}")));
        }
        features::set(&conn, &snake, on).map_err(internal)?;
    }
    Ok(Json(settings_payload(&conn, &state)))
}

async fn admin_list_invites(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT code, note, max_uses, used_count, expires_at, disabled, created_at
             FROM invites ORDER BY created_at DESC",
        )
        .map_err(internal)?;
    let invites: Vec<serde_json::Value> = stmt
        .query_map([], |r| {
            Ok(json!({
                "code": r.get::<_, String>(0)?,
                "note": r.get::<_, String>(1)?,
                "maxUses": r.get::<_, i64>(2)?,
                "usedCount": r.get::<_, i64>(3)?,
                "expiresAt": r.get::<_, i64>(4)?,
                "disabled": r.get::<_, i64>(5)? != 0,
                "createdAt": r.get::<_, i64>(6)?,
            }))
        })
        .map_err(internal)?
        .collect::<Result<_, _>>()
        .map_err(internal)?;
    Ok(Json(json!({ "invites": invites })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewInviteBody {
    #[serde(default)]
    note: String,
    /// 0 = 不限次数
    #[serde(default)]
    max_uses: i64,
    /// 0 = 永不过期
    #[serde(default)]
    expires_in_days: i64,
}

async fn admin_create_invite(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NewInviteBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let code = new_invite_code();
    let expires_at = if body.expires_in_days > 0 {
        now_ts() + body.expires_in_days * 86400
    } else {
        0
    };
    let conn = state.db.lock().unwrap();
    conn.execute(
        "INSERT INTO invites (code, note, max_uses, used_count, expires_at, disabled, created_at)
         VALUES (?1, ?2, ?3, 0, ?4, 0, ?5)",
        params![code, body.note, body.max_uses.max(0), expires_at, now_ts()],
    )
    .map_err(internal)?;
    Ok(Json(json!({ "code": code, "expiresAt": expires_at, "maxUses": body.max_uses.max(0) })))
}

async fn admin_disable_invite(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let n = conn
        .execute("UPDATE invites SET disabled = 1 WHERE code = ?1", [code.trim()])
        .map_err(internal)?;
    if n == 0 {
        return Err(err(StatusCode::NOT_FOUND, "not_found", "邀请码不存在"));
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct AdminSyncBody {
    enabled: bool,
}

async fn admin_set_sync(
    State(state): State<Arc<AppState>>,
    Path(email): Path<String>,
    headers: HeaderMap,
    Json(body): Json<AdminSyncBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let key = headers.get("x-admin-key").and_then(|v| v.to_str().ok()).unwrap_or("");
    if key.is_empty() || key != state.cfg.admin_key {
        return Err(err(StatusCode::UNAUTHORIZED, "unauthorized", "管理密钥错误"));
    }
    let conn = state.db.lock().unwrap();
    let n = conn
        .execute(
            "UPDATE users SET sync_enabled = ?2 WHERE email = ?1",
            params![email.trim().to_lowercase(), body.enabled as i64],
        )
        .map_err(internal)?;
    if n == 0 {
        return Err(err(StatusCode::NOT_FOUND, "not_found", "用户不存在"));
    }
    Ok(Json(json!({ "ok": true, "email": email, "syncEnabled": body.enabled })))
}

async fn admin_list_users(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT id, email, sync_enabled, created_at, email_verified, totp_enabled FROM users ORDER BY created_at")
        .map_err(internal)?;
    let users: Vec<serde_json::Value> = stmt
        .query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, String>(0)?,
                "email": r.get::<_, String>(1)?,
                "syncEnabled": r.get::<_, i64>(2)? != 0,
                "createdAt": r.get::<_, i64>(3)?,
                "emailVerified": r.get::<_, i64>(4)? != 0,
                "totpEnabled": r.get::<_, i64>(5)? != 0,
            }))
        })
        .map_err(internal)?
        .collect::<Result<_, _>>()
        .map_err(internal)?;
    Ok(Json(json!({ "users": users })))
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

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

#[tokio::main]
async fn main() {
    let cfg = PlatformConfig {
        admin_key: std::env::var("KITE_ADMIN_KEY").expect("必须设置 KITE_ADMIN_KEY"),
        feature_gating: matches!(
            env_or("KITE_FEATURE_GATING", "off").to_ascii_lowercase().as_str(),
            "on" | "1" | "true"
        ),
        registration_default: {
            let m = env_or("KITE_REGISTRATION", "open");
            if ["open", "invite", "closed"].contains(&m.as_str()) { m } else { "open".into() }
        },
        smtp: mail::Smtp::from_env(),
    };

    let conn = Connection::open(env_or("KITE_DB", "kite-server.db")).expect("打开数据库失败");
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            email TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            sync_enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tokens (
            token TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            expires_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS platform_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS invites (
            code TEXT PRIMARY KEY,
            note TEXT NOT NULL DEFAULT '',
            max_uses INTEGER NOT NULL DEFAULT 0,
            used_count INTEGER NOT NULL DEFAULT 0,
            expires_at INTEGER NOT NULL DEFAULT 0,
            disabled INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS invite_uses (
            code TEXT NOT NULL,
            user_id TEXT NOT NULL,
            used_at INTEGER NOT NULL
        );
        "#,
    )
    .expect("初始化表失败");

    // ---- 邮箱验证 / TOTP 迁移(存量库补列;email_verified 默认 1 —— 存量用户视为已验证)----
    add_column_if_missing(&conn, "users", "email_verified", "INTEGER NOT NULL DEFAULT 1");
    add_column_if_missing(&conn, "users", "totp_secret", "TEXT");
    add_column_if_missing(&conn, "users", "totp_enabled", "INTEGER NOT NULL DEFAULT 0");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS email_codes (
            user_id TEXT PRIMARY KEY, code TEXT NOT NULL, expires_at INTEGER NOT NULL, sent_at INTEGER NOT NULL
        );",
    ).expect("email_codes 建表失败");
    add_column_if_missing(&conn, "email_codes", "attempts", "INTEGER NOT NULL DEFAULT 0");

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

    let state = Arc::new(AppState {
        db: Mutex::new(conn),
        cfg,
        storage: std::sync::RwLock::new(storage_cfg),
        storage_source: Mutex::new(source),
        rate_hits: Mutex::new(HashMap::new()),
    });
    let app = Router::new()
        .route("/", get(admin_ui))
        .route("/admin", get(admin_ui))
        .route("/api/health", get(health))
        .route("/api/register", post(register))
        .route("/api/register/policy", get(register_policy))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/verify-email", post(verify_email))
        .route("/api/verify-email/resend", post(resend_verification))
        .route("/api/sync/credentials", get(sync_credentials))
        .route("/api/2fa/setup", post(totp_setup))
        .route("/api/2fa/enable", post(totp_enable))
        .route("/api/2fa/disable", post(totp_disable))
        .route("/api/features", get(features))
        .route("/api/admin/stats", get(admin_stats))
        .route("/api/admin/settings", get(admin_get_settings).post(admin_set_settings))
        .route("/api/admin/storage", get(admin_get_storage).post(admin_set_storage))
        .route("/api/admin/storage/test", post(admin_test_storage))
        .route("/api/admin/invites", get(admin_list_invites).post(admin_create_invite))
        .route("/api/admin/invites/{code}/disable", post(admin_disable_invite))
        .route("/api/admin/users", get(admin_list_users))
        .route("/api/admin/users/{email}/sync", post(admin_set_sync))
        .route("/api/admin/users/{email}/revoke", post(admin_revoke))
        .route("/api/admin/users/{email}/usage", get(admin_usage))
        .route("/api/admin/users/{email}/verify", post(admin_verify_user))
        .route("/api/admin/users/{email}/2fa/reset", post(admin_reset_totp))
        .with_state(state);

    let listen = env_or("KITE_LISTEN", "127.0.0.1:8787");
    println!("[server] listening on {listen}");
    let listener = tokio::net::TcpListener::bind(&listen).await.expect("bind 失败");
    // 需要 ConnectInfo 才能拿到客户端 IP 做注册限流
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .expect("serve 失败");
}
