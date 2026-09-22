//! 端到端转发测试:真实 SSH 通道 + 真实 TCP 目标,不打桩。
//!
//! 进程内起两个服务器 —— 一个 russh 测试 SSH 服务端(任何密码放行,direct-tcpip
//! 请求照实连出去),一个回显 TCP 服务 —— 然后让 `direct_conn` / `socks5_conn`
//! 在两者之间真地搬运字节,验证通路、流量计数与取消语义。
//!
//! 覆盖不到 `start_forward`:它要 `AppHandle`。函数本身对 runtime 泛型,给得出
//! `mock_app()` 的句柄就能测,但那要把 tauri 的 `test` feature 放进
//! dev-dependencies —— cargo test 与 cargo build 的 feature 集就此分家,tauri 得
//! 各编译一份。为这一个用例不值,遂作罢。listener 管理与 stop 的联动由 forward.rs
//! 里的单元测试覆盖。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use super::{direct_conn, socks5_conn};
use crate::db::Db;
use crate::models::Host;
use crate::ssh::ClientHandler;
use crate::vault::Vault;

// ---- 脚手架 ----

/// 测试用 SSH 服务端:任何密码都放行,direct-tcpip 照实连到请求的目标
struct TestSshServer;

impl russh::server::Handler for TestSshServer {
    type Error = russh::Error;

    async fn auth_password(
        &mut self,
        _user: &str,
        _password: &str,
    ) -> std::result::Result<russh::server::Auth, Self::Error> {
        Ok(russh::server::Auth::Accept)
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: russh::Channel<russh::server::Msg>,
        host: &str,
        port: u32,
        _originator: &str,
        _originator_port: u32,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut russh::server::Session,
    ) -> std::result::Result<(), Self::Error> {
        let addr = format!("{host}:{port}");
        reply.accept().await;
        tokio::spawn(async move {
            if let Ok(mut target) = TcpStream::connect(addr).await {
                let mut ch = channel.into_stream();
                let _ = tokio::io::copy_bidirectional(&mut ch, &mut target).await;
            }
        });
        Ok(())
    }
}

/// 起测试 SSH 服务端,返回监听端口
async fn spawn_ssh_server() -> u16 {
    use russh::keys::ssh_key::getrandom::SysRng;
    use russh::keys::ssh_key::rand_core::UnwrapErr;
    use russh::keys::ssh_key::{Algorithm, PrivateKey};

    let mut rng = UnwrapErr(SysRng);
    let key = PrivateKey::random(&mut rng, Algorithm::Ed25519).unwrap();
    let config = Arc::new(russh::server::Config {
        keys: vec![key],
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let config = config.clone();
            tokio::spawn(async move {
                if let Ok(session) = russh::server::run_stream(config, stream, TestSshServer).await
                {
                    let _ = session.await;
                }
            });
        }
    });
    port
}

/// 起回显 TCP 服务,返回监听端口
async fn spawn_echo_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = s.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    port
}

/// 连上测试 SSH 服务端,拿到与产品代码同款的 client handle
async fn connect(name: &str, ssh_port: u16) -> Arc<russh::client::Handle<ClientHandler>> {
    // 每次重编译都是新二进制,系统钥匙串会弹授权框卡住无人值守的测试
    std::env::set_var("KITE_NO_KEYCHAIN", "1");

    let dir = std::env::temp_dir().join("kite-forward-e2e").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let db = Arc::new(Db::open(dir.clone()).unwrap());
    let vault = Vault::open(&dir).unwrap();
    let host = Host {
        id: "h-e2e".into(),
        name: "e2e".into(),
        addr: "127.0.0.1".into(),
        port: ssh_port,
        user: "tester".into(),
        group_id: String::new(),
        tags: vec![],
        key_id: None,
        last_connected_at: None,
        latency_ms: None,
    };
    db.upsert_host_raw(&host).unwrap();

    let (handle, _, _, _) = crate::ssh::connect_host_full(db, &vault, &host.id, Some("pw"))
        .await
        .expect("应能连上测试 SSH 服务端");
    handle
}

/// 造一对互联的 TcpStream:返回 (交给被测函数的一端, 测试自己持有的一端)
async fn stream_pair() -> (TcpStream, TcpStream) {
    let bridge = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = bridge.local_addr().unwrap();
    let dialing = tokio::spawn(async move { TcpStream::connect(addr).await.unwrap() });
    let (accepted, _) = bridge.accept().await.unwrap();
    (accepted, dialing.await.unwrap())
}

