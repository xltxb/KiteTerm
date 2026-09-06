//! S3 存储配置:运行期可改,DB 优先、环境变量兜底(仅作初值)。
//! 密钥明文落 SQLite(与此前明文放环境变量的强度一致),DB 文件权限由部署方保证。

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const KEY: &str = "storage_config";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StorageConfig {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    /// 空或形如 `prod/`(规范化后必以 `/` 结尾)
    #[serde(default)]
    pub prefix: String,
    pub access_key: String,
    pub secret_key: String,
    /// MinIO / 自建网关需开启;AWS S3 / R2 关闭
    #[serde(default = "default_true")]
    pub path_style: bool,
    /// auto | require | off
    #[serde(default = "default_sts")]
    pub sts_mode: String,
}

fn default_true() -> bool { true }
fn default_sts() -> String { "auto".into() }

/// `prod` / `/prod/` / `prod//` → `prod/`;空 → 空
pub fn normalize_prefix(raw: &str) -> String {
    let t = raw.trim().trim_matches('/');
    if t.is_empty() { String::new() } else { format!("{t}/") }
}

/// 归一化后的前缀是否只含安全字符:字母、数字、`.` `_` `-` `/`,且不含 `..` 或 `\`。
/// 空前缀视为合法(表示不加前缀)。
pub fn prefix_valid(p: &str) -> bool {
    if p.is_empty() {
        return true;
    }
    if p.contains("..") || p.contains('\\') {
        return false;
    }
    p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}

impl StorageConfig {
    pub fn from_env() -> Self {
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        Self {
            endpoint: env("S3_ENDPOINT"),
            region: { let r = env("S3_REGION"); if r.is_empty() { "us-east-1".into() } else { r } },
            bucket: { let b = env("S3_BUCKET"); if b.is_empty() { "vela-sync".into() } else { b } },
            prefix: normalize_prefix(&env("S3_PREFIX")),
            access_key: env("S3_ACCESS_KEY"),
            secret_key: env("S3_SECRET_KEY"),
            path_style: !matches!(env("S3_PATH_STYLE").to_ascii_lowercase().as_str(), "off" | "0" | "false"),
            sts_mode: { let m = env("KITE_STS"); if m.is_empty() { "auto".into() } else { m } },
        }
    }

    pub fn normalized(mut self) -> Self {
        self.endpoint = self.endpoint.trim().trim_end_matches('/').to_string();
        self.region = self.region.trim().to_string();
        self.bucket = self.bucket.trim().to_string();
        self.prefix = normalize_prefix(&self.prefix);
        self.access_key = self.access_key.trim().to_string();
        if !["auto", "require", "off"].contains(&self.sts_mode.as_str()) {
            self.sts_mode = "auto".into();
        }
        self
    }

    pub fn is_configured(&self) -> bool {
        !self.endpoint.is_empty() && !self.bucket.is_empty() && !self.access_key.is_empty()
    }

    pub fn user_prefix(&self, user_id: &str) -> String {
        format!("{}users/{user_id}", self.prefix)
    }

    pub fn bucket(&self) -> Result<Box<s3::Bucket>, String> {
        let region = s3::Region::Custom { region: self.region.clone(), endpoint: self.endpoint.clone() };
        let creds = s3::creds::Credentials::new(Some(&self.access_key), Some(&self.secret_key), None, None, None)
            .map_err(|e| e.to_string())?;
        let b = s3::Bucket::new(&self.bucket, region, creds).map_err(|e| e.to_string())?;
        Ok(if self.path_style { b.with_path_style() } else { b })
    }

    /// 对外形态:secret 只回传"是否已设置"
    pub fn public_json(&self, source: &str) -> Value {
        json!({
            "endpoint": self.endpoint, "region": self.region, "bucket": self.bucket,
            "prefix": self.prefix, "accessKey": self.access_key,
            "hasSecret": !self.secret_key.is_empty(),
            "pathStyle": self.path_style, "stsMode": self.sts_mode,
            "configured": self.is_configured(), "source": source,
        })
    }
}

