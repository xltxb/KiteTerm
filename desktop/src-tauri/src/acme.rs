//! ACME 证书:签发/续期/部署状态机与续期守护。纯函数在文件末尾单测。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::Handle;
use russh_sftp::client::SftpSession;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;

use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, LetsEncrypt,
    NewAccount, NewOrder, OrderStatus, RetryPolicy, RevocationRequest, ZeroSsl,
};
use rcgen::{CertificateParams, KeyPair};
use rustls_pki_types::CertificateDer;
use x509_parser::pem::parse_x509_pem;

use crate::db::{CertRow, Db};
use crate::error::{AppError, Result};
use crate::models::{CertInfo, IssueRequest};
use crate::ssh::{self, ClientHandler, SessionManager};
use crate::vault::Vault;

/// HTTP-01 校验文件默认写入的 Web 根目录。
/// IssueRequest 目前没有 webroot 字段(该模型被前端复用,不改签名),
/// 因此取常见的 Nginx/Apache 默认站点根;如需自定义,后续可给 IssueRequest 增补字段。
const DEFAULT_WEBROOT: &str = "/var/www/html";
/// reload_cmd 为空时的默认服务重载命令
const DEFAULT_RELOAD_CMD: &str = "sudo nginx -s reload";
/// DNS-01 手动确认的等待超时(秒)
const DNS_CONFIRM_TIMEOUT_SECS: u64 = 300;
/// 订单/证书轮询的超时(秒)
const POLL_TIMEOUT_SECS: u64 = 120;

/// 跟踪进行中的签发/续期任务;续期守护单独 spawn
#[derive(Default)]
pub struct CertManager {
    /// DNS-01 手动校验的等待器:cert_id → Notify,由 confirm_dns 唤醒
    pub dns_waiters: Mutex<HashMap<String, Arc<Notify>>>,
}

/// 发送五步进度事件。事件名固定为 `cert://progress`,payload 携带 certId。
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
pub fn confirm_dns(manager: &CertManager, cert_id: &str) {
    let waiter = manager.dns_waiters.lock().unwrap().get(cert_id).cloned();
    if let Some(notify) = waiter {
        notify.notify_one();
    }
}

/// 失败时落库为 error 状态(尽力而为,忽略落库自身的错误)。
/// 若该 cert_id 已存在(续期失败场景),保留原有到期时间与证书材料,仅改 state,
/// 避免一次失败的续期把仍然有效的证书数据清空。
///
/// 已是 error 状态时跳过落库:该函数只改 state 一个字段,状态未变即整行内容未变,
/// 而 upsert_cert 会写含完整 cert_pem/chain_pem + 两段密文的 oplog —— 域名校验失败、
/// CA 限流、SFTP/reload 出错等每一步失败都会走到这里,若不判重,续不动的证书每次
/// 尝试(含续期守护每 12h 一轮)都会无上界地产生一条 oplog。
fn upsert_error(db: &Db, cert_id: &str, req: &IssueRequest, created_at: &str) {
    let row = match db.get_cert_full(cert_id) {
        Ok(mut existing) => {
            if existing.state == "error" {
                return;
            }
            existing.state = "error".to_string();
            existing
        }
        Err(_) => CertRow {
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
            key_pem_enc: None,
            owner_device_id: db.device_id(),
            created_at: created_at.to_string(),
        },
    };
    let _ = db.upsert_cert(&row);
}

/// 从 deploy_path 推导同目录下另一文件的完整路径
fn sibling_path(path: &str, name: &str) -> String {
    match path.rfind('/') {
        Some(i) => format!("{}/{}", &path[..i], name),
        None => name.to_string(),
    }
}

/// 取证书链 PEM 中的第一段(叶证书)
fn first_cert_pem(chain: &str) -> String {
    const END: &str = "-----END CERTIFICATE-----";
    match chain.find(END) {
        Some(idx) => format!("{}\n", chain[..idx + END.len()].trim_start()),
        None => chain.to_string(),
    }
}

/// 解析叶证书 PEM 的 not_after,返回毫秒时间戳
fn parse_not_after_ms(chain_pem: &str) -> Result<i64> {
    let (_, pem) = parse_x509_pem(chain_pem.as_bytes())
        .map_err(|e| AppError(format!("解析证书 PEM 失败: {e}")))?;
    let cert = pem
        .parse_x509()
        .map_err(|e| AppError(format!("解析 X.509 失败: {e}")))?;
    Ok(cert.validity().not_after.timestamp() * 1000)
}

