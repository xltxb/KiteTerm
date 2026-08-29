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
//!   S3_ENDPOINT / S3_BUCKET / S3_REGION / S3_ACCESS_KEY / S3_SECRET_KEY

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
use serde::{Deserialize, Serialize};
use serde_json::json;

mod sts;

struct PlatformConfig {
    admin_key: String,
    s3_endpoint: String,
    s3_bucket: String,
    s3_region: String,
    s3_access_key: String,
    s3_secret_key: String,
    /// STS 模式:auto(默认,失败回退共享凭据)| require | off
    sts_mode: String,
    /// 注册策略初值(仅在 DB 尚无设置时生效)
    registration_default: String,
    /// 功能门禁总闸(KITE_FEATURE_GATING,默认 off)。
    ///
    /// off:登录/云同步/AI 助手三项开关全部失效 —— 客户端不隐藏任何入口,
    ///     服务端也不据此拦截。平台只当账户与凭据的中枢,不管功能可见性。
    /// on: 三项开关生效,由管理控制台逐项开关并下发。
    ///
    /// 之所以做成部署配置而不是控制台开关:它决定的是这套管控要不要存在,
    /// 属于部署形态,不该被日常运维误点。
    feature_gating: bool,
}

struct AppState {
    db: Mutex<Connection>,
    cfg: PlatformConfig,
    /// 注册限流:IP → 最近注册尝试的时间戳(滑动窗口)
    register_hits: Mutex<HashMap<String, Vec<i64>>>,
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

fn setting_get(conn: &Connection, key: &str, default: &str) -> String {
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

fn setting_set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
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

/// 滑动窗口限流;超限返回 false
fn rate_limit_ok(state: &AppState, ip: &str) -> bool {
    let now = now_ts();
    let mut hits = state.register_hits.lock().unwrap();
    let entry = hits.entry(ip.to_string()).or_default();
    entry.retain(|t| now - *t < REGISTER_WINDOW_SECS);
    if entry.len() >= register_limit() {
        return false;
    }
    entry.push(now);
    // 顺带清理其它 IP 的过期记录,避免无限增长
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UserOut {
    id: String,
    email: String,
    sync_enabled: bool,
}

#[derive(Deserialize)]
struct Credentials {
    email: String,
    password: String,
    /// 仅 invite 模式需要
    #[serde(default)]
    invite_code: Option<String>,
}

struct AuthedUser {
    id: String,
    email: String,
    sync_enabled: bool,
}

fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<AuthedUser, ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "unauthorized", "缺少令牌"))?;
    let conn = state.db.lock().unwrap();
    let row: Option<(String, String, i64, i64)> = conn
        .query_row(
            "SELECT u.id, u.email, u.sync_enabled, t.expires_at
             FROM tokens t JOIN users u ON u.id = t.user_id WHERE t.token = ?1",
            [token],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(internal)?;
    match row {
        Some((id, email, sync_enabled, expires_at)) if expires_at > now_ts() => Ok(AuthedUser {
            id,
            email,
            sync_enabled: sync_enabled != 0,
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
/// 单项开关的原始值(不看总闸)—— 管理控制台展示用
fn feature_raw(conn: &Connection, feature: &str) -> bool {
    setting_get(conn, &format!("feature_{feature}"), "1") == "1"
}

/// 生效值:总闸关闭时一律视为开启 —— 不启用门禁就等于什么都不限制
fn feature_on(state: &AppState, conn: &Connection, feature: &str) -> bool {
    !state.cfg.feature_gating || feature_raw(conn, feature)
}

/// 客户端据此决定哪些入口可见。**免鉴权** —— 客户端未登录时也要能拿到,
/// 否则「登录开关」自身就没法下发。
async fn features(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let conn = state.db.lock().unwrap();
    Ok(Json(json!({
        // 注册策略是平台自己的准入政策,与功能门禁无关,总闸关掉也照样生效
        "registration": setting_get(&conn, "registration_mode", &state.cfg.registration_default),
        "enforced": state.cfg.feature_gating,
        "login": feature_on(&state, &conn, "login"),
        "sync": feature_on(&state, &conn, "sync"),
        "ai": feature_on(&state, &conn, "ai"),
        "passwordMinLength": PASSWORD_MIN_LEN,
        "passwordRule": "至少 8 位,且包含字母、数字、符号中的至少两类",
    })))
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
    if !rate_limit_ok(&state, &ip) {
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
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO users (id, email, password_hash, sync_enabled, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, email, hash, if default_sync == "1" { 1 } else { 0 }, now_ts()],
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
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "userId": id,
            "token": token,
            "expiresAt": expires_at,
            "user": { "id": id, "email": email, "syncEnabled": sync_enabled },
        })),
    ))
}

async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Credentials>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let email = body.email.trim().to_lowercase();
    let conn = state.db.lock().unwrap();
    let row: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT id, password_hash, sync_enabled FROM users WHERE email = ?1",
            [&email],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(internal)?;
    let Some((user_id, hash, sync_enabled)) = row else {
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
    if !feature_on(&state, &conn, "login") {
        return Err(err(StatusCode::FORBIDDEN, "LOGIN_DISABLED", "平台当前已关闭登录"));
    }
    let (token, expires_at) = issue_token(&conn, &user_id)?;
    Ok(Json(json!({
        "token": token,
        "expiresAt": expires_at,
        "user": { "id": user_id, "email": email, "syncEnabled": sync_enabled != 0 },
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
) -> Result<Json<UserOut>, ApiError> {
    let user = authenticate(&state, &headers)?;
    Ok(Json(UserOut { id: user.id, email: user.email, sync_enabled: user.sync_enabled }))
}

/// 平台管控的同步凭据:登录且账户同步已放开时下发。
/// 优先经 STS 签发按 users/{id}/* 收敛的限权临时凭据(1h);
/// STS 不可用时回退共享凭据 + 前缀约定(KITE_STS=require 则直接报错)。
async fn sync_credentials(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = authenticate(&state, &headers)?;
    {
        let conn = state.db.lock().unwrap();
        if !feature_on(&state, &conn, "sync") {
            return Err(err(
                StatusCode::FORBIDDEN,
                "SYNC_DISABLED",
                "平台当前已关闭云同步",
            ));
        }
    }
    if !user.sync_enabled {
        return Err(err(
            StatusCode::FORBIDDEN,
            "SYNC_DISABLED",
            "该账户的云同步已被平台停用,请联系管理员",
        ));
    }
    let prefix = format!("users/{}", user.id);
    if state.cfg.sts_mode != "off" {
        match sts::assume_role_for_user(
            &state.cfg.s3_endpoint,
            &state.cfg.s3_region,
            &state.cfg.s3_access_key,
            &state.cfg.s3_secret_key,
            &state.cfg.s3_bucket,
            &user.id,
            3600,
        )
        .await
        {
            Ok(t) => {
                return Ok(Json(json!({
                    "endpoint": state.cfg.s3_endpoint,
                    "bucket": state.cfg.s3_bucket,
                    "region": state.cfg.s3_region,
                    "accessKey": t.access_key,
                    "secretKey": t.secret_key,
                    "sessionToken": t.session_token,
                    "expiration": t.expiration,
                    "prefix": prefix,
                })));
            }
            Err(e) if state.cfg.sts_mode == "require" => {
                return Err(err(StatusCode::BAD_GATEWAY, "sts_error", &e));
            }
            Err(e) => eprintln!("[server] STS 失败,回退共享凭据: {e}"),
        }
    }
    Ok(Json(json!({
        "endpoint": state.cfg.s3_endpoint,
        "bucket": state.cfg.s3_bucket,
        "region": state.cfg.s3_region,
        "accessKey": state.cfg.s3_access_key,
        "secretKey": state.cfg.s3_secret_key,
        "prefix": prefix,
    })))
}

fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let key = headers.get("x-admin-key").and_then(|v| v.to_str().ok()).unwrap_or("");
    if key.is_empty() || key != state.cfg.admin_key {
        return Err(err(StatusCode::UNAUTHORIZED, "unauthorized", "管理密钥错误"));
    }
    Ok(())
}

async fn admin_stats(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
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
        "bucket": state.cfg.s3_bucket,
        "endpoint": state.cfg.s3_endpoint,
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

fn platform_bucket(cfg: &PlatformConfig) -> Result<Box<s3::Bucket>, ApiError> {
    let region = s3::Region::Custom { region: cfg.s3_region.clone(), endpoint: cfg.s3_endpoint.clone() };
    let creds = s3::creds::Credentials::new(
        Some(&cfg.s3_access_key),
        Some(&cfg.s3_secret_key),
        None,
        None,
        None,
    )
    .map_err(internal)?;
    Ok(s3::Bucket::new(&cfg.s3_bucket, region, creds)
        .map_err(internal)?
        .with_path_style())
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
    let bucket = platform_bucket(&state.cfg)?;
    let pages = bucket
        .list(format!("users/{user_id}/"), None)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, "s3_error", &e.to_string()))?;
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
    json!({
        "registrationMode": setting_get(conn, "registration_mode", &state.cfg.registration_default),
        "defaultSyncEnabled": setting_get(conn, "default_sync_enabled", "1") == "1",
        "featureGating": state.cfg.feature_gating,
        "featureLogin": feature_raw(conn, "login"),
        "featureSync": feature_raw(conn, "sync"),
        "featureAi": feature_raw(conn, "ai"),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminSettingsBody {
    registration_mode: Option<String>,
    default_sync_enabled: Option<bool>,
    feature_login: Option<bool>,
    feature_sync: Option<bool>,
    feature_ai: Option<bool>,
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
    for (feature, value) in [
        ("login", body.feature_login),
        ("sync", body.feature_sync),
        ("ai", body.feature_ai),
    ] {
        if let Some(on) = value {
            setting_set(&conn, &format!("feature_{feature}"), if on { "1" } else { "0" })
                .map_err(internal)?;
        }
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
    let key = headers.get("x-admin-key").and_then(|v| v.to_str().ok()).unwrap_or("");
    if key.is_empty() || key != state.cfg.admin_key {
        return Err(err(StatusCode::UNAUTHORIZED, "unauthorized", "管理密钥错误"));
    }
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT id, email, sync_enabled, created_at FROM users ORDER BY created_at")
        .map_err(internal)?;
    let users: Vec<serde_json::Value> = stmt
        .query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, String>(0)?,
                "email": r.get::<_, String>(1)?,
                "syncEnabled": r.get::<_, i64>(2)? != 0,
                "createdAt": r.get::<_, i64>(3)?,
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

#[tokio::main]
async fn main() {
    let cfg = PlatformConfig {
        admin_key: std::env::var("KITE_ADMIN_KEY").expect("必须设置 KITE_ADMIN_KEY"),
        s3_endpoint: std::env::var("S3_ENDPOINT").expect("必须设置 S3_ENDPOINT"),
        s3_bucket: env_or("S3_BUCKET", "vela-sync"),
        s3_region: env_or("S3_REGION", "us-east-1"),
        s3_access_key: std::env::var("S3_ACCESS_KEY").expect("必须设置 S3_ACCESS_KEY"),
        s3_secret_key: std::env::var("S3_SECRET_KEY").expect("必须设置 S3_SECRET_KEY"),
        sts_mode: env_or("KITE_STS", "auto"),
        feature_gating: matches!(
            env_or("KITE_FEATURE_GATING", "off").to_ascii_lowercase().as_str(),
            "on" | "1" | "true"
        ),
        registration_default: {
            let m = env_or("KITE_REGISTRATION", "open");
            if ["open", "invite", "closed"].contains(&m.as_str()) { m } else { "open".into() }
        },
    };

    // 平台负责保证桶存在
    {
        let region = s3::Region::Custom { region: cfg.s3_region.clone(), endpoint: cfg.s3_endpoint.clone() };
        let creds = s3::creds::Credentials::new(
            Some(&cfg.s3_access_key),
            Some(&cfg.s3_secret_key),
            None,
            None,
            None,
        )
        .expect("S3 凭据无效");
        match s3::Bucket::create_with_path_style(&cfg.s3_bucket, region, creds, Default::default()).await {
            Ok(_) => println!("[server] 桶 {} 已就绪", cfg.s3_bucket),
            Err(e) => println!("[server] 建桶跳过(可能已存在): {e}"),
        }
    }

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

    let state = Arc::new(AppState {
        db: Mutex::new(conn),
        cfg,
        register_hits: Mutex::new(HashMap::new()),
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
        .route("/api/sync/credentials", get(sync_credentials))
        .route("/api/features", get(features))
        .route("/api/admin/stats", get(admin_stats))
        .route("/api/admin/settings", get(admin_get_settings).post(admin_set_settings))
        .route("/api/admin/invites", get(admin_list_invites).post(admin_create_invite))
        .route("/api/admin/invites/{code}/disable", post(admin_disable_invite))
        .route("/api/admin/users", get(admin_list_users))
        .route("/api/admin/users/{email}/sync", post(admin_set_sync))
        .route("/api/admin/users/{email}/revoke", post(admin_revoke))
        .route("/api/admin/users/{email}/usage", get(admin_usage))
        .with_state(state);

    let listen = env_or("KITE_LISTEN", "127.0.0.1:8787");
    println!("[server] listening on {listen}");
    let listener = tokio::net::TcpListener::bind(&listen).await.expect("bind 失败");
    // 需要 ConnectInfo 才能拿到客户端 IP 做注册限流
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .expect("serve 失败");
}
