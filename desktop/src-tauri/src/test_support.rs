//! 测试共用的进程内服务器:一个 russh SSH 服务端,一个回显 TCP 服务。
//!
//! 抽在这里是因为 forward 的端到端测试和 ssh 的会话测试都要一个真的
//! `Handle<ClientHandler>`,而造它只能靠真地连一次 SSH。

use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};

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
pub(crate) async fn spawn_ssh_server() -> u16 {
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
pub(crate) async fn spawn_echo_server() -> u16 {
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

