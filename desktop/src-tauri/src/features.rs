//! 功能开关:由管理平台下发,客户端缓存后据此决定哪些入口可见。
//!
//! 取值顺序:
//! 1. 能连上平台 → 用刚拉到的,并落缓存;
//! 2. 连不上但有缓存 → 沿用上次下发的(断网不该把人锁在外面);
//! 3. 连不上且从未拉到过 → **全部关闭**(未获授权即不可用)。
//!
//! 第 3 条有一个必须的例外:平台地址本身的配置入口永远可达(在「设置」里),
//! 否则全新安装时四项全禁,用户就再也没法指向自己的平台了。
//!
//! 注意这是**产品管控,不是安全边界** —— 桌面端二进制在用户手里,
//! 登录与云同步在服务端一并拦截,AI 助手无对应服务端调用,只能靠客户端遵守。

use serde::{Deserialize, Serialize};

use crate::account::platform_url;
use crate::db::Db;
use crate::error::{AppError, Result};

const KV_CACHE: &str = "feature_flags";

fn default_pw_len() -> usize {
    8
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Features {
    /// 注册策略:open | invite | closed
    pub registration: String,
    pub login: bool,
    pub sync: bool,
    pub ai: bool,
    /// 口令规则随开关一起下发,省掉一次单独的 /api/register/policy 往返
    #[serde(default = "default_pw_len")]
    pub password_min_length: usize,
    #[serde(default)]
    pub password_rule: String,
    /// 是否真的从平台拿到过。false 表示走的是「从未拉到」的全关兜底,
    /// 界面据此把话说清楚:是「平台关的」还是「还没连上平台」。
    #[serde(default)]
    pub known: bool,
}

impl Features {
    /// 从未拉到过时的兜底:全关
    fn unknown() -> Self {
        Self {
            registration: "closed".into(),
            login: false,
            sync: false,
            ai: false,
            password_min_length: default_pw_len(),
            password_rule: String::new(),
            known: false,
        }
    }
}

/// 读缓存;没有缓存就返回全关。不发网络请求,供界面同步取用。
pub fn cached(db: &Db) -> Features {
    db.kv_get(KV_CACHE)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str::<Features>(&j).ok())
        .map(|mut f| {
            f.known = true;
            f
        })
        .unwrap_or_else(Features::unknown)
}

/// 拉取并刷新缓存。平台不可达时**不动缓存**,返回现有缓存值 ——
/// 一次网络抖动不该让功能忽然消失。
pub async fn refresh(db: &Db) -> Result<Features> {
    let url = format!("{}/api/features", platform_url(db));
    let resp = match reqwest::Client::new().get(&url).send().await {
        Ok(r) if r.status().is_success() => r,
        // 平台不可达或返回异常:沿用缓存(没缓存就是全关)
        _ => return Ok(cached(db)),
    };
    let mut flags: Features = resp.json().await.map_err(|e| AppError(e.to_string()))?;
    flags.known = true;
    db.kv_set(KV_CACHE, &serde_json::to_string(&flags)?)?;
    Ok(flags)
}
