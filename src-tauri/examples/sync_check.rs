//! oplog 增量同步端到端自检(平台管控版):
//! 两个"设备"(独立数据目录)以同一平台账户登录,凭据由管理平台下发,经 MinIO 互同步。
//! 覆盖:注册/登录 → 凭据下发 → 变更捕获 → 加密推送 → 游标增量拉取 → LWW → 墓碑 → 密钥 E2E。
//! 前置:MinIO 运行于 127.0.0.1:19000;vela-server 运行于 127.0.0.1:8787。
//! 期望输出 SYNC_CHECK_PASS。

use vela_term_lib::account;
use vela_term_lib::db::Db;
use vela_term_lib::models::{HostInput, SyncConfig, SyncScopes};
use vela_term_lib::sync;
use vela_term_lib::vault::{self, Vault};

const PLATFORM: &str = "http://127.0.0.1:8787";

fn scratch(name: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join("vela-sync-check");
    let dir = base.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn configure(db: &Db, v: &Vault) {
    // 平台模式下本地只配置分项开关与主密码
    sync::save_config(
        db,
        v,
        SyncConfig {
            endpoint: String::new(),
            bucket: String::new(),
            region: String::new(),
            access_key: String::new(),
            has_secret: false,
            has_master_key: false,
            scopes: SyncScopes { hosts: true, keys: true, snippets: true, settings: false },
            secret_key: None,
            master_password: Some("test-master-密码".into()),
        },
    )
    .expect("save_config");
}

#[tokio::main]
async fn main() {
    // 两台"设备",同一平台账户
    let db_a = Db::open(scratch("dev-a")).unwrap();
    let db_b = Db::open(scratch("dev-b")).unwrap();
    let v_a = Vault::open(&db_a.data_dir).unwrap();
    let v_b = Vault::open(&db_b.data_dir).unwrap();
    account::set_platform_url(&db_a, PLATFORM).unwrap();
    account::set_platform_url(&db_b, PLATFORM).unwrap();

    let email = format!("sync-check-{}@test.local", db_a.device_id());
    account::register(&db_a, &email, "check-password-123").await.expect("注册");
    let info_a = account::login(&db_a, &v_a, &email, "check-password-123").await.expect("A 登录");
    let info_b = account::login(&db_b, &v_b, &email, "check-password-123").await.expect("B 登录");
    assert_eq!(info_a.id, info_b.id, "同一账户");
    assert!(info_a.sync_enabled, "新账户默认放开同步");
    println!("step0 注册/登录/凭据 OK(user={})", info_a.id);

    configure(&db_a, &v_a);
    configure(&db_b, &v_b);
    assert_ne!(db_a.device_id(), db_b.device_id(), "设备 ID 必须不同");

    // 1) A 创建主机 + 片段 + 同步密钥
    let host = db_a
        .save_host(HostInput {
            id: None,
            name: "sync-host".into(),
            addr: Some("10.9.9.9".into()),
            port: Some(22),
            user: Some("root".into()),
            group_id: None,
            tags: Some(vec!["同步测试".into()]),
            key_id: None,
        })
        .unwrap();
    let snip = db_a
        .save_snippet(vela_term_lib::models::SnippetInput {
            id: None,
            name: "sync-snippet".into(),
            command: "echo synced".into(),
            tags: None,
        })
        .unwrap();
    let generated = vault::generate("ed25519", "sync-check").unwrap();
    let enc = v_a.encrypt(generated.private_pem.as_bytes()).unwrap();
    let key_id = db_a
        .insert_key("id_sync_test", "ed25519", &generated.fingerprint, &generated.public_openssh, &enc)
        .unwrap();
    db_a.set_key_sync(&key_id, true).unwrap();

    sync::sync_once(&db_a, &v_a).await.expect("A push");
    sync::sync_once(&db_b, &v_b).await.expect("B pull");

    let b_host = db_b.list_hosts().unwrap().into_iter().find(|h| h.name == "sync-host");
    assert!(b_host.is_some(), "B 应收到主机");
    assert!(db_b.list_snippets().unwrap().iter().any(|s| s.name == "sync-snippet"), "B 应收到片段");
    let b_key = db_b.list_keys().unwrap().into_iter().find(|k| k.fingerprint == generated.fingerprint);
    assert!(b_key.is_some(), "B 应收到密钥");
    // 密钥 E2E:B 侧应能用自己的设备密钥解出同一把私钥
    let b_pem = v_b.decrypt(&db_b.get_key_private_enc(&b_key.unwrap().id).unwrap()).unwrap();
    assert_eq!(String::from_utf8(b_pem).unwrap(), generated.private_pem, "私钥应一致");
    println!("step1 初始同步 OK");

    // 2) B 改名 → A 应收到(LWW:B 的 lamport 更新)
    let b_host = b_host.unwrap();
    db_b.save_host(HostInput {
        id: Some(b_host.id.clone()),
        name: "sync-host-renamed".into(),
        addr: None,
        port: None,
        user: None,
        group_id: None,
        tags: None,
        key_id: None,
    })
    .unwrap();
    sync::sync_once(&db_b, &v_b).await.expect("B push rename");
    sync::sync_once(&db_a, &v_a).await.expect("A pull rename");
    assert!(
        db_a.list_hosts().unwrap().iter().any(|h| h.name == "sync-host-renamed"),
        "A 应收到改名(LWW)"
    );
    println!("step2 LWW 改名 OK");

    // 3) A 删除片段 → B 应删除(墓碑)
    db_a.delete_snippet(&snip.id).unwrap();
    sync::sync_once(&db_a, &v_a).await.expect("A push del");
    sync::sync_once(&db_b, &v_b).await.expect("B pull del");
    assert!(
        !db_b.list_snippets().unwrap().iter().any(|s| s.name == "sync-snippet"),
        "B 的片段应被墓碑删除"
    );
    println!("step3 墓碑删除 OK");

    // 4) 旧条目不回卷:B 再拉一次,改名结果不被 A 的旧 put 覆盖
    sync::sync_once(&db_b, &v_b).await.expect("B pull again");
    assert!(
        db_b.list_hosts().unwrap().iter().any(|h| h.name == "sync-host-renamed"),
        "旧条目不得覆盖新状态"
    );
    println!("step4 幂等重放 OK");

    // 5) 并发冲突:A、B 离线同时改同一主机,双方同步两轮后必须收敛到同一结果
    let hid = host.id.clone();
    db_a.save_host(HostInput { id: Some(hid.clone()), name: "conflict-A".into(), addr: None, port: None, user: None, group_id: None, tags: None, key_id: None }).unwrap();
    db_b.save_host(HostInput { id: Some(hid.clone()), name: "conflict-B".into(), addr: None, port: None, user: None, group_id: None, tags: None, key_id: None }).unwrap();
    for _ in 0..2 {
        sync::sync_once(&db_a, &v_a).await.unwrap();
        sync::sync_once(&db_b, &v_b).await.unwrap();
    }
    let name_a = db_a.list_hosts().unwrap().into_iter().find(|h| h.id == hid).unwrap().name;
    let name_b = db_b.list_hosts().unwrap().into_iter().find(|h| h.id == hid).unwrap().name;
    assert_eq!(name_a, name_b, "冲突后两端必须收敛一致(A={name_a} B={name_b})");
    assert!(name_a.starts_with("conflict-"), "收敛结果应为冲突双方之一");
    println!("step5 并发冲突收敛 OK({name_a})");

    // 6) 平台管控:管理员停用该账户同步 → 客户端同步被拒;重新放开 → 恢复
    let client = reqwest::Client::new();
    let admin_url = format!("{PLATFORM}/api/admin/users/{email}/sync");
    let r = client
        .post(&admin_url)
        .header("x-admin-key", "test-admin-key")
        .json(&serde_json::json!({ "enabled": false }))
        .send()
        .await
        .expect("admin disable");
    assert!(r.status().is_success(), "管理员停用失败: {}", r.status());
    let denied = sync::sync_once(&db_a, &v_a).await;
    let msg = denied.expect_err("停用后同步必须被拒").to_string();
    assert!(msg.contains("SYNC_DISABLED"), "错误应标记 SYNC_DISABLED,实际: {msg}");
    let r = client
        .post(&admin_url)
        .header("x-admin-key", "test-admin-key")
        .json(&serde_json::json!({ "enabled": true }))
        .send()
        .await
        .expect("admin enable");
    assert!(r.status().is_success());
    sync::sync_once(&db_a, &v_a).await.expect("恢复后同步应成功");
    println!("step6 平台管控停用/恢复 OK");

    println!("SYNC_CHECK_PASS");
}
