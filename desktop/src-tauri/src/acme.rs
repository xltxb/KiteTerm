//! ACME 证书:签发/续期/部署状态机与续期守护。纯函数在文件末尾单测。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::Handle;
use russh_sftp::client::SftpSession;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use instant_acme::{
    Account, AuthorizationStatus, ChallengeType, Identifier, LetsEncrypt, NewAccount, NewOrder,
    OrderStatus, RetryPolicy, ZeroSsl,
};
use rcgen::{CertificateParams, KeyPair};
use x509_parser::pem::parse_x509_pem;

use crate::db::{CertRow, Db};
use crate::error::{AppError, Result};
use crate::models::{CertInfo, IssueRequest};
use crate::ssh::{self, ClientHandler, SessionManager};
use crate::vault::Vault;

/// HTTP-01 校验文件默认写入的 Web 根目录。
/// IssueRequest 目前没有 webroot 字段(该模型被前端复用,不改签名),
/// 因此取常见的 Nginx/Apache 默认站点根;如需自定义,后续可给 IssueRequest 增补字段。
#[allow(dead_code)]
const DEFAULT_WEBROOT: &str = "/var/www/html";
/// reload_cmd 为空时的默认服务重载命令
#[allow(dead_code)]
const DEFAULT_RELOAD_CMD: &str = "sudo nginx -s reload";
/// DNS-01 手动确认的等待超时(秒)
#[allow(dead_code)]
const DNS_CONFIRM_TIMEOUT_SECS: u64 = 300;
/// 订单/证书轮询的超时(秒)
#[allow(dead_code)]
const POLL_TIMEOUT_SECS: u64 = 120;

/// 跟踪进行中的签发/续期任务;续期守护单独 spawn
#[derive(Default)]
#[allow(dead_code)]
pub struct CertManager {
    pub jobs: Mutex<HashMap<String, JoinHandle<()>>>,
    /// DNS-01 手动校验的等待器:cert_id → Notify,由 confirm_dns 唤醒
    pub dns_waiters: Mutex<HashMap<String, Arc<Notify>>>,
}

/// 发送五步进度事件。事件名固定为 `cert://progress`,payload 携带 certId。
#[allow(dead_code)]
fn emit_progress(app: &AppHandle, cert_id: &str, step: Step, state: &str, error: Option<&str>) {
    let _ = app.emit(
        "cert://progress",
        serde_json::json!({
            "certId": cert_id,
            "step": step.as_index(),
            "label": step.label(),
            "state": state,
            "error": error,
        }),
    );
}

/// 唤醒等待 DNS-01 手动确认的签发流程
#[allow(dead_code)]
pub fn confirm_dns(manager: &CertManager, cert_id: &str) {
    let waiter = manager.dns_waiters.lock().unwrap().get(cert_id).cloned();
    if let Some(notify) = waiter {
        notify.notify_one();
    }
}

/// 失败时落库为 error 状态(尽力而为,忽略落库自身的错误)
#[allow(dead_code)]
fn upsert_error(db: &Db, cert_id: &str, req: &IssueRequest, created_at: &str) {
    let row = CertRow {
        id: cert_id.to_string(),
        domains: req.domains.clone(),
        challenge: req.challenge.clone(),
        ca: req.ca.clone(),
        host_id: req.host_id.clone(),
        deploy_path: req.deploy_path.clone(),
        reload_cmd: req.reload_cmd.clone(),
        not_after: 0,
        auto_renew: true,
        state: "error".to_string(),
        account_key_enc: None,
        cert_pem: None,
        chain_pem: None,
        created_at: created_at.to_string(),
    };
    let _ = db.upsert_cert(&row);
}

/// 从 deploy_path 推导同目录下另一文件的完整路径
#[allow(dead_code)]
fn sibling_path(path: &str, name: &str) -> String {
    match path.rfind('/') {
        Some(i) => format!("{}/{}", &path[..i], name),
        None => name.to_string(),
    }
}

/// 取证书链 PEM 中的第一段(叶证书)
#[allow(dead_code)]
fn first_cert_pem(chain: &str) -> String {
    const END: &str = "-----END CERTIFICATE-----";
    match chain.find(END) {
        Some(idx) => format!("{}\n", chain[..idx + END.len()].trim_start()),
        None => chain.to_string(),
    }
}