/// 复用 sftp.rs 的开会话方式,创建并写入一个远端文件
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

/// ACME 签发:生成新 cert_id 后跑签发内核。首次签发入口。
///
/// 步骤:Place(账户/订单/放置校验)→ Verify(CA 校验)→ Sign(签发)→ Upload(上传)→ Reload(重载)。
pub async fn issue(
    app: AppHandle,
    db: Arc<Db>,
    vault: Arc<Vault>,
    sessions: Arc<SessionManager>,
    manager: Arc<CertManager>,
    req: IssueRequest,
) -> Result<CertInfo> {
    let cert_id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    run_issue(app, db, vault, sessions, manager, req, cert_id, created_at).await
}

/// 签发内核:issue(新 cert_id)与 renew(沿用已存 cert_id/created_at)共用,避免重复五步流程。
///
/// 步骤:Place(账户/订单/放置校验)→ Verify(CA 校验)→ Sign(签发)→ Upload(上传)→ Reload(重载)。
#[allow(clippy::too_many_arguments)]
async fn run_issue(
    app: AppHandle,
    db: Arc<Db>,
    vault: Arc<Vault>,
    sessions: Arc<SessionManager>,
    manager: Arc<CertManager>,
    req: IssueRequest,
    cert_id: String,
    created_at: String,
) -> Result<CertInfo> {
    // 0. 门禁:必须有活动会话,否则不做任何 ACME 网络工作
    sessions
        .get_handle(&req.host_id)
        .map_err(|_| AppError("ERR_NO_SESSION".to_string()))?;

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
                // 用 {{ ...; }} 2>&1 包住两段命令,把 stderr 折进被捕获的 stdout;
                // 标记仍只在命令组退出 0 时 echo,失败检测不变,但错误消息可带出 stderr 原因
                let cmd = format!(
                    "{{ mkdir -p '{}' && printf %s '{}' > '{}'; }} 2>&1 && echo __ACME_OK__",
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
    // 2>&1 绑定到 reload_cmd,stderr 折进 stdout;标记仍只在退出 0 时 echo
    let reload_full = format!("{reload_cmd} 2>&1 && echo __RELOAD_OK__");
    let out = step_try!(Step::Reload, ssh::exec_capture(&handle, &reload_full).await);
    if !out.contains("__RELOAD_OK__") {
        step_fail!(Step::Reload, format!("服务重载失败: {}", out.trim()));
    }
    emit_progress(&app, &cert_id, Step::Reload, "ok", None);

    // 9. 成功落库(state=active),返回 CertInfo
    // 私钥 PEM 经 vault 加密后落库,供后续 deploy 重新上传(含部署到其他主机)
    let key_pem_enc = step_try!(Step::Sign, vault.encrypt(privkey_pem.as_bytes()));
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
        key_pem_enc: Some(key_pem_enc),
        owner_device_id: db.device_id(),
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
        owner_device_id: db.device_id(),
        created_at,
    })
}

/// 从已存 CertRow 重建 IssueRequest(纯函数,便于单测续期参数还原)
pub fn req_from_row(row: &CertRow) -> IssueRequest {
    IssueRequest {
        domains: row.domains.clone(),
        challenge: row.challenge.clone(),
        ca: row.ca.clone(),
        host_id: row.host_id.clone(),
        deploy_path: row.deploy_path.clone(),
        reload_cmd: row.reload_cmd.clone(),
    }
}

/// 续期:用库中已存参数重建 IssueRequest 后重跑签发内核,沿用原 cert_id 与 created_at 就地更新。
/// 要求该 host 有活跃会话(内核门禁 get_handle → ERR_NO_SESSION)。
pub async fn renew(
    app: AppHandle,
    db: Arc<Db>,
    vault: Arc<Vault>,
    sessions: Arc<SessionManager>,
    manager: Arc<CertManager>,
    cert_id: &str,
) -> Result<CertInfo> {
    let row = db.get_cert_full(cert_id)?;
    let req = req_from_row(&row);
    run_issue(app, db, vault, sessions, manager, req, row.id, row.created_at).await
}

