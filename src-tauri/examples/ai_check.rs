//! AI 多协议分发自检:三种协议各配一个模型,验证请求打到正确的端点、
//! 带对应的鉴权头,且响应按各自格式解析;并验证 command 模式的输出收敛。
//!
//! 前置:mock 服务运行于 127.0.0.1:18900
//! 运行:`KITE_NO_KEYCHAIN=1 cargo run --example ai_check`
//! 期望输出 AI_CHECK_PASS。

use kite_term_lib::ai::{self, AiProvider, ChatMessage};
use kite_term_lib::db::Db;
use kite_term_lib::vault::Vault;

const MOCK: &str = "http://127.0.0.1:18900";

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("kite-ai-check").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn user(text: &str) -> Vec<ChatMessage> {
    vec![ChatMessage { role: "user".into(), content: text.into() }]
}

#[tokio::main]
async fn main() {
    let db = Db::open(scratch("dev")).unwrap();
    let vault = Vault::open(&db.data_dir).unwrap();

    // 三种协议各建一个模型
    for (name, protocol, model) in [
        ("Claude", "anthropic", "claude-opus-5"),
        ("OpenAI 兼容", "openai", "gpt-4o"),
        ("本地 Ollama", "ollama", "llama3.1"),
    ] {
        ai::save_provider(
            &db,
            &vault,
            AiProvider {
                id: String::new(),
                name: name.into(),
                protocol: protocol.into(),
                base_url: MOCK.into(),
                model: model.into(),
                has_key: false,
            },
            // ollama 不配密钥,验证无密钥也能用
            if protocol == "ollama" { None } else { Some(format!("secret-for-{protocol}")) },
        )
        .expect("保存模型");
    }
    let settings = ai::get_settings(&db).unwrap();
    assert_eq!(settings.providers.len(), 3, "应有三个模型");
    assert!(settings.active_id.is_some(), "首个模型应自动设为当前");
    assert!(
        settings.providers.iter().filter(|p| p.has_key).count() == 2,
        "两个模型有密钥,ollama 无密钥"
    );
    println!("step1 多模型配置 OK");

    // 逐个切换并请求,验证分发到不同端点
    let find = |name: &str| {
        ai::get_settings(&db)
            .unwrap()
            .providers
            .into_iter()
            .find(|p| p.name == name)
            .unwrap()
    };

    ai::set_active(&db, &find("Claude").id).unwrap();
    let r = ai::ask(&db, &vault, user("hi"), "ctx".into(), "chat").await.expect("anthropic");
    assert!(r.contains("ANTHROPIC_OK"), "应走 Anthropic 协议: {r}");

    ai::set_active(&db, &find("OpenAI 兼容").id).unwrap();
    let r = ai::ask(&db, &vault, user("hi"), String::new(), "chat").await.expect("openai");
    assert_eq!(r, "OPENAI_OK", "应走 OpenAI 协议");

    ai::set_active(&db, &find("本地 Ollama").id).unwrap();
    let r = ai::ask(&db, &vault, user("hi"), String::new(), "chat").await.expect("ollama");
    assert_eq!(r, "OLLAMA_OK", "应走 Ollama 协议(且无需密钥)");
    println!("step2 三协议分发 OK");

    // command 模式:mock 回的是带代码块的文本,应被收敛成单行命令
    ai::set_active(&db, &find("Claude").id).unwrap();
    let cmd = ai::ask(&db, &vault, user("找出占用 8080 的进程"), String::new(), "command")
        .await
        .expect("command");
    assert_eq!(cmd, "lsof -i :8080", "命令模式应剥掉代码块与说明,实际: {cmd:?}");
    println!("step3 命令模式输出收敛 OK");

    // 校验 mock 侧真实收到的请求
    let log: Vec<serde_json::Value> = reqwest::get(format!("{MOCK}/__log"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let anthropic = log.iter().find(|e| e["path"] == "/v1/messages").expect("有 anthropic 请求");
    assert_eq!(anthropic["x_api_key"], "secret-for-anthropic", "应带 x-api-key");
    assert_eq!(anthropic["anthropic_version"], "2023-06-01", "应带 anthropic-version");
    assert!(anthropic["system"].is_string(), "anthropic 用顶层 system");

    let openai = log.iter().find(|e| e["path"] == "/v1/chat/completions").expect("有 openai 请求");
    assert_eq!(openai["auth"], "Bearer secret-for-openai", "应带 Bearer 鉴权");
    assert_eq!(openai["messages"][0]["role"], "system", "openai 把 system 放进 messages");

    let ollama = log.iter().find(|e| e["path"] == "/api/chat").expect("有 ollama 请求");
    assert!(ollama["auth"].is_null(), "ollama 不应带鉴权头");
    println!("step4 请求头与报文格式 OK");

    // 终端上下文应注入最后一条用户消息
    let ctx_req = log
        .iter()
        .find(|e| e["path"] == "/v1/messages" && e["messages"][0]["content"].as_str().unwrap_or("").contains("terminal_output"))
        .expect("终端上下文应被注入");
    assert!(
        ctx_req["messages"][0]["content"].as_str().unwrap().contains("ctx"),
        "上下文内容应在消息里"
    );
    println!("step5 终端上下文注入 OK");

    // 删除与切换
    let openai_id = find("OpenAI 兼容").id;
    let after = ai::delete_provider(&db, &openai_id).unwrap();
    assert_eq!(after.providers.len(), 2, "删除后剩两个");
    assert!(after.providers.iter().all(|p| p.id != openai_id));
    println!("step6 模型删除 OK");

    // 7) 响应形态兼容:曾经报「模型未返回文本内容」的几种真实形态都要能取出正文
    let with_model = |name: &str, protocol: &str, model: &str| {
        ai::save_provider(
            &db,
            &vault,
            AiProvider {
                id: String::new(),
                name: name.into(),
                protocol: protocol.into(),
                base_url: MOCK.into(),
                model: model.into(),
                has_key: false,
            },
            Some("k".into()),
        )
        .unwrap();
        ai::get_settings(&db).unwrap().providers.into_iter().find(|p| p.name == name).unwrap().id
    };

    for (name, model) in [
        ("多模态数组", "array-content"),
        ("推理模型", "reasoning"),
        ("旧式补全", "legacy"),
    ] {
        let id = with_model(name, "openai", model);
        ai::set_active(&db, &id).unwrap();
        let r = ai::ask(&db, &vault, user("hi"), String::new(), "chat")
            .await
            .unwrap_or_else(|e| panic!("{name} 应能解析出正文: {e}"));
        assert_eq!(r, "OPENAI_OK", "{name} 的正文提取不正确");
    }
    let id = with_model("Ollama generate", "ollama", "generate-style");
    ai::set_active(&db, &id).unwrap();
    let r = ai::ask(&db, &vault, user("hi"), String::new(), "chat").await.expect("ollama response 字段");
    assert_eq!(r, "OLLAMA_OK");
    println!("step7 响应形态兼容 OK");

    // 8) 拿不到正文时,错误信息要能指出原因而不是一句空话
    let id = with_model("只思考", "anthropic", "only-thinking");
    ai::set_active(&db, &id).unwrap();
    let err = ai::ask(&db, &vault, user("hi"), String::new(), "command")
        .await
        .expect_err("只有思考块应报错");
    let msg = err.to_string();
    assert!(msg.contains("思考") && msg.contains("max_tokens"), "错误应说明原因,实际: {msg}");

    let id = with_model("被截断", "openai", "truncated");
    ai::set_active(&db, &id).unwrap();
    let err = ai::ask(&db, &vault, user("hi"), String::new(), "chat")
        .await
        .expect_err("空正文应报错");
    assert!(err.to_string().contains("长度上限"), "错误应说明被截断,实际: {err}");
    println!("step8 空正文的错误诊断 OK");

    // 9) 接口地址规整:带 /v1 后缀会被去掉,避免拼成 /v1/v1/...
    ai::save_provider(
        &db,
        &vault,
        AiProvider {
            id: String::new(),
            name: "带 v1 后缀".into(),
            protocol: "openai".into(),
            base_url: format!("{MOCK}/v1/"),
            model: "gpt-4o".into(),
            has_key: false,
        },
        Some("k".into()),
    )
    .unwrap();
    let p = ai::get_settings(&db).unwrap().providers.into_iter().find(|p| p.name == "带 v1 后缀").unwrap();
    assert_eq!(p.base_url, MOCK, "末尾 /v1 应被规整掉");
    ai::set_active(&db, &p.id).unwrap();
    let r = ai::ask(&db, &vault, user("hi"), String::new(), "chat").await.expect("规整后应可用");
    assert_eq!(r, "OPENAI_OK");

    let bad = ai::save_provider(
        &db,
        &vault,
        AiProvider {
            id: String::new(),
            name: "缺协议头".into(),
            protocol: "openai".into(),
            base_url: "api.example.com".into(),
            model: "m".into(),
            has_key: false,
        },
        None,
    );
    assert!(bad.is_err(), "缺少 http(s):// 应被拒绝");
    println!("step9 接口地址规整 OK");

    println!("AI_CHECK_PASS");
}
