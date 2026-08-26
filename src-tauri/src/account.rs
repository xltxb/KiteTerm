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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub id: String,
    pub email: String,
    pub sync_enabled: bool,
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

pub async fn register(db: &Db, email: &str, password: &str) -> Result<()> {
    let url = format!("{}/api/register", platform_url(db));
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&json!({ "email": email, "password": password }))
        .send()
        .await
        .map_err(|e| AppError(format!("无法连接管理平台: {e}")))?;
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    Ok(())
}

pub async fn login(db: &Db, vault: &Vault, email: &str, password: &str) -> Result<AccountInfo> {
    let url = format!("{}/api/login", platform_url(db));
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&json!({ "email": email, "password": password }))
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
    let info = AccountInfo {
        id: body["user"]["id"].as_str().unwrap_or_default().to_string(),
        email: body["user"]["email"].as_str().unwrap_or_default().to_string(),
        sync_enabled: body["user"]["syncEnabled"].as_bool().unwrap_or(false),
    };
    db.kv_set(
        KV_TOKEN,
        &base64::engine::general_purpose::STANDARD.encode(vault.encrypt(token.as_bytes())?),
    )?;
    db.kv_set(KV_ACCOUNT, &serde_json::to_string(&info)?)?;
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
