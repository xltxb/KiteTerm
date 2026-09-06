//! STS AssumeRole(SigV4 手工签名)— 向 S3 兼容存储(MinIO 等)换取
//! 按用户前缀收敛的临时凭据。凭据带 session policy:仅允许操作
//! arn:aws:s3:::{bucket}/{prefix}users/{uid}/* 与对应前缀的 ListBucket。

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub struct TempCreds {
    pub access_key: String,
    pub secret_key: String,
    pub session_token: String,
    pub expiration: String,
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// RFC3986 百分号编码(表单值用)
fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn session_policy(bucket: &str, prefix: &str, user_id: &str) -> String {
    serde_json::json!({
        "Version": "2012-10-17",
        "Statement": [
            {
                "Effect": "Allow",
                "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"],
                "Resource": [format!("arn:aws:s3:::{bucket}/{prefix}users/{user_id}/*")]
            },
            {
                "Effect": "Allow",
                "Action": ["s3:ListBucket"],
                "Resource": [format!("arn:aws:s3:::{bucket}")],
                "Condition": { "StringLike": { "s3:prefix": [format!("{prefix}users/{user_id}/*")] } }
            }
        ]
    })
    .to_string()
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
}

/// 调用 STS AssumeRole,返回限权临时凭据;endpoint 为 S3 服务根地址
pub async fn assume_role_for_user(
    endpoint: &str,
    region: &str,
    access_key: &str,
    secret_key: &str,
    bucket: &str,
    prefix: &str,
    user_id: &str,
    duration_secs: u32,
) -> Result<TempCreds, String> {
    let policy = session_policy(bucket, prefix, user_id);
    let body = format!(
        "Action=AssumeRole&Version=2011-06-15&DurationSeconds={duration_secs}\
         &RoleArn={}&RoleSessionName={}&Policy={}",
        urlencode("arn:aws:iam::000000000000:role/vela-sync"),
        urlencode(&format!("kite-{user_id}")),
        urlencode(&policy),
    );

    let host = endpoint
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_string();
    let now = chrono::Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = now.format("%Y%m%d").to_string();
    let payload_hash = sha256_hex(body.as_bytes());

    let canonical = format!(
        "POST\n/\n\ncontent-type:application/x-www-form-urlencoded\nhost:{host}\nx-amz-date:{amz_date}\n\ncontent-type;host;x-amz-date\n{payload_hash}"
    );
    let scope = format!("{date}/{region}/sts/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical.as_bytes())
    );
    let k_date = hmac(format!("AWS4{secret_key}").as_bytes(), &date);
    let k_region = hmac(&k_date, region);
    let k_service = hmac(&k_region, "sts");
    let k_signing = hmac(&k_service, "aws4_request");
    let signature = hex::encode(hmac(&k_signing, &string_to_sign));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders=content-type;host;x-amz-date, Signature={signature}"
    );

    let resp = reqwest::Client::new()
        .post(format!("{}/", endpoint.trim_end_matches('/')))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("x-amz-date", amz_date)
        .header("authorization", authorization)
        .body(body)
        .send()
        .await
        .map_err(|e| format!("STS 请求失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("STS HTTP {status}: {}", text.chars().take(300).collect::<String>()));
    }
    Ok(TempCreds {
        access_key: extract_tag(&text, "AccessKeyId").ok_or("STS 响应缺少 AccessKeyId")?,
        secret_key: extract_tag(&text, "SecretAccessKey").ok_or("STS 响应缺少 SecretAccessKey")?,
        session_token: extract_tag(&text, "SessionToken").ok_or("STS 响应缺少 SessionToken")?,
        expiration: extract_tag(&text, "Expiration").unwrap_or_default(),
    })
}