/// DB 有就用 DB(source=db),否则环境变量(source=env)
pub fn load(conn: &Connection) -> (StorageConfig, &'static str) {
    let from_db = crate::setting_get(conn, KEY, "");
    if !from_db.is_empty() {
        if let Ok(c) = serde_json::from_str::<StorageConfig>(&from_db) {
            return (c.normalized(), "db");
        }
    }
    (StorageConfig::from_env().normalized(), "env")
}

pub fn save(conn: &Connection, cfg: &StorageConfig) -> rusqlite::Result<()> {
    crate::setting_set(conn, KEY, &serde_json::to_string(cfg).unwrap_or_default())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReport {
    pub ok: bool,
    pub latency_ms: u128,
    pub list: Option<String>,
    pub put: Option<String>,
    pub delete: Option<String>,
    pub sts: Option<String>,
}

/// 探测:ListBucket → Put 探针 → Delete 探针;开启 STS 时再试一次 AssumeRole。
/// 每项 None 表示通过,Some(msg) 表示失败原因。
pub async fn probe(cfg: &StorageConfig) -> ProbeReport {
    let t0 = std::time::Instant::now();
    let mut r = ProbeReport { ok: false, latency_ms: 0, list: None, put: None, delete: None, sts: None };
    let bucket = match cfg.bucket() {
        Ok(b) => b,
        Err(e) => {
            // bucket 都建不起来,put/delete(以及要求的 sts)根本没跑,
            // 别让它们的 None 被控制台渲染成 ✓
            r.put = Some("未执行".into());
            r.delete = Some("未执行".into());
            if cfg.sts_mode != "off" {
                r.sts = Some("未执行".into());
            }
            r.list = Some(e);
            r.latency_ms = t0.elapsed().as_millis();
            return r;
        }
    };
    if let Err(e) = bucket.list(cfg.prefix.clone(), Some("/".into())).await {
        r.list = Some(e.to_string());
    }
    let probe_key = format!("{}.kite-probe", cfg.prefix);
    match bucket.put_object(&probe_key, b"probe").await {
        Ok(_) => {
            if let Err(e) = bucket.delete_object(&probe_key).await { r.delete = Some(e.to_string()); }
        }
        Err(e) => r.put = Some(e.to_string()),
    }
    if cfg.sts_mode != "off" {
        if let Err(e) = crate::sts::assume_role_for_user(
            &cfg.endpoint, &cfg.region, &cfg.access_key, &cfg.secret_key, &cfg.bucket, &cfg.prefix, "probe", 900,
        ).await {
            r.sts = Some(e);
        }
    }
    r.latency_ms = t0.elapsed().as_millis();
    r.ok = r.list.is_none() && r.put.is_none() && r.delete.is_none() && (cfg.sts_mode != "require" || r.sts.is_none());
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_normalization() {
        assert_eq!(normalize_prefix(""), "");
        assert_eq!(normalize_prefix("prod"), "prod/");
        assert_eq!(normalize_prefix("/prod//"), "prod/");
        assert!(prefix_valid(""));
        assert!(prefix_valid("prod/v2"));
        assert!(!prefix_valid("../evil"));
        assert!(!prefix_valid("a\\b"));
        let c = StorageConfig { endpoint: "http://x/".into(), region: "r".into(), bucket: "b".into(), prefix: "p".into(),
            access_key: "a".into(), secret_key: "s".into(), path_style: true, sts_mode: "weird".into() }.normalized();
        assert_eq!(c.endpoint, "http://x");
        assert_eq!(c.user_prefix("u1"), "p/users/u1");
        assert_eq!(c.sts_mode, "auto");
        assert_eq!(c.public_json("db")["hasSecret"], json!(true));
        assert!(c.public_json("db").get("secretKey").is_none());
    }

    #[test]
    fn db_roundtrip_and_env_fallback() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE platform_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)").unwrap();
        let (_, src) = load(&conn);
        assert_eq!(src, "env");
        let c = StorageConfig { endpoint: "http://m".into(), region: "r".into(), bucket: "b".into(), prefix: "".into(),
            access_key: "a".into(), secret_key: "s".into(), path_style: false, sts_mode: "off".into() };
        save(&conn, &c).unwrap();
        let (got, src) = load(&conn);
        assert_eq!(src, "db");
        assert_eq!(got, c);
    }
}
