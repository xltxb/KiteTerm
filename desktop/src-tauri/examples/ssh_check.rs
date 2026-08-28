//! 无界面 SSH 栈自检:对 local-test 主机走真实 russh 连接 → 公钥认证 → exec。
//! 期望输出 SSH_CHECK_PASS;known_hosts 指纹被篡改时应输出 HOSTKEY_CHANGED 错误。
//!
//! ⚠️ 本例读的是**真实应用数据目录**(需先跑 seed 建出 local-test 主机与密钥),
//! 因此必须和应用用同一把设备密钥 —— **不要加 KITE_NO_KEYCHAIN=1**。加了会另起一把
//! 文件密钥,既解不开钥匙串加密的私钥,还会在数据目录里留下杂散的 device.key。
//! 代价是每次重新编译都会弹一次钥匙串授权框(新二进制),点允许即可。

use std::sync::Arc;

use kite_term_lib::db::Db;
use kite_term_lib::ssh;
use kite_term_lib::vault::Vault;

fn data_dir() -> std::path::PathBuf {
    dirs::data_dir().expect("no data dir").join("com.kiteterm.app")
}

#[tokio::main]
async fn main() {
    let db = Arc::new(Db::open(data_dir()).expect("db open"));
    let vault = Vault::open(&db.data_dir).expect("vault open");
    let host = db
        .list_hosts()
        .expect("list hosts")
        .into_iter()
        .find(|h| h.name == "local-test")
        .expect("先运行 seed 示例");

    match ssh::connect_host(db.clone(), &vault, &host.id, None).await {
        Ok((handle, host, latency)) => {
            println!("connected {}@{}:{} latency={}ms", host.user, host.addr, host.port, latency);
            let out = ssh::exec_capture(&handle, "uname -a && echo KITE_OK")
                .await
                .expect("exec");
            print!("{out}");
            if out.contains("KITE_OK") {
                println!("SSH_CHECK_PASS");
            } else {
                println!("SSH_CHECK_FAIL: exec 输出缺少标记");
                std::process::exit(1);
            }
        }
        Err(e) => {
            println!("CONNECT_ERR: {e}");
            std::process::exit(2);
        }
    }
}
