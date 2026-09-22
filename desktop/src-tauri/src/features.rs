//! 功能开关:由管理平台下发,客户端缓存后据此决定哪些入口可见。
//!
//! 是否启用门禁由平台的部署配置 `KITE_FEATURE_GATING` 决定,随开关一起下发
//! (`enforced` 字段)。总闸关闭时不做任何限制,各项功能(12 项)全部可用 ——
//! 平台此时只当账户与凭据的中枢,不管功能可见性。
//!
//! 取值顺序:
//! 1. 能连上平台 → 用刚拉到的,并落缓存;
//! 2. 连不上但有缓存 → 沿用上次下发的(断网不该把已被管控的机器一键解锁,
//!    也不该把没被管控的机器忽然锁上);
//! 3. 连不上且从未拉到过 → **按未启用门禁处理,各项全开**。
//!
//! 第 3 条是刻意的:没配过平台的纯本地用户不该被一套他从未启用的管控挡住。
//! 代价是「装了客户端但不配平台」就绕过了门禁 —— 这本来就拦不住,因为
//! 这是**产品管控,不是安全边界**:桌面端二进制在用户手里。真正的强约束在服务端 ——
//! 登录与云同步没有平台放行就是用不了,AI 助手则完全跑在本地、无从拦截。

use serde::{Deserialize, Serialize};

use crate::account::{client, platform_url};
use crate::db::Db;
use crate::error::{AppError, Result};

const KV_CACHE: &str = "feature_flags";

fn default_pw_len() -> usize {
    8
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Features {
    /// 注册策略:open | invite | closed
    pub registration: String,
    /// 平台是否启用了功能门禁。false 时下面三项恒为 true。
    #[serde(default)]
    pub enforced: bool,
    pub login: bool,
    pub sync: bool,
    pub ai: bool,
    #[serde(default = "default_true")]
    pub key_sync: bool,
    #[serde(default = "default_true")]
    pub recording_sync: bool,
    #[serde(default = "default_true")]
    pub ai_command: bool,
    #[serde(default = "default_true")]
    pub ai_diagnose: bool,
    #[serde(default = "default_true")]
    pub ai_summary: bool,
    /// Agent 能执行命令,与「生成文本给人看」的三项性质不同,单独可禁
    #[serde(default = "default_true")]
    pub ai_agent: bool,
    /// 安全策略:由服务端拦截,客户端只用来提示
    #[serde(default)]
    pub email_verify: bool,
    #[serde(default)]
    pub totp_required: bool,
    #[serde(default = "default_true")]
    pub multi_device: bool,
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
    /// 从未拉到过时的兜底:按未启用门禁处理 —— 不限制任何功能。
    /// 注册按 open 渲染即可,真正的准入以服务端为准。
    pub(crate) fn unknown() -> Self {
        Self {
            registration: "open".into(),
            enforced: false,
            login: true,
            sync: true,
            ai: true,
            key_sync: true,
            recording_sync: true,
            ai_command: true,
            ai_diagnose: true,
            ai_summary: true,
            ai_agent: true,
            email_verify: false,
            totp_required: false,
            multi_device: true,
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
    let url = format!("{}/api/features", platform_url());
    let resp = match client().get(&url).send().await {
        Ok(r) if r.status().is_success() => r,
        // 平台不可达或返回异常:沿用缓存(没缓存就是全关)
        _ => return Ok(cached(db)),
    };
    let mut flags: Features = resp.json().await.map_err(|e| AppError(e.to_string()))?;
    flags.known = true;
    db.kv_set(KV_CACHE, &serde_json::to_string(&flags)?)?;
    Ok(flags)
}