/// 解析叶证书 PEM 的 not_after,返回毫秒时间戳
#[allow(dead_code)]
fn parse_not_after_ms(chain_pem: &str) -> Result<i64> {
    let (_, pem) = parse_x509_pem(chain_pem.as_bytes())
        .map_err(|e| AppError(format!("解析证书 PEM 失败: {e}")))?;
    let cert = pem
        .parse_x509()
        .map_err(|e| AppError(format!("解析 X.509 失败: {e}")))?;
    Ok(cert.validity().not_after.timestamp() * 1000)
}

/// 复用 sftp.rs 的开会话方式,创建并写入一个远端文件
#[allow(dead_code)]
async fn sftp_upload(handle: &Handle<ClientHandler>, path: &str, data: &[u8]) -> Result<()> {
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| AppError(format!("SFTP 通道失败: {e}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| AppError(format!("SFTP 子系统失败: {e}")))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| AppError(format!("SFTP 会话失败: {e}")))?;
    let mut file = sftp
        .create(path)
        .await
        .map_err(|e| AppError(format!("创建远端 {path} 失败: {e}")))?;
    file.write_all(data)
        .await
        .map_err(|e| AppError(format!("写入远端 {path} 失败: {e}")))?;
    file.shutdown().await.ok();
    Ok(())
}

/// ACME 签发状态机:HTTP-01 经 SSH 放置校验文件,DNS-01 手动等待确认,五步进度事件。
///
/// 步骤:Place(账户/订单/放置校验)→ Verify(CA 校验)→ Sign(签发)→ Upload(上传)→ Reload(重载)。
#[allow(dead_code)]
pub async fn issue(
    app: AppHandle,
    db: Arc<Db>,
    vault: Arc<Vault>,
    sessions: Arc<SessionManager>,
    manager: Arc<CertManager>,
    req: IssueRequest,
) -> Result<CertInfo> {
    // 0. 门禁:必须有活动会话,否则不做任何 ACME 网络工作
    sessions
        .get_handle(&req.host_id)
        .map_err(|_| AppError("ERR_NO_SESSION".to_string()))?;

    let cert_id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();

    // 任一可失败步骤:出错则发 error 事件、落库 error、返回 Err。
    // 宏在此定义,可引用外层的 app/cert_id/db/req/created_at。
    macro_rules! step_try {
        ($step:expr, $expr:expr) => {
            match $expr {
                Ok(v) => v,
                Err(e) => {
                    let msg = e.to_string();
                    emit_progress(&app, &cert_id, $step, "error", Some(&msg));
                    upsert_error(&db, &cert_id, &req, &created_at);
                    return Err(AppError(msg));
                }
            }
        };
    }
    // 非 Result 的显式失败(命令未成功、超时等)
    macro_rules! step_fail {
        ($step:expr, $msg:expr) => {{
            let msg = $msg;
            emit_progress(&app, &cert_id, $step, "error", Some(&msg));
            upsert_error(&db, &cert_id, &req, &created_at);
            return Err(AppError(msg));
        }};
    }

    // 1. Place:开始
    emit_progress(&app, &cert_id, Step::Place, "running", None);

    let want_dns =
        req.challenge.eq_ignore_ascii_case("dns-01") || req.challenge.eq_ignore_ascii_case("dns");

    // 2. ACME 账户(ZeroSSL 或 Let's Encrypt 生产环境目录)
    let directory_url = if req.ca.eq_ignore_ascii_case("zerossl") {
        ZeroSsl::Production.url().to_string()
    } else {
        LetsEncrypt::Production.url().to_string()
    };
    let new_account = NewAccount {
        contact: &[],
        terms_of_service_agreed: true,
        only_return_existing: false,
    };
    let (account, credentials) = step_try!(Step::Place, {
        async {
            let builder = Account::builder()?;
            builder
                .create(&new_account, directory_url.clone(), None)
                .await
        }
        .await
    });
    // 账户凭据(含账户私钥)序列化后经 vault 加密备用
    let cred_json = step_try!(Step::Place, serde_json::to_vec(&credentials));
    let account_key_enc = step_try!(Step::Place, vault.encrypt(&cred_json));

    // 3. 新建订单
    let identifiers: Vec<Identifier> =
        req.domains.iter().map(|d| Identifier::Dns(d.clone())).collect();
    let mut order = step_try!(
        Step::Place,
        account.new_order(&NewOrder::new(&identifiers)).await
    );

    // 4. 逐个 authorization 放置校验响应并标记 ready
    {
        let mut authzs = order.authorizations();
        while let Some(authz_res) = authzs.next().await {
            let mut authz = step_try!(Step::Place, authz_res);
            // 已验证过的 identifier 直接跳过
            if authz.status == AuthorizationStatus::Valid {
                continue;
            }

            if want_dns {
                // DNS-01(手动):emit 记录并等待 confirm_dns
                let mut ch = step_try!(
                    Step::Place,
                    authz
                        .challenge(ChallengeType::Dns01)
                        .ok_or_else(|| AppError("服务器未提供 DNS-01 校验".to_string()))
                );
                let value = ch.key_authorization().dns_value();
                let ident = ch.identifier();
                let base_domain = match ident.identifier {
                    Identifier::Dns(ref d) => d.clone(),
                    _ => ident.to_string(),
                };
                let record_name = format!("_acme-challenge.{base_domain}");

                // 注册等待器后再 emit,避免 confirm_dns 早于等待
                let notify = Arc::new(Notify::new());
                manager
                    .dns_waiters
                    .lock()
                    .unwrap()
                    .insert(cert_id.clone(), notify.clone());
                let _ = app.emit(
                    "cert://dns-record",
                    serde_json::json!({
                        "certId": cert_id,
                        "name": record_name,
                        "value": value,
                    }),
                );

                // 等待用户在 DNS 处添加 TXT 记录并确认(带超时)
                let waited = tokio::time::timeout(
                    Duration::from_secs(DNS_CONFIRM_TIMEOUT_SECS),
                    notify.notified(),
                )
                .await;
                if waited.is_err() {
                    manager.dns_waiters.lock().unwrap().remove(&cert_id);
                    step_fail!(Step::Place, "DNS-01 校验等待超时".to_string());
                }
                step_try!(Step::Place, ch.set_ready().await);
            } else {
                // HTTP-01:经 SSH 写入 <webroot>/.well-known/acme-challenge/<token>
                let mut ch = step_try!(
                    Step::Place,
                    authz
                        .challenge(ChallengeType::Http01)
                        .ok_or_else(|| AppError("服务器未提供 HTTP-01 校验".to_string()))
                );
                let key_auth = ch.key_authorization();
                let token = ch.token.clone();
                let dir = format!("{DEFAULT_WEBROOT}/.well-known/acme-challenge");
                let file_path = format!("{DEFAULT_WEBROOT}{}", challenge_path(&token));
                let cmd = format!(
                    "mkdir -p '{}' && printf %s '{}' > '{}' && echo __ACME_OK__",
                    dir,
                    key_auth.as_str(),
                    file_path
                );
                let handle = step_try!(Step::Place, sessions.get_handle(&req.host_id));
                let out = step_try!(Step::Place, ssh::exec_capture(&handle, &cmd).await);
                if !out.contains("__ACME_OK__") {
                    step_fail!(
                        Step::Place,
                        format!("放置 HTTP-01 校验文件失败: {}", out.trim())
                    );
                }
                step_try!(Step::Place, ch.set_ready().await);
            }
        }
    }
    // 清理可能残留的等待器
    manager.dns_waiters.lock().unwrap().remove(&cert_id);
    emit_progress(&app, &cert_id, Step::Place, "ok", None);

    // 5. Verify:等待订单变为 ready
    emit_progress(&app, &cert_id, Step::Verify, "running", None);
    let ready_policy = RetryPolicy::default().timeout(Duration::from_secs(POLL_TIMEOUT_SECS));
    let status = step_try!(Step::Verify, order.poll_ready(&ready_policy).await);
    if status != OrderStatus::Ready {
        step_fail!(Step::Verify, format!("订单校验未通过: {status:?}"));
    }
    emit_progress(&app, &cert_id, Step::Verify, "ok", None);

    // 6. Sign:rcgen 生成密钥与 CSR,finalize,拉取证书链,解析 not_after
    emit_progress(&app, &cert_id, Step::Sign, "running", None);
    let key_pair = step_try!(Step::Sign, KeyPair::generate());
    let params = step_try!(Step::Sign, CertificateParams::new(req.domains.clone()));
    let csr = step_try!(Step::Sign, params.serialize_request(&key_pair));
    step_try!(Step::Sign, order.finalize_csr(csr.der()).await);
    let cert_policy = RetryPolicy::default().timeout(Duration::from_secs(POLL_TIMEOUT_SECS));
    let chain_pem = step_try!(Step::Sign, order.poll_certificate(&cert_policy).await);
    let privkey_pem = key_pair.serialize_pem();
    let not_after_ms = step_try!(Step::Sign, parse_not_after_ms(&chain_pem));
    let leaf_pem = first_cert_pem(&chain_pem);
    emit_progress(&app, &cert_id, Step::Sign, "ok", None);

    // 7. Upload:SFTP 上传证书链到 deploy_path,私钥到同目录 privkey.pem
    emit_progress(&app, &cert_id, Step::Upload, "running", None);
    let handle = step_try!(Step::Upload, sessions.get_handle(&req.host_id));
    let privkey_path = sibling_path(&req.deploy_path, "privkey.pem");
    step_try!(
        Step::Upload,
        sftp_upload(&handle, &req.deploy_path, chain_pem.as_bytes()).await
    );
    step_try!(
        Step::Upload,
        sftp_upload(&handle, &privkey_path, privkey_pem.as_bytes()).await
    );
    emit_progress(&app, &cert_id, Step::Upload, "ok", None);

    // 8. Reload:重载服务
    emit_progress(&app, &cert_id, Step::Reload, "running", None);
    let reload_cmd = if req.reload_cmd.trim().is_empty() {
        DEFAULT_RELOAD_CMD.to_string()
    } else {
        req.reload_cmd.clone()
    };
    let handle = step_try!(Step::Reload, sessions.get_handle(&req.host_id));
    let reload_full = format!("{reload_cmd} && echo __RELOAD_OK__");
    let out = step_try!(Step::Reload, ssh::exec_capture(&handle, &reload_full).await);
    if !out.contains("__RELOAD_OK__") {
        step_fail!(Step::Reload, format!("服务重载失败: {}", out.trim()));
    }
    emit_progress(&app, &cert_id, Step::Reload, "ok", None);

    // 9. 成功落库(state=active),返回 CertInfo
    let row = CertRow {
        id: cert_id.clone(),
        domains: req.domains.clone(),
        challenge: req.challenge.clone(),
        ca: req.ca.clone(),
        host_id: req.host_id.clone(),
        deploy_path: req.deploy_path.clone(),
        reload_cmd: req.reload_cmd.clone(),
        not_after: not_after_ms,
        auto_renew: true,
        state: "active".to_string(),
        account_key_enc: Some(account_key_enc),
        cert_pem: Some(leaf_pem),
        chain_pem: Some(chain_pem),
        created_at: created_at.clone(),
    };
    db.upsert_cert(&row)?;

    Ok(CertInfo {
        id: cert_id,
        domains: req.domains,
        challenge: req.challenge,
        ca: req.ca,
        host_id: req.host_id,
        deploy_path: req.deploy_path,
        reload_cmd: req.reload_cmd,
        not_after: not_after_ms,
        auto_renew: true,
        state: "active".to_string(),
        created_at,
    })
}

#[allow(dead_code)]
pub fn renew_due(not_after_ms: i64, now_ms: i64) -> bool {
    // 距到期 ≤ 30 天(含已过期)即应续期
    not_after_ms - now_ms <= 30 * 24 * 60 * 60 * 1000
}

#[allow(dead_code)]
pub fn days_left(not_after_ms: i64, now_ms: i64) -> i64 {
    (not_after_ms - now_ms) / (24 * 60 * 60 * 1000)
}

#[allow(dead_code)]
pub fn challenge_path(token: &str) -> String {
    format!("/.well-known/acme-challenge/{token}")
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum Step {
    Place,
    Verify,
    Sign,
    Upload,
    Reload,
}

#[allow(dead_code)]
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

    #[test]
    fn sibling_path_replaces_filename() {
        assert_eq!(
            sibling_path("/etc/nginx/ssl/example.com/fullchain.pem", "privkey.pem"),
            "/etc/nginx/ssl/example.com/privkey.pem"
        );
        assert_eq!(sibling_path("fullchain.pem", "privkey.pem"), "privkey.pem");
    }

    #[test]
    fn first_cert_pem_takes_leaf() {
        let chain = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\nBBBB\n-----END CERTIFICATE-----\n";
        let leaf = first_cert_pem(chain);
        assert!(leaf.contains("AAAA"));
        assert!(!leaf.contains("BBBB"));
    }
}
