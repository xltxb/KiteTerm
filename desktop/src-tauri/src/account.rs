//! 账户:对接管理平台(注册/登录/令牌/同步凭据)。
//! 令牌经设备密钥加密落 kv;主密码不经过平台(E2E 保持零知识)。

use std::sync::OnceLock;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::vault::Vault;

const KV_PLATFORM: &str = "platform_url";
const KV_TOKEN: &str = "account_token_enc";
const KV_ACCOUNT: &str = "account_info";
/// 正式发布指向的管理平台;本地开发用环境变量 `KITE_PLATFORM_URL` 覆盖
/// (如 `KITE_PLATFORM_URL=http://127.0.0.1:8787 pnpm tauri dev`),仅在用户尚未手动设置过地址时生效。
const DEFAULT_PLATFORM: &str = "https://m.kiteterm.com";
/// 单次平台请求的总超时;reqwest 默认不设超时,网络黑洞时登录按钮会永远转圈
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

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
    /// MinIO / 自建网关需开启;AWS S3 / R2 关闭(旧版平台未下发该字段时按开启处理)
    #[serde(default = "default_true")]
    pub path_style: bool,
}

/// 所有平台请求共用的 HTTP 客户端(连接池 + 超时)
pub fn client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .connect_timeout(CONNECT_TIMEOUT)
                .build()
                .expect("reqwest client")
        })
        .clone()
}

fn default_platform_url() -> String {
    std::env::var("KITE_PLATFORM_URL")
        .ok()
        .and_then(|v| normalize_platform_url(&v).ok())
        .unwrap_or_else(|| DEFAULT_PLATFORM.into())
}

pub fn platform_url(db: &Db) -> String {
    db.kv_get(KV_PLATFORM)
        .ok()
        .flatten()
        .unwrap_or_else(default_platform_url)
}

/// 校验并规范化用户输入的平台地址,返回落库的最终形式。
/// 之前只去尾部斜杠:没写协议会得到 reqwest 的 `relative URL without a base`,
/// 写了 `http://` 指向公网则密码与令牌明文传输 —— 这里都在保存时就拦住。
pub fn set_platform_url(db: &Db, url: &str) -> Result<String> {
    let normalized = normalize_platform_url(url)?;
    db.kv_set(KV_PLATFORM, &normalized)?;
    Ok(normalized)
}

fn is_loopback_host(url: &reqwest::Url) -> bool {
    use url::Host;
    match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost") || d.to_ascii_lowercase().ends_with(".localhost"),
        None => false,
    }
}

/// 规则:
/// - 去首尾空白与尾部 `/`;
/// - 没写协议按 https 补全(本机地址补 http,方便本地开发);
/// - 只接受 http/https;http 仅允许本机(127.0.0.1 / localhost / ::1);
/// - 丢弃 query 与 fragment。
pub fn normalize_platform_url(raw: &str) -> Result<String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(AppError("平台地址不能为空".into()));
    }
    let with_scheme = if s.contains("://") {
        s.to_string()
    } else {
        // 先按 https 解析出 host,再决定本机走 http
        let probe = reqwest::Url::parse(&format!("https://{s}"))
            .map_err(|_| AppError(format!("平台地址无法解析: {s}")))?;
        let scheme = if is_loopback_host(&probe) { "http" } else { "https" };
        format!("{scheme}://{s}")
    };
    let mut url = reqwest::Url::parse(&with_scheme)
        .map_err(|_| AppError(format!("平台地址无法解析: {s}")))?;
    if url.host_str().is_none() {
        return Err(AppError(format!("平台地址缺少主机名: {s}")));
    }
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_host(&url) => {}
        "http" => {
            return Err(AppError(
                "非本机地址必须使用 https://,否则密码与登录令牌会明文传输".into(),
            ))
        }
        other => {
            return Err(AppError(format!(
                "不支持的协议 {other}://,平台地址需以 https:// 开头"
            )))
        }
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
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
    let resp = client()
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
    let resp = client()
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
    let resp = client()
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
        let _ = client().post(&url).bearer_auth(t).send().await;
    }
    let conn = db.conn.lock().unwrap();
    conn.execute("DELETE FROM kv WHERE key IN (?1, ?2)", [KV_TOKEN, KV_ACCOUNT])?;
    Ok(())
}

/// 统一的带鉴权 POST 助手:令牌失效时清出 `LOGIN_REQUIRED` 错误串,交界面判断
async fn authed_post(db: &Db, vault: &Vault, path: &str, body: serde_json::Value) -> Result<serde_json::Value> {
    let t = token(db, vault)?;
    let resp = client()
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
    let resp = client()
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
    let resp = client()
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

#[cfg(test)]
mod tests {
    use super::normalize_platform_url as n;

    #[test]
    fn adds_https_and_strips_slash() {
        assert_eq!(n("m.kiteterm.com").unwrap(), "https://m.kiteterm.com");
        assert_eq!(n("  https://m.kiteterm.com/  ").unwrap(), "https://m.kiteterm.com");
        assert_eq!(n("https://m.kiteterm.com/kite/").unwrap(), "https://m.kiteterm.com/kite");
        assert_eq!(n("https://m.kiteterm.com/?x=1#f").unwrap(), "https://m.kiteterm.com");
    }

    #[test]
    fn loopback_may_use_http() {
        assert_eq!(n("http://127.0.0.1:8787").unwrap(), "http://127.0.0.1:8787");
        assert_eq!(n("127.0.0.1:8787").unwrap(), "http://127.0.0.1:8787");
        assert_eq!(n("localhost:8787").unwrap(), "http://localhost:8787");
        assert_eq!(n("http://[::1]:8787").unwrap(), "http://[::1]:8787");
    }

    #[test]
    fn rejects_plain_http_on_public_host() {
        let e = n("http://m.kiteterm.com").unwrap_err().0;
        assert!(e.contains("https"), "{e}");
        assert!(n("http://192.168.1.10:8787").is_err());
    }

    #[test]
    fn rejects_garbage() {
        assert!(n("").is_err());
        assert!(n("   ").is_err());
        assert!(n("ftp://m.kiteterm.com").is_err());
        assert!(n("https://").is_err());
        assert!(n("not a url").is_err());
    }
}
