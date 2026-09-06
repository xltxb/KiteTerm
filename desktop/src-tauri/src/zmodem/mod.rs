pub mod frame;
pub mod receive;
pub mod send;

use std::io;

/// 抽象一条可读可写的字节通道:russh 通道、内存管道或本机 lrzsz 子进程 stdio
pub trait ZChannel {
    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    async fn write_all(&mut self, data: &[u8]) -> io::Result<()>;
}

/// 状态机向外报告进度,由 Tauri 侧实现为 TransferJob 事件,测试侧实现为记录器
pub trait ProgressSink {
    fn start(&self, direction: &str, name: &str, total: u64, index: usize, count: usize);
    fn advance(&self, transferred: u64);
    fn finish(&self, ok: bool, error: Option<String>);
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Detected {
    Receive, // 远端 sz,本机收
    Send,    // 远端 rz,本机发
}

/// 在终端输出流里滑窗匹配 zmodem 握手头
pub struct Detector {
    win: Vec<u8>,
}

impl Detector {
    pub fn new() -> Self {
        Self { win: Vec::new() }
    }
    pub fn push(&mut self, data: &[u8]) -> Option<(Detected, usize)> {
        // 维护不超过 8 字节的窗口 + 本次 data 拼接后查找 **\x18B00 / **\x18B01
        let base = self.win.len();
        self.win.extend_from_slice(data);
        let hay = &self.win;
        let find = |needle: &[u8]| hay.windows(needle.len()).position(|w| w == needle);
        let rq = find(b"\x2a\x2a\x18B00");
        let ri = find(b"\x2a\x2a\x18B01");
        let hit = match (rq, ri) {
            (Some(a), Some(b)) if a <= b => Some((Detected::Receive, a)),
            (Some(_), Some(b)) => Some((Detected::Send, b)),
            (Some(a), None) => Some((Detected::Receive, a)),
            (None, Some(b)) => Some((Detected::Send, b)),
            (None, None) => None,
        };
        if let Some((kind, at)) = hit {
            let in_data = at.saturating_sub(base); // 命中点在本次 data 中的下标
            self.win.clear();
            return Some((kind, in_data));
        }
        // 只保留尾部 7 字节以便跨 push 匹配
        if self.win.len() > 7 {
            let cut = self.win.len() - 7;
            self.win.drain(0..cut);
        }
        None
    }
}

impl Default for Detector {
    fn default() -> Self {
        Self::new()
    }
}

/// 测试用桩件:内存双向管道 `MemChannel` 与记录进度调用的 `RecordingSink`。
/// 标记为 `pub(crate)` 以便 Task 4/5 的接收/发送状态机测试复用。
#[cfg(test)]
pub(crate) mod testutil {
    use super::*;
    use std::sync::Mutex;
    use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

    /// 基于 `tokio::sync::mpsc` 的内存双向管道:A 端写入的字节可在 B 端读到,反之亦然。
    /// 每个方向各用一条无界字节通道,`read` 至少阻塞等到 1 字节,再尽量非阻塞地多取几个填满 buf。
    pub struct MemChannel {
        tx: UnboundedSender<u8>,
        rx: UnboundedReceiver<u8>,
    }

    impl MemChannel {
        /// 构造互联的一对端点:a 写入的数据从 b 读出,反之亦然
        #[allow(dead_code)]
        pub fn pair() -> (MemChannel, MemChannel) {
            let (tx_a, rx_a) = mpsc::unbounded_channel::<u8>();
            let (tx_b, rx_b) = mpsc::unbounded_channel::<u8>();
            (
                MemChannel { tx: tx_a, rx: rx_b },
                MemChannel { tx: tx_b, rx: rx_a },
            )
        }
    }

    impl ZChannel for MemChannel {
        async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            let first = self
                .rx
                .recv()
                .await
                .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "channel closed"))?;
            buf[0] = first;
            let mut n = 1;
            while n < buf.len() {
                match self.rx.try_recv() {
                    Ok(b) => {
                        buf[n] = b;
                        n += 1;
                    }
                    Err(_) => break,
                }
            }
            Ok(n)
        }

        async fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
            for &b in data {
                self.tx
                    .send(b)
                    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "peer dropped"))?;
            }
            Ok(())
        }
    }

    /// 记录 start/advance/finish 调用序列,供测试断言进度回调是否符合预期
    #[derive(Default)]
    pub struct RecordingSink {
        pub starts: Mutex<Vec<(String, String, u64, usize, usize)>>,
        pub advances: Mutex<Vec<u64>>,
        pub finishes: Mutex<Vec<(bool, Option<String>)>>,
    }

    impl RecordingSink {
        #[allow(dead_code)]
        pub fn new() -> Self {
            Self::default()
        }
    }

    impl ProgressSink for RecordingSink {
        fn start(&self, direction: &str, name: &str, total: u64, index: usize, count: usize) {
            self.starts.lock().unwrap().push((
                direction.to_string(),
                name.to_string(),
                total,
                index,
                count,
            ));
        }
        fn advance(&self, transferred: u64) {
            self.advances.lock().unwrap().push(transferred);
        }
        fn finish(&self, ok: bool, error: Option<String>) {
            self.finishes.lock().unwrap().push((ok, error));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    #[test]
    fn detects_rz_zrqinit() {
        let mut d = Detector::new();
        // sz 发的 ZRQINIT 十六进制头以 **\x18B00 开头
        assert!(matches!(
            d.push(b"junk\x2a\x2a\x18B00"),
            Some((Detected::Receive, _))
        ));
    }

    #[test]
    fn detects_rz_zrinit() {
        let mut d = Detector::new();
        assert!(matches!(
            d.push(b"\x2a\x2a\x18B01"),
            Some((Detected::Send, _))
        ));
    }

    #[test]
    fn ignores_plain_output() {
        let mut d = Detector::new();
        assert!(d.push(b"total 0\r\ndrwxr-xr-x\r\n").is_none());
    }

    #[test]
    fn detects_across_two_pushes() {
        let mut d = Detector::new();
        assert!(d.push(b"\x2a\x2a\x18").is_none());
        assert!(matches!(d.push(b"B00"), Some((Detected::Receive, _))));
    }

    /// 证明 MemChannel::pair 确实能双向传字节:Task 4/5 会依赖这个行为来跑接收/发送状态机测试。
    #[tokio::test]
    async fn mem_channel_round_trips_bytes() {
        let (mut a, mut b) = MemChannel::pair();

        a.write_all(b"hello").await.unwrap();
        let mut buf = [0u8; 5];
        let n = b.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"hello");

        b.write_all(b"world").await.unwrap();
        let mut buf2 = [0u8; 5];
        let n2 = a.read(&mut buf2).await.unwrap();
        assert_eq!(&buf2[..n2], b"world");
    }
}