/// 尽力发起 ACME 网络吊销(需账户凭据 + 叶证书)。失败由调用方忽略。
async fn try_acme_revoke(vault: &Vault, acct_enc: &[u8], leaf_pem: &str) -> Result<()> {
    // 还原账户凭据(vault 解密 → AccountCredentials)
    let cred_json = vault.decrypt(acct_enc)?;
    let credentials: AccountCredentials = serde_json::from_slice(&cred_json)
        .map_err(|e| AppError(format!("解析账户凭据失败: {e}")))?;
    let account = Account::builder()
        .map_err(|e| AppError(format!("构建 ACME 账户失败: {e}")))?
        .from_credentials(credentials)
        .await
        .map_err(|e| AppError(format!("恢复 ACME 账户失败: {e}")))?;
    // 叶证书 PEM → DER
    let (_, pem) = parse_x509_pem(leaf_pem.as_bytes())
        .map_err(|e| AppError(format!("解析叶证书 PEM 失败: {e}")))?;
    let der = CertificateDer::from(pem.contents);
    account
        .revoke(&RevocationRequest {
            certificate: &der,
            reason: None,
        })
        .await
        .map_err(|e| AppError(format!("ACME 吊销请求失败: {e}")))?;
    Ok(())
}

/// 吊销:尽力调 ACME revoke(有账户凭据 + 叶证书时),网络吊销失败仅记录、不阻断,最终始终本地删除。
pub async fn revoke(db: Arc<Db>, vault: Arc<Vault>, cert_id: &str) -> Result<()> {
    if let Ok(row) = db.get_cert_full(cert_id) {
        if let (Some(acct_enc), Some(leaf_pem)) =
            (row.account_key_enc.as_ref(), row.cert_pem.as_ref())
        {
            if let Err(e) = try_acme_revoke(&vault, acct_enc, leaf_pem).await {
                // 尽力而为:网络吊销失败不影响本地删除
                eprintln!("ACME 吊销失败(忽略,继续本地删除): {e}");
            }
        }
    }
    db.delete_cert(cert_id)
}

/// 部署/重新部署:取已存证书链与私钥(vault 解密),门禁目标 host 会话,SFTP 上传 + 重载。
/// 支持把同一张证书部署到 host_id 指定的任意主机(deploy-to-other-host)。
///
/// 注:私钥经 vault 加密存于 key_pem_enc,故本函数比 brief 里的签名多需 `vault` 参数用于解密;
/// 否则无法拿到私钥,也就无法部署到其他主机。
pub async fn deploy(
    app: AppHandle,
    db: Arc<Db>,
    vault: Arc<Vault>,
    sessions: Arc<SessionManager>,
    cert_id: &str,
    host_id: &str,
) -> Result<()> {
    let row = db.get_cert_full(cert_id)?;
    let chain_pem = row
        .chain_pem
        .clone()
        .ok_or_else(|| AppError("证书链缺失,无法部署".to_string()))?;
    let key_enc = row
        .key_pem_enc
        .clone()
        .ok_or_else(|| AppError("私钥缺失,无法部署".to_string()))?;
    let privkey_pem = vault.decrypt(&key_enc)?;

    // 门禁:目标 host 必须有活跃会话
    let handle = sessions
        .get_handle(host_id)
        .map_err(|_| AppError("ERR_NO_SESSION".to_string()))?;

    // Upload:证书链到 deploy_path,私钥到同目录 privkey.pem
    emit_progress(&app, cert_id, Step::Upload, "running", None);
    let privkey_path = sibling_path(&row.deploy_path, "privkey.pem");
    if let Err(e) = sftp_upload(&handle, &row.deploy_path, chain_pem.as_bytes()).await {
        let msg = e.to_string();
        emit_progress(&app, cert_id, Step::Upload, "error", Some(&msg));
        return Err(AppError(msg));
    }
    if let Err(e) = sftp_upload(&handle, &privkey_path, &privkey_pem).await {
        let msg = e.to_string();
        emit_progress(&app, cert_id, Step::Upload, "error", Some(&msg));
        return Err(AppError(msg));
    }
    emit_progress(&app, cert_id, Step::Upload, "ok", None);

    // Reload:重载服务
    emit_progress(&app, cert_id, Step::Reload, "running", None);
    let reload_cmd = if row.reload_cmd.trim().is_empty() {
        DEFAULT_RELOAD_CMD.to_string()
    } else {
        row.reload_cmd.clone()
    };
    let reload_full = format!("{reload_cmd} 2>&1 && echo __RELOAD_OK__");
    let out = match ssh::exec_capture(&handle, &reload_full).await {
        Ok(o) => o,
        Err(e) => {
            let msg = e.to_string();
            emit_progress(&app, cert_id, Step::Reload, "error", Some(&msg));
            return Err(AppError(msg));
        }
    };
    if !out.contains("__RELOAD_OK__") {
        let msg = format!("服务重载失败: {}", out.trim());
        emit_progress(&app, cert_id, Step::Reload, "error", Some(&msg));
        return Err(AppError(msg));
    }
    emit_progress(&app, cert_id, Step::Reload, "ok", None);
    Ok(())
}

