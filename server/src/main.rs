//! Vela Term 管理平台
//!
//! 职责:账户注册/登录(Argon2id 口令哈希 + 不透明令牌)、同步授权开关、
//! 平台持有的 S3 桶凭据下发(按用户前缀 users/{user_id} 隔离)。
//! 平台不接触用户主密码 —— 桶内仍是客户端主密钥加密的密文,平台零知识。
//!
//! 配置(环境变量):
//!   VELA_LISTEN       默认 127.0.0.1:8787
//!   VELA_DB           默认 ./vela-server.db
//!   VELA_ADMIN_KEY    管理接口密钥(必填)
//!   S3_ENDPOINT / S3_BUCKET / S3_REGION / S3_ACCESS_KEY / S3_SECRET_KEY

use std::sync::{Arc, Mutex};

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

struct PlatformConfig {
    admin_key: String,
    s3_endpoint: String,
    s3_bucket: String,
    s3_region: String,
    s3_access_key: String,
    s3_secret_key: String,
}

struct AppState {
    db: Mutex<Connection>,
    cfg: PlatformConfig,
}

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
    Json(json!({ "ok": true, "service": "vela-server" }))
}

async fn register(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Credentials>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let email = body.email.trim().to_lowercase();
    if !email.contains('@') || email.len() < 5 {
        return Err(err(StatusCode::BAD_REQUEST, "invalid_email", "邮箱格式不正确"));
    }
    if body.password.len() < 8 {
        return Err(err(StatusCode::BAD_REQUEST, "weak_password", "密码至少 8 位"));
    }
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(internal)?
        .to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let conn = state.db.lock().unwrap();
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO users (id, email, password_hash, sync_enabled, created_at)
         VALUES (?1, ?2, ?3, 1, ?4)",
        params![id, email, hash, now_ts()],
    )
    .map_err(internal)?;
    if inserted == 0 {
        return Err(err(StatusCode::CONFLICT, "email_taken", "该邮箱已注册"));
    }
    Ok((StatusCode::CREATED, Json(json!({ "userId": id }))))
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
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let expires_at = now_ts() + 30 * 24 * 3600;
    conn.execute(
        "INSERT INTO tokens (token, user_id, expires_at) VALUES (?1, ?2, ?3)",
        params![token, user_id, expires_at],
    )
    .map_err(internal)?;
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

/// 平台管控的同步凭据:登录且账户同步已放开时,下发平台桶 + 该用户前缀。
/// v1 为共享凭据 + 前缀约定;换 STS 临时凭据为后续硬化项。
async fn sync_credentials(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = authenticate(&state, &headers)?;
    if !user.sync_enabled {
        return Err(err(
            StatusCode::FORBIDDEN,
            "SYNC_DISABLED",
            "该账户的云同步已被平台停用,请联系管理员",
        ));
    }
    Ok(Json(json!({
        "endpoint": state.cfg.s3_endpoint,
        "bucket": state.cfg.s3_bucket,
        "region": state.cfg.s3_region,
        "accessKey": state.cfg.s3_access_key,
        "secretKey": state.cfg.s3_secret_key,
        "prefix": format!("users/{}", user.id),
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
        admin_key: std::env::var("VELA_ADMIN_KEY").expect("必须设置 VELA_ADMIN_KEY"),
        s3_endpoint: std::env::var("S3_ENDPOINT").expect("必须设置 S3_ENDPOINT"),
        s3_bucket: env_or("S3_BUCKET", "vela-sync"),
        s3_region: env_or("S3_REGION", "us-east-1"),
        s3_access_key: std::env::var("S3_ACCESS_KEY").expect("必须设置 S3_ACCESS_KEY"),
        s3_secret_key: std::env::var("S3_SECRET_KEY").expect("必须设置 S3_SECRET_KEY"),
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

    let conn = Connection::open(env_or("VELA_DB", "vela-server.db")).expect("打开数据库失败");
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
        "#,
    )
    .expect("初始化表失败");

    let state = Arc::new(AppState { db: Mutex::new(conn), cfg });
    let app = Router::new()
        .route("/", get(admin_ui))
        .route("/admin", get(admin_ui))
        .route("/api/health", get(health))
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/sync/credentials", get(sync_credentials))
        .route("/api/admin/stats", get(admin_stats))
        .route("/api/admin/users", get(admin_list_users))
        .route("/api/admin/users/{email}/sync", post(admin_set_sync))
        .route("/api/admin/users/{email}/revoke", post(admin_revoke))
        .route("/api/admin/users/{email}/usage", get(admin_usage))
        .with_state(state);

    let listen = env_or("VELA_LISTEN", "127.0.0.1:8787");
    println!("[server] listening on {listen}");
    let listener = tokio::net::TcpListener::bind(&listen).await.expect("bind 失败");
    axum::serve(listener, app).await.expect("serve 失败");
}
