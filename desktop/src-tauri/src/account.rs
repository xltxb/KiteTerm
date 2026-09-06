//! 账户:对接管理平台(注册/登录/令牌/同步凭据)。
//! 令牌经设备密钥加密落 kv;主密码不经过平台(E2E 保持零知识)。

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::vault::Vault;

const KV_PLATFORM: &str = "platform_url";
const KV_TOKEN: &str = "account_token_enc";
const KV_ACCOUNT: &str = "account_info";
const DEFAULT_PLATFORM: &str = "http://127.0.0.1:8787";

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub id: String,
    pub email: String,
    pub sync_enabled: bool,
    #[serde(default = "default_true")]
    pub email_verified: bool,
    #[serde(default)]
    pub totp_enabled: bool,
    /// 平台强制 2FA 且本账户尚未绑定 —— 同步会被拒,界面要引导绑定
    #[serde(default)]
    pub totp_setup_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformCreds {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    /// STS 临时凭据的会话令牌(平台开启 STS 时下发)
    #[serde(default)]
    pub session_token: Option<String>,
    pub prefix: String,
}

pub fn platform_url(db: &Db) -> String {
    db.kv_get(KV_PLATFORM)
        .ok()
        .flatten()
        .unwrap_or_else(|| DEFAULT_PLATFORM.into())
}

pub fn set_platform_url(db: &Db, url: &str) -> Result<()> {
    db.kv_set(KV_PLATFORM, url.trim_end_matches('/'))
}

pub fn current(db: &Db) -> Result<Option<AccountInfo>> {
    Ok(db
        .kv_get(KV_ACCOUNT)?
        .and_then(|j| serde_json::from_str(&j).ok()))
}

fn token(db: &Db, vault: &Vault) -> Result<String> {
    let enc = db
        .kv_get(KV_TOKEN)?
        .ok_or_else(|| AppError("LOGIN_REQUIRED: 请先登录账户".into()))?;
    let raw = vault.decrypt(
        &base64::engine::general_purpose::STANDARD
            .decode(enc)
            .map_err(|e| AppError(e.to_string()))?,
    )?;
    String::from_utf8(raw).map_err(|_| AppError("令牌解码失败".into()))
}

async fn api_error(resp: reqwest::Response) -> AppError {
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    let code = body["error"]["code"].as_str().unwrap_or("");
    let msg = body["error"]["message"].as_str().unwrap_or("请求失败");
    AppError(format!("{code}{}{msg}(HTTP {status})", if code.is_empty() { "" } else { ": " }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterPolicy {
    /// open | invite | closed
    pub mode: String,
    pub password_min_length: usize,
    pub password_rule: String,
}

/// 从注册/登录响应体的 `user`(及顶层 `totpSetupRequired`)解析账户信息
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
    db.kv_set(
        KV_TOKEN,
        &base64::engine::general_purpose::STANDARD.encode(vault.encrypt(token.as_bytes())?),
    )?;
    db.kv_set(KV_ACCOUNT, &serde_json::to_string(info)?)
}

pub async fn register_policy(db: &Db) -> Result<RegisterPolicy> {
    let url = format!("{}/api/register/policy", platform_url(db));
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError(format!("无法连接管理平台: {e}")))?;
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    resp.json().await.map_err(|e| AppError(e.to_string()))
}

/// 注册成功后平台直接签发令牌,这里顺带完成登录(免二次往返)
pub async fn register(
    db: &Db,
    vault: &Vault,
    email: &str,
    password: &str,
    invite_code: Option<&str>,
) -> Result<AccountInfo> {
    let url = format!("{}/api/register", platform_url(db));
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&json!({
            "email": email,
            "password": password,
            "invite_code": invite_code.unwrap_or_default(),
        }))
        .send()
        .await
        .map_err(|e| AppError(format!("无法连接管理平台: {e}")))?;
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| AppError(e.to_string()))?;
    let token = body["token"]
        .as_str()
        .ok_or_else(|| AppError("平台响应缺少令牌".into()))?;
    let info = info_from(&body);
    store_login(db, vault, token, &info)?;
    Ok(info)
}

