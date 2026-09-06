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
