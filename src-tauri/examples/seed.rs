//! 测试播种:用应用自身的 vault/db 模块生成密钥并注册 local-test 主机(127.0.0.1:2222)。
//! 用法: cargo run --example seed -- <authorized_keys 路径>

use vela_term_lib::db::Db;
use vela_term_lib::models::HostInput;
use vela_term_lib::vault::{self, Vault};

fn data_dir() -> std::path::PathBuf {
    dirs::data_dir().expect("no data dir").join("com.vela.term")
}

fn main() {
    let authorized_keys = std::env::args().nth(1).expect("用法: seed <authorized_keys 路径>");
    let db = Db::open(data_dir()).expect("db open");
    let vault_store = Vault::open(&db.data_dir).expect("vault open");

    // 幂等:密钥与主机已存在则复用
    let key = db
        .list_keys()
        .expect("list keys")
        .into_iter()
        .find(|k| k.name == "id_ed25519_localtest");
    let (key_id, public_key) = match key {
        Some(k) => (k.id, k.public_key),
        None => {
            let generated = vault::generate("ed25519", "vela-localtest").expect("keygen");
            let enc = vault_store
                .encrypt(generated.private_pem.as_bytes())
                .expect("encrypt");
            let id = db
                .insert_key(
                    "id_ed25519_localtest",
                    &generated.algo,
                    &generated.fingerprint,
                    &generated.public_openssh,
                    &enc,
                )
                .expect("insert key");
            (id, generated.public_openssh)
        }
    };

    let host = db
        .list_hosts()
        .expect("list hosts")
        .into_iter()
        .find(|h| h.name == "local-test");
    if host.is_none() {
        db.save_host(HostInput {
            id: None,
            name: "local-test".into(),
            addr: Some("127.0.0.1".into()),
            port: Some(2222),
            user: Some(whoami::username()),
            group_id: None,
            tags: Some(vec!["本机".into(), "测试".into()]),
            key_id: Some(key_id.clone()),
        })
        .expect("save host");
    }

    // 公钥写入测试 sshd 的 authorized_keys(幂等)
    let existing = std::fs::read_to_string(&authorized_keys).unwrap_or_default();
    if !existing.contains(&public_key) {
        std::fs::write(&authorized_keys, format!("{existing}{public_key}\n")).expect("write ak");
    }
    println!("SEED_OK key_id={key_id}");
}
