//! 测试共用的进程内服务器:一个 russh SSH 服务端,一个回显 TCP 服务。
//!
//! 抽在这里是因为 forward 的端到端测试和 ssh 的会话测试都要一个真的
//! `Handle<ClientHandler>`,而造它只能靠真地连一次 SSH。

use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};

/// 测试用 SSH 服务端:任何密码都放行,direct-tcpip 照实连到请求的目标,
/// tcpip-forward 照实在本机监听并把进来的连接经 forwarded-tcpip 回送给客户端
/// —— 与 OpenSSH sshd 对这两类请求的处理一致。
#[derive(Default)]
struct TestSshServer {
    /// tcpip-forward 起的监听任务。russh 在会话结束时丢弃 handler,
    /// 这里随之中止监听 —— 对应 sshd 在客户端断开后释放远程转发端口
    listeners: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for TestSshServer {
    fn drop(&mut self) {
        for t in &self.listeners {
            t.abort();
        }
    }
}

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

    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut russh::server::Session,
    ) -> std::result::Result<bool, Self::Error> {
        let Ok(listener) = TcpListener::bind((address, *port as u16)).await else {
            return Ok(false); // 端口被占用等:按协议回绝,客户端收到失败
        };
        if *port == 0 {
            *port = listener.local_addr().unwrap().port() as u32;
        }
        let (address, port, handle) = (address.to_string(), *port, session.handle());
        self.listeners.push(tokio::spawn(async move {
            while let Ok((mut stream, peer)) = listener.accept().await {
                let (address, handle) = (address.clone(), handle.clone());
                tokio::spawn(async move {
                    // 客户端拒绝(未登记的端口)时打开失败,stream 随之 drop,对端读到 EOF
                    let opened = handle
                        .channel_open_forwarded_tcpip(
                            address,
                            port,
                            peer.ip().to_string(),
                            peer.port() as u32,
                        )
                        .await;
                    if let Ok(channel) = opened {
                        let mut ch = channel.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut ch, &mut stream).await;
                    }
                });
            }
        }));
        Ok(true)
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
                if let Ok(session) = russh::server::run_stream(config, stream, TestSshServer::default()).await
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

