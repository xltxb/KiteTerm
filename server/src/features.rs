//! 功能开关注册表。两类:
//! - 产品开关(gated=true):受部署总闸 `KITE_FEATURE_GATING` 约束,总闸关时恒为开;
//! - 安全策略(gated=false):始终按设置值生效(邮箱验证、强制 2FA、多设备)。
//! 注册策略(open/invite/closed)不在这里,它是三档而非布尔,见 main.rs。

use rusqlite::Connection;
use serde_json::{json, Map, Value};

/// (名字, 默认值, 是否受门禁总闸)
pub const NAMES: &[(&str, bool, bool)] = &[
    ("login", true, true),
    ("sync", true, true),
    ("ai", true, true),
    ("key_sync", true, true),
    ("recording_sync", true, true),
    ("ai_command", true, true),
    ("ai_diagnose", true, true),
    ("ai_summary", true, true),
    ("ai_agent", true, true),
    ("email_verify", false, false),
    ("totp_required", false, false),
    ("multi_device", true, false),
];

fn meta(name: &str) -> (bool, bool) {
    NAMES
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, d, g)| (*d, *g))
        .unwrap_or((true, true))
}

pub fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in name.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.push(c.to_ascii_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// 原始设置值(不看总闸)—— 控制台展示用
pub fn raw(conn: &Connection, name: &str) -> bool {
    let (default, _) = meta(name);
    crate::setting_get(conn, &format!("feature_{name}"), if default { "1" } else { "0" }) == "1"
}

/// 生效值:受门禁的开关在总闸关闭时一律视为开
pub fn effective(conn: &Connection, gating: bool, name: &str) -> bool {
    let (_, gated) = meta(name);
    if gated && !gating {
        return true;
    }
    raw(conn, name)
}

pub fn set(conn: &Connection, name: &str, on: bool) -> rusqlite::Result<()> {
    crate::setting_set(conn, &format!("feature_{name}"), if on { "1" } else { "0" })
}

/// `/api/features` 里的开关部分(camelCase)
pub fn payload(conn: &Connection, gating: bool) -> Map<String, Value> {
    let mut m = Map::new();
    for (name, _, _) in NAMES {
        m.insert(camel(name), json!(effective(conn, gating, name)));
    }
    m
}

/// 管理接口里的原始值(`featureXxx` 键)
pub fn admin_payload(conn: &Connection) -> Map<String, Value> {
    let mut m = Map::new();
    for (name, _, _) in NAMES {
        let key = format!("feature{}", {
            let c = camel(name);
            let mut cs = c.chars();
            match cs.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + cs.as_str(),
                None => String::new(),
            }
        });
        m.insert(key, json!(raw(conn, name)));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE platform_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)").unwrap();
        c
    }

    #[test]
    fn defaults_and_gating() {
        let c = db();
        assert!(raw(&c, "login"));
        assert!(!raw(&c, "email_verify"));
        assert!(raw(&c, "multi_device"));
        set(&c, "login", false).unwrap();
        assert!(!raw(&c, "login"));
        // 总闸关:产品开关恒为开
        assert!(effective(&c, false, "login"));
        assert!(!effective(&c, true, "login"));
        // 安全策略不看总闸
        set(&c, "totp_required", true).unwrap();
        assert!(effective(&c, false, "totp_required"));
    }

    #[test]
    fn camel_keys() {
        assert_eq!(camel("recording_sync"), "recordingSync");
        let p = payload(&db(), true);
        assert!(p.contains_key("aiDiagnose"));
        let a = admin_payload(&db());
        assert_eq!(a["featureKeySync"], json!(true));
    }
}
