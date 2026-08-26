use serde::Serialize;

/// 统一 IPC 错误 — 前端拿到的是纯字符串消息
#[derive(Debug, Serialize)]
pub struct AppError(pub String);

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AppError {}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self { AppError(format!("db: {e}")) }
}
impl From<russh::Error> for AppError {
    fn from(e: russh::Error) -> Self { AppError(format!("ssh: {e}")) }
}
impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self { AppError(format!("io: {e}")) }
}
impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self { AppError(format!("json: {e}")) }
}
impl From<String> for AppError {
    fn from(e: String) -> Self { AppError(e) }
}
impl From<&str> for AppError {
    fn from(e: &str) -> Self { AppError(e.to_string()) }
}

pub type Result<T> = std::result::Result<T, AppError>;
