use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostGroup {
    pub id: String,
    pub name: String,
    pub sort: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub port: u16,
    pub user: String,
    pub group_id: String,
    pub tags: Vec<String>,
    pub key_id: Option<String>,
    pub last_connected_at: Option<String>,
    pub latency_ms: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInput {
    pub id: Option<String>,
    pub name: String,
    pub addr: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub group_id: Option<String>,
    pub tags: Option<Vec<String>>,
    pub key_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyMeta {
    pub id: String,
    pub name: String,
    pub algo: String, // "ed25519" | "rsa4096"
    pub fingerprint: String,
    pub public_key: String,
    pub sync_enabled: bool,
    pub created_at: String,
    pub used_by_hosts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub name: String,
    pub command: String,
    pub tags: Vec<String>,
    pub use_count: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetInput {
    pub id: Option<String>,
    pub name: String,
    pub command: String,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardRule {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String, // "L" | "R" | "D"
    pub local_port: u16,
    pub target: String,
    pub via_host_id: String,
    pub enabled: bool,
    pub bytes_up: u64,
    pub bytes_down: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardInput {
    #[serde(rename = "type")]
    pub kind: String,
    pub local_port: u16,
    pub target: String,
    pub via_host_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertInfo {
    pub id: String,
    pub domains: Vec<String>,
    pub challenge: String,
    pub ca: String,
    pub host_id: String,
    pub deploy_path: String,
    pub reload_cmd: String,
    pub not_after: i64,
    pub auto_renew: bool,
    pub state: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueRequest {
    pub domains: Vec<String>,
    pub challenge: String,
    pub ca: String,
    pub host_id: String,
    pub deploy_path: String,
    pub reload_cmd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub host_id: String,
    pub host_name: String,
    pub user: String,
    pub addr: String,
    pub port: u16,
    pub state: String, // connecting | connected | disconnected
    pub latency_ms: Option<u32>,
    pub cipher: String,
    pub cols: u16,
    pub rows: u16,
    pub recording: bool,
    pub recording_since: Option<i64>,
    /// 会话类型:ssh | local
    #[serde(default = "default_kind_ssh")]
    pub kind: String,
}

fn default_kind_ssh() -> String {
    "ssh".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferJob {
    pub id: String,
    pub direction: String, // upload | download
    pub name: String,
    pub local_path: String,
    pub remote_path: String,
    pub session_id: String,
    pub progress: f64,
    pub rate_bps: u64,
    pub state: String, // queued | running | done | failed
    pub error: Option<String>,
    pub elapsed_ms: u64,
    /// 传输协议:sftp | zmodem
    #[serde(default = "default_protocol_sftp")]
    pub protocol: String,
}

fn default_protocol_sftp() -> String {
    "sftp".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    pub session_id: String,
    pub direction: String,
    pub local_path: String,
    pub remote_path: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub id: String,
    pub session_host: String,
    pub name: String,
    pub duration_ms: u64,
    pub started_at: String,
    pub sync_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorSample {
    pub host_id: String,
    pub host_name: String,
    pub online: bool,
    pub uptime: String,
    pub cpu: u8,
    pub mem: u8,
    pub disk: u8,
    pub load: String,
    pub net_up: String,
    pub net_down: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncScopes {
    pub hosts: bool,
    pub keys: bool,
    pub snippets: bool,
    pub settings: bool,
    /// 会话录制(asciicast)是否参与同步:文件较大,默认关闭,需用户显式开启
    #[serde(default)]
    pub recordings: bool,
}

/// 同步配置:S3 端点与凭据由管理平台下发,本地只保留分项开关与主密码派生密钥。
/// endpoint 等字段保留用于展示平台下发信息(只读)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfig {
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub bucket: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub access_key: String,
    #[serde(default)]
    pub has_secret: bool,
    /// 是否已设置主密码(派生密钥已缓存)
    #[serde(default)]
    pub has_master_key: bool,
    pub scopes: SyncScopes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDevice {
    pub id: String,
    pub name: String,
    pub kind: String, // laptop | desktop | phone
    pub last_seen_at: String,
    pub current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub configured: bool,
    pub syncing: bool,
    pub last_sync_at: Option<String>,
    pub devices: Vec<SyncDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub theme: String,
    pub terminal_font: String,
    pub terminal_font_size: u8,
    pub cursor_blink: bool,
    pub restore_sessions: bool,
    pub auto_reconnect: bool,
    pub confirm_on_quit: bool,
    /// 终端里直接输入白话时,自动转交 AI 生成命令
    #[serde(default = "default_true")]
    pub natural_language_input: bool,
    /// zmodem 下载目录;为空时取系统 Downloads
    #[serde(default)]
    pub download_dir: String,
}

fn default_true() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            terminal_font: "JetBrains Mono".into(),
            terminal_font_size: 13,
            cursor_blink: true,
            restore_sessions: true,
            auto_reconnect: true,
            confirm_on_quit: false,
            natural_language_input: true,
            download_dir: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermData {
    pub id: String,
    /// base64 编码的原始输出,保证二进制安全
    pub data: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_info_kind_defaults_to_ssh() {
        let json = r#"{"id":"1","hostId":"h","hostName":"n","user":"u","addr":"a","port":22,
            "state":"connected","latencyMs":null,"cipher":"c","cols":80,"rows":24,
            "recording":false,"recordingSince":null}"#;
        let info: SessionInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.kind, "ssh");
    }

    #[test]
    fn transfer_job_protocol_defaults_to_sftp() {
        let json = r#"{"id":"1","direction":"upload","name":"n","localPath":"l","remotePath":"r",
            "sessionId":"s","progress":0,"rateBps":0,"state":"queued","error":null,"elapsedMs":0}"#;
        let job: TransferJob = serde_json::from_str(json).unwrap();
        assert_eq!(job.protocol, "sftp");
    }

    #[test]
    fn app_settings_download_dir_defaults_empty() {
        let json = r#"{"theme":"dark","terminalFont":"f","terminalFontSize":13,"cursorBlink":true,
            "restoreSessions":true,"autoReconnect":true,"confirmOnQuit":false}"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.download_dir, "");
    }
}