pub async fn login(
    db: &Db,
    vault: &Vault,
    email: &str,
    password: &str,
    totp: Option<&str>,
) -> Result<AccountInfo> {
    let url = format!("{}/api/login", platform_url(db));
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&json!({ "email": email, "password": password, "totp": totp.unwrap_or_default() }))
        .send()
        .await
        .map_err(|e| AppError(format!("无法连接管理平台: {e}")))?;
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| AppError(e.to_string()))?;
    let token = body["token"]
        .as_str()
        .ok_or_else(|| AppError("平台响应缺少令牌".into()))?;
    let info = info_from(&body);
    store_login(db, vault, token, &info)?;
    Ok(info)
}

pub async fn logout(db: &Db, vault: &Vault) -> Result<()> {
    if let Ok(t) = token(db, vault) {
        let url = format!("{}/api/logout", platform_url(db));
        let _ = reqwest::Client::new().post(&url).bearer_auth(t).send().await;
    }
    let conn = db.conn.lock().unwrap();
    conn.execute("DELETE FROM kv WHERE key IN (?1, ?2)", [KV_TOKEN, KV_ACCOUNT])?;
    Ok(())
}

/// 统一的带鉴权 POST 助手:令牌失效时清出 `LOGIN_REQUIRED` 错误串,交界面判断
async fn authed_post(db: &Db, vault: &Vault, path: &str, body: serde_json::Value) -> Result<serde_json::Value> {
    let t = token(db, vault)?;
    let resp = reqwest::Client::new()
        .post(format!("{}{path}", platform_url(db)))
        .bearer_auth(t)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError(format!("无法连接管理平台: {e}")))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError("LOGIN_REQUIRED: 登录已失效,请重新登录".into()));
    }
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    resp.json().await.map_err(|e| AppError(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpSetup {
    pub secret: String,
    pub otpauth_url: String,
}

pub async fn verify_email(db: &Db, vault: &Vault, code: &str) -> Result<AccountInfo> {
    authed_post(db, vault, "/api/verify-email", json!({ "code": code })).await?;
    refresh(db, vault).await
}

pub async fn resend_verification(db: &Db, vault: &Vault) -> Result<()> {
    authed_post(db, vault, "/api/verify-email/resend", json!({}))
        .await
        .map(|_| ())
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
        .get(format!("{}/api/me", platform_url(db)))
        .bearer_auth(t)
        .send()
        .await
        .map_err(|e| AppError(format!("无法连接管理平台: {e}")))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        let conn = db.conn.lock().unwrap();
        conn.execute("DELETE FROM kv WHERE key IN (?1, ?2)", [KV_TOKEN, KV_ACCOUNT])?;
        return Err(AppError(
            "LOGIN_REQUIRED: 登录已失效(可能在其它设备登录或被管理员下线),请重新登录".into(),
        ));
    }
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
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

/// 向平台索取同步凭据(登录 + 平台放开同步时才会成功)
pub async fn sync_credentials(db: &Db, vault: &Vault) -> Result<PlatformCreds> {
    let t = token(db, vault)?;
    let url = format!("{}/api/sync/credentials", platform_url(db));
    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(t)
        .send()
        .await
        .map_err(|e| AppError(format!("无法连接管理平台: {e}")))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError("LOGIN_REQUIRED: 登录已失效,请重新登录".into()));
    }
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    let creds: PlatformCreds = resp.json().await.map_err(|e| AppError(e.to_string()))?;
    // 同步账户状态缓存
    if let Ok(Some(mut info)) = current(db) {
        info.sync_enabled = true;
        let _ = db.kv_set(KV_ACCOUNT, &serde_json::to_string(&info)?);
    }
    Ok(creds)
}
