//! AI 终端助手 —— 多模型可选,按所选模型的协议请求对应接口。
//!
//! 支持三种协议,覆盖绝大多数服务:
//!   anthropic  POST {base}/v1/messages          (Claude)
//!   openai     POST {base}/v1/chat/completions  (OpenAI 及所有兼容实现:DeepSeek/Qwen/
//!                                                Moonshot/OpenRouter/vLLM/LM Studio…)
//!   ollama     POST {base}/api/chat             (本地模型,无需密钥)
//!
//! 密钥按模型分别加密存储(设备密钥),不回传前端。

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::vault::Vault;

const KV_PROVIDERS: &str = "ai_providers";
const KV_ACTIVE: &str = "ai_active";
const KEY_PREFIX: &str = "ai_key_";

/// 一个可选的模型配置(密钥不在其中)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProvider {
    pub id: String,
    /// 显示名
    pub name: String,
    /// anthropic | openai | ollama
    pub protocol: String,
    /// 接口根地址,如 https://api.anthropic.com
    pub base_url: String,
    /// 模型 id,如 claude-opus-5
    pub model: String,
    /// 是否已配置密钥(ollama 可不需要)
    #[serde(default)]
    pub has_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub providers: Vec<AiProvider>,
    /// 当前使用的模型 id
    pub active_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // user | assistant
    pub content: String,
}

/// 协助场景 —— 决定系统提示与输出约束
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AskMode {
    /// 侧栏自由对话
    Chat,
    /// 解释/诊断当前终端输出
    Explain,
    /// 自然语言 → 单条 shell 命令(只回命令本身)
    Command,
}

impl AskMode {
    fn parse(s: &str) -> Self {
        match s {
            "explain" => AskMode::Explain,
            "command" => AskMode::Command,
            _ => AskMode::Chat,
        }
    }

    fn system_prompt(self) -> &'static str {
        match self {
            AskMode::Chat => {
                "你是 KiteTerm(桌面 SSH 终端)内置的终端助手。用户正在远程主机的终端会话中向你提问。\
                 回答保持简洁,用中文;命令或代码放进 markdown 代码块(```)以便一键运行;\
                 危险操作(删除、重启、改动生产配置)先给出风险提示。"
            }
            AskMode::Explain => {
                "你是 KiteTerm(桌面 SSH 终端)内置的终端助手。用户会给你一段终端输出,\
                 请用中文简洁说明:这段输出表示什么、是否有错误、如果有该怎么解决。\
                 先给结论再给依据,不要逐行复述。建议的命令放进 markdown 代码块(```);\
                 危险操作先提示风险。"
            }
            AskMode::Command => {
                "你是 shell 命令生成器。根据用户的自然语言需求,输出**一条**可直接执行的 shell 命令。\
                 严格要求:只输出命令本身,不要解释、不要 markdown 代码块、不要反引号、不要前后缀说明。\
                 若需求本身危险(删除数据、重启服务等),仍然输出命令,由客户端负责让用户确认。\
                 若无法用一条命令完成,输出用 && 或 | 连接的一条复合命令。"
            }
        }
    }
}

// ---- 配置读写 ----

pub fn get_settings(db: &Db) -> Result<AiSettings> {
    let providers: Vec<AiProvider> = db
        .kv_get(KV_PROVIDERS)?
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    // has_key 以实际存储为准,避免前端传来的值失真
    let providers = providers
        .into_iter()
        .map(|mut p| {
            p.has_key = db
                .kv_get(&format!("{KEY_PREFIX}{}", p.id))
                .ok()
                .flatten()
                .is_some();
            p
        })
        .collect();
    Ok(AiSettings {
        providers,
        active_id: db.kv_get(KV_ACTIVE)?,
    })
}