/// 续期守护:仿 ssh::spawn_latency_loop。每 12h 扫全部证书,对开启自动续期且进入续期窗口
/// (≤30 天到期)者尝试续期。单张证书失败只落状态(仍有效→expiring,已过期→error)并 emit,
/// 绝不 panic 整个任务;成功由 renew 就地更新为 active。
pub fn spawn_renew_loop(
    app: AppHandle,
    db: Arc<Db>,
    vault: Arc<Vault>,
    sessions: Arc<SessionManager>,
    manager: Arc<CertManager>,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(12 * 3600));
        loop {
            ticker.tick().await;
            let now_ms = chrono::Utc::now().timestamp_millis();
            let certs = db.list_certs().unwrap_or_default();
            let this_device = db.device_id();
            for c in certs {
                if !c.auto_renew
                    || !renew_owned_by(&c.owner_device_id, &this_device)
                    || !renew_due(c.not_after, now_ms)
                {
                    continue;
                }
                let res = renew(
                    app.clone(),
                    db.clone(),
                    vault.clone(),
                    sessions.clone(),
                    manager.clone(),
                    &c.id,
                )
                .await;
                if let Err(e) = res {
                    // 续期失败:仍有效标 expiring,已过期标 error;保留既有材料(get_cert_full)。
                    // 目标状态与现有状态相同时跳过落库,否则续不动的证书会每 12h 写一条 oplog、无上界。
                    if let Ok(mut row) = db.get_cert_full(&c.id) {
                        if let Some(next_state) =
                            renew_failure_next_state(&row.state, row.not_after, now_ms)
                        {
                            row.state = next_state.to_string();
                            let _ = db.upsert_cert(&row);
                        }
                    }
                    emit_progress(&app, &c.id, Step::Sign, "error", Some(&e.to_string()));
                }
            }
        }
    });
}

pub fn renew_due(not_after_ms: i64, now_ms: i64) -> bool {
    // 距到期 ≤ 30 天(含已过期)即应续期
    not_after_ms - now_ms <= 30 * 24 * 60 * 60 * 1000
}

/// 自动续期归属判定:只有归属设备才跑续期,避免多设备对同一域名重复签发
/// (Let's Encrypt 对同一组域名有「每周 5 张重复证书」的限额)。
/// 归属为空 = 存量证书尚未认领,由本机接手。
pub fn renew_owned_by(owner_device_id: &str, this_device: &str) -> bool {
    owner_device_id.is_empty() || owner_device_id == this_device
}

/// 续期失败时的落库判据:算出目标状态(仍有效→expiring,已过期→error),
/// 与现有状态相同则返回 None(跳过落库),不同才返回 Some(目标状态)。
/// 避免一张续不动的证书每 12h 都写一条含证书材料的 oplog、无上界地产生同步噪音。
fn renew_failure_next_state(
    current_state: &str,
    not_after_ms: i64,
    now_ms: i64,
) -> Option<&'static str> {
    let target = if not_after_ms > now_ms {
        "expiring"
    } else {
        "error"
    };
    if current_state == target {
        None
    } else {
        Some(target)
    }
}