/// 轮询等待条件成立;计数器的 fetch_add 排在 write_all 之后,读到数据不等于计数已落定
async fn wait_for(mut cond: impl FnMut() -> bool, what: &str) {
    for _ in 0..200 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等待超时: {what}");
}

fn counters() -> (Arc<AtomicU64>, Arc<AtomicU64>) {
    (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)))
}

// ---- 测试 ----

/// L 型:字节真地穿过 SSH direct-tcpip 通道抵达目标并回来,上下行分别计数
#[tokio::test]
async fn direct_conn_pipes_through_real_ssh_channel() {
    let echo_port = spawn_echo_server().await;
    let ssh_port = spawn_ssh_server().await;
    let handle = connect("direct", ssh_port).await;
    let (server_side, mut client) = stream_pair().await;
    let (up, down) = counters();

    let target = format!("127.0.0.1:{echo_port}");
    let (u, d) = (up.clone(), down.clone());
    tokio::spawn(async move { direct_conn(handle, server_side, 1234, &target, u, d).await });

    client.write_all(b"hello").await.unwrap();
    let mut buf = [0u8; 5];
    tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut buf))
        .await
        .expect("回显应在 5 秒内返回")
        .unwrap();
    assert_eq!(&buf, b"hello");

    wait_for(|| up.load(Ordering::Relaxed) == 5, "上行计数为 5").await;
    wait_for(|| down.load(Ordering::Relaxed) == 5, "下行计数为 5").await;
}

/// D 型:走完 SOCKS5 握手后同样能把字节送到目标
#[tokio::test]
async fn socks5_conn_handshakes_then_pipes() {
    let echo_port = spawn_echo_server().await;
    let ssh_port = spawn_ssh_server().await;
    let handle = connect("socks5", ssh_port).await;
    let (server_side, mut client) = stream_pair().await;
    let (up, down) = counters();

    let (u, d) = (up.clone(), down.clone());
    tokio::spawn(async move { socks5_conn(handle, server_side, 1234, u, d).await });

    // 握手:版本 5,一种方法(no-auth)
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut greet = [0u8; 2];
    client.read_exact(&mut greet).await.unwrap();
    assert_eq!(greet, [5, 0], "应选中 no-auth");

    // CONNECT 到 127.0.0.1:echo_port(地址类型 1 = IPv4)
    let mut req = vec![5, 1, 0, 1, 127, 0, 0, 1];
    req.extend_from_slice(&echo_port.to_be_bytes());
    client.write_all(&req).await.unwrap();
    let mut reply = [0u8; 10];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[..2], [5, 0], "CONNECT 应成功");

    client.write_all(b"socks").await.unwrap();
    let mut buf = [0u8; 5];
    tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut buf))
        .await
        .expect("回显应在 5 秒内返回")
        .unwrap();
    assert_eq!(&buf, b"socks");

    wait_for(|| up.load(Ordering::Relaxed) == 5, "上行计数为 5").await;
    wait_for(|| down.load(Ordering::Relaxed) == 5, "下行计数为 5").await;
}

/// 取消令牌落下时,真实连接必须断开 —— 这是 stop() 掐断进行中连接的底层依据
#[tokio::test]
async fn cancelling_closes_a_live_connection() {
    let echo_port = spawn_echo_server().await;
    let ssh_port = spawn_ssh_server().await;
    let handle = connect("cancel", ssh_port).await;
    let (server_side, mut client) = stream_pair().await;
    let (up, down) = counters();

    // 与 start_forward 中每条连接的包法一致
    let cancel = CancellationToken::new();
    let conn_cancel = cancel.clone();
    let target = format!("127.0.0.1:{echo_port}");
    tokio::spawn(async move {
        tokio::select! {
            _ = conn_cancel.cancelled() => {}
            _ = direct_conn(handle, server_side, 1234, &target, up, down) => {}
        }
    });

    // 先确认通路是活的
    client.write_all(b"ping").await.unwrap();
    let mut buf = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut buf))
        .await
        .expect("回显应在 5 秒内返回")
        .unwrap();
    assert_eq!(&buf, b"ping");

    cancel.cancel();

    // 取消后被测那端的 TcpStream 被 drop,本端应读到 EOF 而不是继续等数据
    let mut tail = [0u8; 8];
    let n = tokio::time::timeout(Duration::from_secs(5), client.read(&mut tail))
        .await
        .expect("取消后连接应关闭,而不是挂住")
        .unwrap();
    assert_eq!(n, 0, "取消后应读到 EOF");
}
