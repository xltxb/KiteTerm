//! 设备密钥迁移自检:构造「钥匙串密钥 + 历史 device.key」两把密钥混存的库,
//! 验证旧密文仍能解开、迁移后改用新密钥、且旧密钥文件被安全移除。
//!
//! 运行:`cargo run --example rewrap_check`
//! 期望输出 REWRAP_CHECK_PASS。
//!
//! 钥匙串那一侧用 `Vault::with_device_key` 直接给一把密钥来模拟,
//! 不触碰系统钥匙串,因此可无人值守运行。

use chacha20poly1305::aead::{KeyInit, OsRng as AeadOsRng};
use chacha20poly1305::XChaCha20Poly1305;
use kite_term_lib::db::Db;
use kite_term_lib::vault::Vault;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("kite-rewrap-check").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn main() {
    let dir = scratch("dev");

    // 1) 先用文件密钥(KITE_NO_KEYCHAIN 分支)加密两条数据 —— 模拟开发/自检期间写入的存量
    std::env::set_var("KITE_NO_KEYCHAIN", "1");
    let db = Db::open(dir.clone()).unwrap();
    let old_vault = Vault::open(&db.data_dir).unwrap();
    assert!(!old_vault.has_legacy(), "纯文件密钥场景不应有历史密钥");

    let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nSECRET\n-----END-----\n";
    let key_id = db
        .insert_key(
            "legacy-key",
            "ed25519",
            "SHA256:x",
            "ssh-ed25519 AAAA",
            &old_vault.encrypt(pem.as_bytes()).unwrap(),
        )
        .unwrap();

    let api_key = "sk-legacy-0123456789";
    db.kv_set(
        "ai_key_p1",
        &base64_encode(&old_vault.encrypt(api_key.as_bytes()).unwrap()),
    )
    .unwrap();
    println!("step1 用历史密钥写入存量数据 OK");

    // 2) 换一把「钥匙串密钥」,device.key 仍在 —— 正是出问题的那个状态
    //    (直接构造 Vault 以避免动系统钥匙串;等价于 open() 的 Keychain 分支)
    let keychain_key = XChaCha20Poly1305::generate_key(&mut AeadOsRng);
    let key_bytes: [u8; 32] = keychain_key.into();
    let vault = Vault::with_device_key(&key_bytes, &db.data_dir);
    assert!(vault.has_legacy(), "device.key 还在,应识别为历史密钥");

    // 新密钥单独解不开旧密文
    let blob = db.get_key_private_enc(&key_id).unwrap();
    assert!(vault.decrypt_current(&blob).is_err(), "新密钥不该能解开旧密文");
    // 但带回退的 decrypt 能解开 —— 这就是修复后不再报错的原因
    assert_eq!(vault.decrypt(&blob).unwrap(), pem.as_bytes(), "回退解密应拿回原文");
    println!("step2 历史密文可经回退解开 OK");

    // 3) 迁移
    let n = kite_term_lib::rewrap::run(&db, &vault).unwrap();
    assert_eq!(n, 2, "两条密文都该被重新加密,实际 {n}");

    let blob = db.get_key_private_enc(&key_id).unwrap();
    assert_eq!(
        vault.decrypt_current(&blob).unwrap(),
        pem.as_bytes(),
        "迁移后应能只用当前密钥解开私钥"
    );
    let enc = db.kv_get("ai_key_p1").unwrap().unwrap();
    assert_eq!(
        vault.decrypt_current(&base64_decode(&enc)).unwrap(),
        api_key.as_bytes(),
        "迁移后应能只用当前密钥解开 AI 密钥"
    );
    println!("step3 迁移后改用当前密钥 OK");

    // 4) 全部成功才删 device.key
    assert!(!db.data_dir.join("device.key").exists(), "迁移完成后应删除历史 device.key");
    println!("step4 历史 device.key 已移除 OK");

    // 5) 幂等:再跑一次不该有任何改动(此时已无历史密钥)
    let vault2 = Vault::with_device_key(&key_bytes, &db.data_dir);
    assert!(!vault2.has_legacy(), "device.key 已删,不应再有历史密钥");
    assert_eq!(kite_term_lib::rewrap::run(&db, &vault2).unwrap(), 0, "重复迁移应为空操作");
    println!("step5 幂等 OK");

    println!("REWRAP_CHECK_PASS");
}

fn base64_encode(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn base64_decode(s: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}