/// 仅供单测使用(前端自行计算剩余天数);保留窄化 allow 而非放宽整模块
#[allow(dead_code)]
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

    #[test]
    fn sibling_path_replaces_filename() {
        assert_eq!(
            sibling_path("/etc/nginx/ssl/example.com/fullchain.pem", "privkey.pem"),
            "/etc/nginx/ssl/example.com/privkey.pem"
        );
        assert_eq!(sibling_path("fullchain.pem", "privkey.pem"), "privkey.pem");
    }

    #[test]
    fn req_from_row_roundtrips_params() {
        let row = CertRow {
            id: "cid".into(),
            domains: vec!["a.example.com".into(), "b.example.com".into()],
            challenge: "http-01".into(),
            ca: "letsencrypt".into(),
            host_id: "host-1".into(),
            deploy_path: "/etc/nginx/ssl/fullchain.pem".into(),
            reload_cmd: "sudo nginx -s reload".into(),
            not_after: 123,
            auto_renew: true,
            state: "active".into(),
            account_key_enc: None,
            cert_pem: None,
            chain_pem: None,
            key_pem_enc: None,
            owner_device_id: "dev-test".into(),
            created_at: "2026-01-01 00:00".into(),
        };
        let req = req_from_row(&row);
        assert_eq!(req.domains, row.domains);
        assert_eq!(req.challenge, "http-01");
        assert_eq!(req.ca, "letsencrypt");
        assert_eq!(req.host_id, "host-1");
        assert_eq!(req.deploy_path, "/etc/nginx/ssl/fullchain.pem");
        assert_eq!(req.reload_cmd, "sudo nginx -s reload");
    }

    #[test]
    fn first_cert_pem_takes_leaf() {
        let chain = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\nBBBB\n-----END CERTIFICATE-----\n";
        let leaf = first_cert_pem(chain);
        assert!(leaf.contains("AAAA"));
        assert!(!leaf.contains("BBBB"));
    }

    #[test]
    fn only_owner_device_auto_renews() {
        assert!(renew_owned_by("dev-a", "dev-a"), "归属本机应续期");
        assert!(!renew_owned_by("dev-b", "dev-a"), "归属他机不应续期");
        assert!(renew_owned_by("", "dev-a"), "归属为空视为未认领,本机接手");
    }

    #[test]
    fn renew_failure_next_state_dedupes_repeat_failures() {
        let now = 1_000_000_000_000;
        let not_after_expired = now - 86_400_000; // 已过期
        let not_after_expiring = now + 10 * 86_400_000; // 仍有效,进入续期窗口

        // 第一次失败:active -> error,应写
        assert_eq!(
            renew_failure_next_state("active", not_after_expired, now),
            Some("error")
        );
        // 同一状态重复续期失败:已是 error,再次失败不应重复写
        assert_eq!(
            renew_failure_next_state("error", not_after_expired, now),
            None
        );

        // 未过期分支同理:active -> expiring 应写,expiring -> expiring 应跳过
        assert_eq!(
            renew_failure_next_state("active", not_after_expiring, now),
            Some("expiring")
        );
        assert_eq!(
            renew_failure_next_state("expiring", not_after_expiring, now),
            None
        );
    }

    fn scratch_db(name: &str) -> Db {
        let dir = std::env::temp_dir().join("kite-acme-test").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Db::open(dir).unwrap()
    }

    fn sample_req() -> IssueRequest {
        IssueRequest {
            domains: vec!["a.example.com".into()],
            challenge: "http-01".into(),
            ca: "letsencrypt".into(),
            host_id: "h1".into(),
            deploy_path: "/etc/ssl".into(),
            reload_cmd: "nginx -s reload".into(),
        }
    }

    #[test]
    fn upsert_error_skips_oplog_when_already_error() {
        let db = scratch_db("upsert_error_dedup");
        let req = sample_req();

        // 首次失败(cert_id 尚不存在):应新建一行并产生一条 oplog
        upsert_error(&db, "c1", &req, "2026-09-20");
        let ops = db.oplog_after(0).unwrap();
        assert_eq!(ops.len(), 1, "首次失败应写一条 oplog");
        assert_eq!(db.get_cert_full("c1").unwrap().state, "error");

        // 同一证书再次失败(已经是 error):不应产生新的 oplog 条目
        upsert_error(&db, "c1", &req, "2026-09-20");
        let ops = db.oplog_after(0).unwrap();
        assert_eq!(ops.len(), 1, "同一状态重复失败不应重复写 oplog");
    }
}
