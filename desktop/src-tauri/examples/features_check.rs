//! 功能开关自检:验证下发、缓存、以及平台不可达时的取值。
//!
//! 前置:测试平台运行在 127.0.0.1:8799(见 README 的自检说明)
//! 运行:`KITE_NO_KEYCHAIN=1 cargo run --example features_check`
//! 期望输出 FEATURES_CHECK_PASS。

use kite_term_lib::account;
use kite_term_lib::db::Db;
use kite_term_lib::features;

const PLATFORM: &str = "http://127.0.0.1:8799";

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("kite-features-check").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::main]
async fn main() {
    let db = Db::open(scratch("dev")).unwrap();

    // 1) 从未拉取过 —— 按「未启用门禁」处理,四项全开;known=false 供界面区分文案
    let f = features::cached(&db);
    assert!(!f.known, "从未拉取时 known 应为 false");
    assert!(!f.enforced, "从未拉取时应视为未启用门禁");
    assert!(f.login && f.sync && f.ai, "未启用门禁 → 不限制任何功能");
    println!("step1 首次未知 → 不做门禁 OK");

    // 2) 指向一个不存在的平台:同上,且不应 panic 或报错
    account::set_platform_url(&db, "http://127.0.0.1:1").unwrap();
    let f = features::refresh(&db).await.expect("平台不可达不该返回 Err");
    assert!(!f.known && f.ai, "不可达且无缓存 → 不做门禁");
    println!("step2 平台不可达且无缓存 → 不做门禁 OK");

    // 3) 指向真实平台:拉到并落缓存
    account::set_platform_url(&db, PLATFORM).unwrap();
    let f = features::refresh(&db).await.expect("应能拉到");
    assert!(f.known, "拉到后 known 应为 true");
    println!(
        "step3 下发 enforced={} registration={} login={} sync={} ai={}",
        f.enforced, f.registration, f.login, f.sync, f.ai
    );

    let cached = features::cached(&db);
    assert_eq!(cached.login, f.login, "缓存应与下发一致");
    assert_eq!(cached.ai, f.ai);
    assert!(cached.known, "有缓存时 known 应为 true");
    println!("step4 落缓存 OK");

    // 5) 平台再次不可达:沿用缓存,不回落到全关 —— 断网不该把人锁在外面
    account::set_platform_url(&db, "http://127.0.0.1:1").unwrap();
    let f2 = features::refresh(&db).await.expect("不可达不该 Err");
    assert_eq!(f2.login, cached.login, "不可达时应沿用缓存");
    assert_eq!(f2.ai, cached.ai);
    assert!(f2.known, "沿用缓存时仍算 known");
    println!("step5 不可达 → 沿用缓存 OK");

    println!("FEATURES_CHECK_PASS");
}