/// 新增或更新一个模型配置;api_key 为 None 表示保持原密钥不变
pub fn save_provider(
    db: &Db,
    vault: &Vault,
    mut provider: AiProvider,
    api_key: Option<String>,
) -> Result<AiSettings> {
    if provider.name.trim().is_empty() {
        return Err(AppError("模型名称不能为空".into()));
    }
    if !["anthropic", "openai", "ollama"].contains(&provider.protocol.as_str()) {
        return Err(AppError(format!("不支持的协议: {}", provider.protocol)));
    }
    if provider.model.trim().is_empty() {
        return Err(AppError("模型 id 不能为空".into()));
    }
    if provider.id.trim().is_empty() {
        provider.id = uuid::Uuid::new_v4().to_string();
    }
    provider.base_url = provider.base_url.trim_end_matches('/').to_string();
    if provider.base_url.is_empty() {
        return Err(AppError("接口地址不能为空".into()));
    }

    if let Some(key) = api_key {
        let kv = format!("{KEY_PREFIX}{}", provider.id);
        if key.is_empty() {
            db.kv_del(&kv)?;
        } else {
            db.kv_set(&kv, &b64().encode(vault.encrypt(key.as_bytes())?))?;
        }
    }

    let mut all = get_settings(db)?.providers;
    match all.iter_mut().find(|p| p.id == provider.id) {
        Some(slot) => *slot = provider.clone(),
        None => all.push(provider.clone()),
    }
    db.kv_set(KV_PROVIDERS, &serde_json::to_string(&all)?)?;
    // 首个模型自动设为当前
    if db.kv_get(KV_ACTIVE)?.is_none() {
        db.kv_set(KV_ACTIVE, &provider.id)?;
    }
    get_settings(db)
}

pub fn delete_provider(db: &Db, id: &str) -> Result<AiSettings> {
    let remaining: Vec<AiProvider> = get_settings(db)?
        .providers
        .into_iter()
        .filter(|p| p.id != id)
        .collect();
    db.kv_set(KV_PROVIDERS, &serde_json::to_string(&remaining)?)?;
    db.kv_del(&format!("{KEY_PREFIX}{id}"))?;
    if db.kv_get(KV_ACTIVE)?.as_deref() == Some(id) {
        match remaining.first() {
            Some(p) => db.kv_set(KV_ACTIVE, &p.id)?,
            None => db.kv_del(KV_ACTIVE)?,
        }
    }
    get_settings(db)
}

pub fn set_active(db: &Db, id: &str) -> Result<AiSettings> {
    if !get_settings(db)?.providers.iter().any(|p| p.id == id) {
        return Err(AppError("模型不存在".into()));
    }
    db.kv_set(KV_ACTIVE, id)?;
    get_settings(db)
}

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn provider_key(db: &Db, vault: &Vault, id: &str) -> Result<Option<String>> {
    let Some(enc) = db.kv_get(&format!("{KEY_PREFIX}{id}"))? else {
        return Ok(None);
    };
    let raw = vault.decrypt(&b64().decode(enc).map_err(|e| AppError(e.to_string()))?)?;
    Ok(Some(
        String::from_utf8(raw).map_err(|_| AppError("密钥解码失败".into()))?,
    ))
}

fn active_provider(db: &Db) -> Result<AiProvider> {
    let settings = get_settings(db)?;
    let id = settings
        .active_id
        .ok_or_else(|| AppError("AI_NOT_CONFIGURED: 尚未配置 AI 模型".into()))?;
    settings
        .providers
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| AppError("AI_NOT_CONFIGURED: 当前模型配置已失效".into()))
}

// ---- 请求分发 ----

/// terminal_context:前端截取的当前会话最近输出,作为对话上下文
pub async fn ask(
    db: &Db,
    vault: &Vault,
    messages: Vec<ChatMessage>,
    terminal_context: String,
    mode: &str,
) -> Result<String> {
    let provider = active_provider(db)?;
    let key = provider_key(db, vault, &provider.id)?;
    if key.is_none() && provider.protocol != "ollama" {
        return Err(AppError(format!(
            "AI_NOT_CONFIGURED: 模型「{}」尚未配置密钥",
            provider.name
        )));
    }
    let mode = AskMode::parse(mode);

    // 终端上下文附在最后一条用户消息里,避免污染前面的缓存前缀
    let mut wire: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    for (i, m) in messages.iter().enumerate() {
        let is_last_user = i == messages.len() - 1 && m.role == "user";
        if is_last_user && !terminal_context.is_empty() {
            wire.push(ChatMessage {
                role: m.role.clone(),
                content: format!(
                    "以下是当前终端会话最近的输出,供你参考:\n<terminal_output>\n{}\n</terminal_output>\n\n{}",
                    terminal_context, m.content
                ),
            });
        } else {
            wire.push(m.clone());
        }
    }

    let text = match provider.protocol.as_str() {
        "anthropic" => ask_anthropic(&provider, key.as_deref().unwrap_or_default(), &wire, mode).await?,
        "openai" => ask_openai(&provider, key.as_deref().unwrap_or_default(), &wire, mode).await?,
        "ollama" => ask_ollama(&provider, &wire, mode).await?,
        other => return Err(AppError(format!("不支持的协议: {other}"))),
    };

    Ok(if mode == AskMode::Command {
        clean_command(&text)
    } else {
        text
    })
}

