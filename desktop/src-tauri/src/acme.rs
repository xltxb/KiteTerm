//! ACME 证书:签发/续期/部署状态机与续期守护。纯函数在文件末尾单测。

use std::collections::HashMap;
use std::sync::Mutex;
use tokio::task::JoinHandle;

/// 跟踪进行中的签发/续期任务;续期守护单独 spawn
#[derive(Default)]
#[allow(dead_code)]
pub struct CertManager {
    pub jobs: Mutex<HashMap<String, JoinHandle<()>>>,
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
}
