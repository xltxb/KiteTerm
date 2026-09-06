//! oplog 增量同步端到端自检(平台管控版):
//! 两个"设备"(独立数据目录)以同一平台账户登录,凭据由管理平台下发,经 MinIO 互同步。
//! 覆盖:注册/登录 → 凭据下发 → 变更捕获 → 加密推送 → 游标增量拉取 → LWW → 墓碑 → 密钥 E2E。
//! 前置:MinIO 运行于 127.0.0.1:19000;kite-server 运行于 127.0.0.1:8787。
//! 运行:`KITE_NO_KEYCHAIN=1 cargo run --example sync_check`
//!   (跳过系统钥匙串 —— 每次重新编译都会让钥匙串把二进制当作新程序而弹授权框卡住)
//! 期望输出 SYNC_CHECK_PASS。

use kite_term_lib::account;
use kite_term_lib::db::Db;
use kite_term_lib::models::{HostInput, SyncConfig, SyncScopes};
use kite_term_lib::sync;
use kite_term_lib::vault::{self, Vault};

const PLATFORM: &str = "http://127.0.0.1:8787";

fn scratch(name: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join("kite-sync-check");
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
            scopes: SyncScopes {
                hosts: true,
                keys: true,
                snippets: true,
                settings: false,
                recordings: false,
            },
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
    // 注册即返回登录态(平台直接签发令牌);随后各设备各自登录验证多端
    account::register(&db_a, &v_a, &email, "check-password-123", None)
        .await
        .expect("注册");
    let info_a = account::login(&db_a, &v_a, &email, "check-password-123", None).await.expect("A 登录");
    let info_b = account::login(&db_b, &v_b, &email, "check-password-123", None).await.expect("B 登录");
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
        .save_snippet(kite_term_lib::models::SnippetInput {
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

    // 7) STS 限权凭据:应含 sessionToken;自己前缀可访问,他人前缀被拒
    let creds_a = account::sync_credentials(&db_a, &v_a).await.expect("凭据");
    if let Some(token) = creds_a.session_token.clone() {
        use s3::creds::Credentials;
        use s3::{Bucket, Region};
        let region = Region::Custom { region: creds_a.region.clone(), endpoint: creds_a.endpoint.clone() };
        let c = Credentials::new(
            Some(&creds_a.access_key),
            Some(&creds_a.secret_key),
            Some(&token),
            Some(&token),
            None,
        )
        .unwrap();
        let scoped = Bucket::new(&creds_a.bucket, region, c).unwrap().with_path_style();
        scoped
            .list(format!("{}/oplog/", creds_a.prefix), None)
            .await
            .expect("限权凭据应能访问自己前缀");
        let foreign = scoped.list("users/00000000-not-mine/".to_string(), None).await;
        assert!(foreign.is_err(), "限权凭据不得访问他人前缀");
        println!("step7 STS 限权凭据 OK");
    } else {
        println!("step7 SKIP(平台回退共享凭据模式)");
    }

    // 8) 压实:阈值收窄为 0 → 同步触发压实(旧 oplog 并入快照后删除);
    //    manifest 应为密文;新设备 C 仅凭快照完成引导
    std::env::set_var("KITE_COMPACT_MAX_AGE_SECS", "0");
    sync::sync_once(&db_a, &v_a).await.expect("A 压实同步");
    {
        // 直查桶:oplog 应已清空、快照存在、manifest 非明文 JSON。
        // 用平台下发给本账户的凭据(与客户端真实视角一致),避免把 MinIO 管理账号写死在测试里
        use s3::creds::Credentials;
        use s3::{Bucket, Region};
        let cn = account::sync_credentials(&db_a, &v_a).await.expect("凭据");
        let region = Region::Custom { region: cn.region.clone(), endpoint: cn.endpoint.clone() };
        let c = Credentials::new(
            Some(&cn.access_key),
            Some(&cn.secret_key),
            cn.session_token.as_deref(),
            cn.session_token.as_deref(),
            None,
        )
        .unwrap();
        let root = Bucket::new(&cn.bucket, region, c).unwrap().with_path_style();
        let count = |pages: Vec<s3::serde_types::ListBucketResult>| {
            pages.into_iter().map(|p| p.contents.len()).sum::<usize>()
        };
        let oplog_n = count(root.list(format!("{}/oplog/", creds_a.prefix), None).await.unwrap());
        let snap_n = count(root.list(format!("{}/snapshot/", creds_a.prefix), None).await.unwrap());
        assert_eq!(oplog_n, 0, "压实后 oplog 应清空");
        assert_eq!(snap_n, 1, "应只保留最新快照");
        let m = root.get_object(format!("{}/manifest.json", creds_a.prefix)).await.unwrap();
        assert!(
            serde_json::from_slice::<serde_json::Value>(m.bytes()).is_err(),
            "manifest 应为密文而非明文 JSON"
        );
    }
    let db_c = Db::open(scratch("dev-c")).unwrap();
    let v_c = Vault::open(&db_c.data_dir).unwrap();
    account::set_platform_url(&db_c, PLATFORM).unwrap();
    account::login(&db_c, &v_c, &email, "check-password-123", None).await.expect("C 登录");
    configure(&db_c, &v_c);
    sync::sync_once(&db_c, &v_c).await.expect("C 快照引导");
    assert!(
        db_c.list_hosts().unwrap().iter().any(|h| h.name.starts_with("conflict-")),
        "C 应从快照引导出主机终态"
    );
    assert!(
        db_c.list_keys().unwrap().iter().any(|k| k.fingerprint == generated.fingerprint),
        "C 应从快照引导出密钥"
    );
    assert!(
        !db_c.list_snippets().unwrap().iter().any(|s| s.name == "sync-snippet"),
        "被墓碑删除的片段不应复活"
    );
    println!("step8 压实 + 快照引导 + manifest 密文 OK");

    // 9) 恢复短语:各设备一致(24 词);新设备 D 不知主密码,仅凭短语恢复后可解密同步
    let ph_a = sync::recovery_phrase(&db_a, &v_a).unwrap();
    let ph_b = sync::recovery_phrase(&db_b, &v_b).unwrap();
    assert_eq!(ph_a, ph_b, "同一主密钥的恢复短语应一致");
    assert_eq!(ph_a.split_whitespace().count(), 24);
    let db_d = Db::open(scratch("dev-d")).unwrap();
    let v_d = Vault::open(&db_d.data_dir).unwrap();
    account::set_platform_url(&db_d, PLATFORM).unwrap();
    account::login(&db_d, &v_d, &email, "check-password-123", None).await.expect("D 登录");
    sync::restore_master_from_phrase(&db_d, &v_d, &ph_a).expect("短语恢复");
    sync::sync_once(&db_d, &v_d).await.expect("D 恢复后同步");
    assert!(
        db_d.list_hosts().unwrap().iter().any(|h| h.name.starts_with("conflict-")),
        "D 凭恢复短语应能解密数据"
    );
    println!("step9 BIP39 恢复短语 OK");

    // 10) 分组管理:新建 / 主机归组 / 重命名 / 删除迁移,并跨设备同步
    let grp = db_a.create_group("测试分组").expect("建组");
    assert_eq!(db_a.create_group("测试分组").unwrap().id, grp.id, "同名建组应幂等");
    let gh = db_a
        .save_host(HostInput {
            id: None,
            name: "grouped-host".into(),
            addr: Some("10.1.2.3".into()),
            port: Some(22),
            user: Some("root".into()),
            group_id: Some(grp.id.clone()),
            tags: None,
            key_id: None,
        })
        .unwrap();
    sync::sync_once(&db_a, &v_a).await.unwrap();
    sync::sync_once(&db_b, &v_b).await.unwrap();
    let b_grp = db_b.list_groups().unwrap().into_iter().find(|g| g.name == "测试分组");
    assert!(b_grp.is_some(), "B 应收到新分组");
    let b_host = db_b.list_hosts().unwrap().into_iter().find(|h| h.name == "grouped-host").unwrap();
    assert_eq!(b_host.group_id, b_grp.unwrap().id, "B 上主机应落在同名分组内");

    db_a.rename_group(&grp.id, "改名分组").expect("重命名");
    sync::sync_once(&db_a, &v_a).await.unwrap();
    sync::sync_once(&db_b, &v_b).await.unwrap();
    let b_renamed = db_b.list_groups().unwrap().into_iter().find(|g| g.name == "改名分组");
    assert!(b_renamed.is_some(), "B 应收到分组新名字");
    let b_host = db_b.list_hosts().unwrap().into_iter().find(|h| h.name == "grouped-host").unwrap();
    assert_eq!(b_host.group_id, b_renamed.unwrap().id, "重命名后主机仍在该组");

    let before = db_a.list_groups().unwrap().len();
    db_a.delete_group(&grp.id).expect("删组");
    assert_eq!(db_a.list_groups().unwrap().len(), before - 1, "分组应被删除");
    let a_host = db_a.list_hosts().unwrap().into_iter().find(|h| h.id == gh.id);
    assert!(a_host.is_some(), "删组不应删除主机");
    assert_ne!(a_host.unwrap().group_id, grp.id, "主机应迁到其他分组");
    sync::sync_once(&db_a, &v_a).await.unwrap();
    sync::sync_once(&db_b, &v_b).await.unwrap();
    assert!(
        !db_b.list_groups().unwrap().iter().any(|g| g.name == "改名分组"),
        "B 上该分组应随同步删除"
    );
    assert!(
        db_b.list_hosts().unwrap().iter().any(|h| h.name == "grouped-host"),
        "B 上主机应保留"
    );
    // 最后一个分组不可删
    let all = db_a.list_groups().unwrap();
    for g in all.iter().skip(1) {
        db_a.delete_group(&g.id).unwrap();
    }
    assert!(db_a.delete_group(&all[0].id).is_err(), "最后一个分组不可删除");
    println!("step10 分组增删改 + 跨设备同步 OK");

    println!("SYNC_CHECK_PASS");
}