/// 命令模式下模型未必严格遵循「只输出命令」:可能裹代码块,也可能在代码块前后加说明。
/// 这里统一收敛成单行命令 —— 有代码块就取块内内容,否则取首个非注释非空行。
fn clean_command(raw: &str) -> String {
    let s = raw.trim();
    let body = match s.find("```") {
        Some(start) => {
            // 跳过 ``` 及其后的语言标注行,取到下一个 ``` 为止
            let after = &s[start + 3..];
            let inner = after.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            let end = inner.find("```").unwrap_or(inner.len());
            inner[..end].trim().to_string()
        }
        None => s.to_string(),
    };
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("# "))
        .unwrap_or("")
        .trim_start_matches("$ ")
        .trim_matches('`')
        .trim()
        .to_string()
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

async fn read_error(resp: reqwest::Response) -> AppError {
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    // 三家的错误结构不同,逐个兜底
    let msg = body["error"]["message"]
        .as_str()
        .or_else(|| body["message"].as_str())
        .or_else(|| body["error"].as_str())
        .unwrap_or("请求失败");
    AppError(format!("模型接口 {status}: {msg}"))
}

async fn ask_anthropic(
    p: &AiProvider,
    key: &str,
    messages: &[ChatMessage],
    mode: AskMode,
) -> Result<String> {
    let body = json!({
        "model": p.model,
        "max_tokens": if mode == AskMode::Command { 512 } else { 16000 },
        "system": mode.system_prompt(),
        "messages": messages.iter()
            .map(|m| json!({ "role": m.role, "content": m.content }))
            .collect::<Vec<_>>(),
    });
    let resp = http()
        .post(format!("{}/v1/messages", p.base_url))
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError(format!("请求模型接口失败: {e}")))?;
    if !resp.status().is_success() {
        return Err(read_error(resp).await);
    }
    let payload: serde_json::Value = resp.json().await.map_err(|e| AppError(e.to_string()))?;
    // 安全分类器拒答走 HTTP 200,需显式处理而非当作空响应
    if payload["stop_reason"].as_str() == Some("refusal") {
        let why = payload["stop_details"]["explanation"]
            .as_str()
            .unwrap_or("该请求被模型的安全策略拒绝");
        return Ok(format!("(模型拒绝了这次请求:{why})"));
    }
    let text = payload["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    if text.is_empty() {
        return Err(AppError("模型未返回文本内容".into()));
    }
    Ok(text)
}

async fn ask_openai(
    p: &AiProvider,
    key: &str,
    messages: &[ChatMessage],
    mode: AskMode,
) -> Result<String> {
    let mut msgs = vec![json!({ "role": "system", "content": mode.system_prompt() })];
    msgs.extend(
        messages
            .iter()
            .map(|m| json!({ "role": m.role, "content": m.content })),
    );
    let body = json!({
        "model": p.model,
        "messages": msgs,
        "max_tokens": if mode == AskMode::Command { 512 } else { 4096 },
    });
    let resp = http()
        .post(format!("{}/v1/chat/completions", p.base_url))
        .bearer_auth(key)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError(format!("请求模型接口失败: {e}")))?;
    if !resp.status().is_success() {
        return Err(read_error(resp).await);
    }
    let payload: serde_json::Value = resp.json().await.map_err(|e| AppError(e.to_string()))?;
    let text = payload["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if text.is_empty() {
        return Err(AppError("模型未返回文本内容".into()));
    }
    Ok(text)
}

async fn ask_ollama(p: &AiProvider, messages: &[ChatMessage], mode: AskMode) -> Result<String> {
    let mut msgs = vec![json!({ "role": "system", "content": mode.system_prompt() })];
    msgs.extend(
        messages
            .iter()
            .map(|m| json!({ "role": m.role, "content": m.content })),
    );
    let body = json!({ "model": p.model, "messages": msgs, "stream": false });
    let resp = http()
        .post(format!("{}/api/chat", p.base_url))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError(format!("无法连接本地模型服务({}): {e}", p.base_url)))?;
    if !resp.status().is_success() {
        return Err(read_error(resp).await);
    }
    let payload: serde_json::Value = resp.json().await.map_err(|e| AppError(e.to_string()))?;
    let text = payload["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if text.is_empty() {
        return Err(AppError("模型未返回文本内容".into()));
    }
    Ok(text)
}
