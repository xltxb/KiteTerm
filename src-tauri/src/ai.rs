//! AI 终端助手 — 直连 Claude API(POST /v1/messages,raw HTTP;Rust 无官方 SDK)。
//! API Key 经设备密钥加密存 kv;模型默认 claude-opus-5。

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::vault::Vault;

const KV_MODEL: &str = "ai_model";
const KV_KEY_ENC: &str = "ai_key_enc";
const DEFAULT_MODEL: &str = "claude-opus-5";
const API_URL: &str = "https://api.anthropic.com/v1/messages";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfig {
    pub model: String,
    pub has_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // "user" | "assistant"
    pub content: String,
}

pub fn get_config(db: &Db) -> Result<AiConfig> {
    Ok(AiConfig {
        model: db.kv_get(KV_MODEL)?.unwrap_or_else(|| DEFAULT_MODEL.into()),
        has_key: db.kv_get(KV_KEY_ENC)?.is_some(),
    })
}

pub fn save_config(db: &Db, vault: &Vault, model: String, api_key: Option<String>) -> Result<()> {
    db.kv_set(KV_MODEL, if model.is_empty() { DEFAULT_MODEL } else { &model })?;
    if let Some(key) = api_key {
        if !key.is_empty() {
            let enc = vault.encrypt(key.as_bytes())?;
            db.kv_set(KV_KEY_ENC, &base64::engine::general_purpose::STANDARD.encode(enc))?;
        }
    }
    Ok(())
}

fn api_key(db: &Db, vault: &Vault) -> Result<String> {
    let enc = db
        .kv_get(KV_KEY_ENC)?
        .ok_or_else(|| AppError("AI_KEY_MISSING: 尚未配置 Claude API Key".into()))?;
    let raw = vault.decrypt(
        &base64::engine::general_purpose::STANDARD
            .decode(enc)
            .map_err(|e| AppError(e.to_string()))?,
    )?;
    String::from_utf8(raw).map_err(|_| AppError("API Key 解码失败".into()))
}

/// terminal_context:前端截取的当前会话最近输出,作为对话上下文
pub async fn ask(
    db: &Db,
    vault: &Vault,
    messages: Vec<ChatMessage>,
    terminal_context: String,
) -> Result<String> {
    let key = api_key(db, vault)?;
    let model = db.kv_get(KV_MODEL)?.unwrap_or_else(|| DEFAULT_MODEL.into());

    let system = "你是 Vela Term(桌面 SSH 终端)内置的终端助手。用户在远程主机的终端会话中向你提问。\
        回答保持简洁,用中文;命令或代码放进 markdown 代码块(```)以便一键运行;\
        危险操作(删除、重启、改动生产配置)先给出风险提示。";

    let mut api_messages: Vec<serde_json::Value> = Vec::new();
    for (i, m) in messages.iter().enumerate() {
        // 终端上下文附在最后一条用户消息前,避免污染缓存前缀
        if i == messages.len() - 1 && m.role == "user" && !terminal_context.is_empty() {
            api_messages.push(json!({
                "role": "user",
                "content": format!(
                    "以下是当前终端会话最近的输出,供你参考:\n<terminal_output>\n{}\n</terminal_output>\n\n我的问题:{}",
                    terminal_context, m.content
                ),
            }));
        } else {
            api_messages.push(json!({ "role": m.role, "content": m.content }));
        }
    }

    let body = json!({
        "model": model,
        "max_tokens": 16000,
        "system": system,
        "messages": api_messages,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(API_URL)
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError(format!("请求 Claude API 失败: {e}")))?;

    let status = resp.status();
    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError(format!("响应解析失败: {e}")))?;

    if !status.is_success() {
        let msg = payload["error"]["message"].as_str().unwrap_or("未知错误");
        return Err(AppError(format!("Claude API {status}: {msg}")));
    }

    // refusal:安全分类器拒答(HTTP 200),透出解释而非静默失败
    if payload["stop_reason"].as_str() == Some("refusal") {
        let why = payload["stop_details"]["explanation"].as_str().unwrap_or("该请求被安全策略拒绝");
        return Ok(format!("(AI 拒绝了这次请求:{why})"));
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
        return Err(AppError("AI 未返回文本内容".into()));
    }
    Ok(text)
}
