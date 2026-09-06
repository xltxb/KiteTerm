//! 与真实 lrzsz(`sz`/`rz`)的互操作测试。
//!
//! 回环测试(mod.rs::loopback)只能证明我们自己的编码器/解码器能互通;
//! 唯有对着真实的 lrzsz 二进制跑一遍,才能证明我们的状态机与真实实现在
//! CRC 协商、帧头尾、ZDLE 转义、ZF0 能力位、信息子包格式上都兼容。
//!
//! 用法(需本机已装 lrzsz,故全部标 `#[ignore]`,普通 `cargo test` 会跳过):
//! ```text
//! cargo test zmodem::interop -- --ignored --nocapture
//! ```
//!
//! 通道实现:把子进程的 stdin(我们写)+ stdout(我们读)包成一个 `ZChannel`。
//! - `sz` 互操作:对端跑 `sz`,把 ZMODEM 流写到它的 stdout,从它的 stdin 读我们的回应;
//!   我们跑 `receive`(接收方)。
//! - `rz` 互操作:对端跑 `rz`(收到当前工作目录),我们跑 `send`(发送方)。

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdin, ChildStdout, Command};

use super::testutil::RecordingSink;
use super::{receive::receive, send::send, ZChannel};

/// 整体超时:任一环节挂起时让测试失败而不是永久阻塞。
const OVERALL_TIMEOUT: Duration = Duration::from_secs(20);

/// 基于子进程 stdio 的通道:`read` 读子进程 stdout,`write_all` 写子进程 stdin 并 flush。
/// 子进程关闭 stdout(退出)时 `read` 返回 `Ok(0)`,与 `ZChannel` 的 EOF 语义一致。
struct ChildChannel {
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl ZChannel for ChildChannel {
    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stdout.read(buf).await
    }

    async fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        self.stdin.write_all(data).await?;
        // 真实链路是逐段交互式收发,必须立刻 flush,否则对端等不到我们的帧而挂起。
        self.stdin.flush().await
    }
}

/// 测试用临时目录:系统临时目录下建唯一子目录,Drop 时递归删除(不引入 tempfile 依赖)。
struct TmpDir(PathBuf);
impl TmpDir {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("kiteterm-zmodem-interop-{pid}-{id}"));
        std::fs::create_dir_all(&path).unwrap();
        TmpDir(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 生成确定性伪随机内容(xorshift64,按长度播种):避免全零/重复模式,
/// 使截断、偏移或子包边界类缺陷体现为具体字节不符,而不是被"巧合正确"掩盖。
fn pattern_bytes(len: usize) -> Vec<u8> {
    let mut state: u64 = 0x9E3779B97F4A7C15 ^ (len as u64).wrapping_add(1);
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push((state & 0xFF) as u8);
    }
    out
}

/// 覆盖的文件大小:空、1 字节、跨子包边界(约 5000)、约 1MB 随机。
const SIZES: &[usize] = &[0, 1, 5000, 1024 * 1024];

/// sz 互操作:对端 `sz -b <file>` 发送,我们 `receive` 接收,断言落盘字节完全一致。
#[tokio::test]
#[ignore = "需要本机安装 lrzsz(sz),普通 cargo test 跳过"]
async fn sz_sends_we_receive() {
    for &size in SIZES {
        let src_dir = TmpDir::new();
        let dst_dir = TmpDir::new();
        let content = pattern_bytes(size);
        let src_path = src_dir.path().join("payload.bin");
        std::fs::write(&src_path, &content).unwrap();

        // -b 二进制传输,避免 CR/LF 转换。zmodem 是 sz 的默认协议,无需 --zmodem。
        let mut child = Command::new("sz")
            .arg("-b")
            .arg(&src_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sz 失败(确认 lrzsz 已安装且在 PATH)");

        let mut chan = ChildChannel {
            stdin: child.stdin.take().unwrap(),
            stdout: child.stdout.take().unwrap(),
        };
        let sink = RecordingSink::new();
        let dst = dst_dir.path().to_path_buf();

        let result = tokio::time::timeout(OVERALL_TIMEOUT, receive(&mut chan, &sink, &dst)).await;

        match result {
            Ok(Ok(paths)) => {
                assert_eq!(paths.len(), 1, "size={size}: 应恰好收到 1 个文件");
            }
            Ok(Err(e)) => {
                let _ = child.start_kill();
                panic!("size={size}: receive 返回错误: {e}");
            }
            Err(_) => {
                let _ = child.start_kill();
                panic!("size={size}: 传输超时(挂起)");
            }
        }

        let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;

        let got = std::fs::read(dst_dir.path().join("payload.bin"))
            .unwrap_or_else(|e| panic!("size={size}: 下载文件读取失败: {e}"));
        assert_eq!(got.len(), size, "size={size}: 长度不符");
        assert_eq!(got, content, "size={size}: 内容不符");
    }
}

/// rz 互操作:我们 `send` 发送,对端 `rz -b`(落到其工作目录)接收,断言字节完全一致。
#[tokio::test]
#[ignore = "需要本机安装 lrzsz(rz),普通 cargo test 跳过"]
async fn we_send_rz_receives() {
    for &size in SIZES {
        let src_dir = TmpDir::new();
        let recv_dir = TmpDir::new();
        let content = pattern_bytes(size);
        let src_path = src_dir.path().join("payload.bin");
        std::fs::write(&src_path, &content).unwrap();

        // rz 把文件收到其当前工作目录;-b 二进制传输。
        let mut child = Command::new("rz")
            .arg("-b")
            .current_dir(recv_dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rz 失败(确认 lrzsz 已安装且在 PATH)");

        let mut chan = ChildChannel {
            stdin: child.stdin.take().unwrap(),
            stdout: child.stdout.take().unwrap(),
        };
        let sink = RecordingSink::new();
        let files = vec![src_path.clone()];

        let result = tokio::time::timeout(OVERALL_TIMEOUT, send(&mut chan, &sink, &files)).await;

        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = child.start_kill();
                panic!("size={size}: send 返回错误: {e}");
            }
            Err(_) => {
                let _ = child.start_kill();
                panic!("size={size}: 传输超时(挂起)");
            }
        }

        let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;

        let got = std::fs::read(recv_dir.path().join("payload.bin"))
            .unwrap_or_else(|e| panic!("size={size}: 接收文件读取失败: {e}"));
        assert_eq!(got.len(), size, "size={size}: 长度不符");
        assert_eq!(got, content, "size={size}: 内容不符");
    }
}
