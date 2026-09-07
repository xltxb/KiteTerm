pub mod frame;
pub mod receive;
pub mod send;

/// 与真实 lrzsz 的互操作测试(全部 `#[ignore]`,需本机安装 sz/rz)。
#[cfg(test)]
mod interop;

use std::io;

/// 抽象一条可读可写的字节通道:russh 通道、内存管道或本机 lrzsz 子进程 stdio
pub trait ZChannel {
    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    async fn write_all(&mut self, data: &[u8]) -> io::Result<()>;
}

/// 状态机向外报告进度,由 Tauri 侧实现为 TransferJob 事件,测试侧实现为记录器
pub trait ProgressSink {
    /// 一个文件开始传输。`path` 是本地路径:下载为实际落盘路径(已同名去重),上传为源文件。
    fn start(
        &self,
        direction: &str,
        name: &str,
        path: &std::path::Path,
        total: u64,
        index: usize,
        count: usize,
    );
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

/// 一次传输所需的上下文:通道 + 进度回调 + 收方向用的下载目录 + 发方向用的待发文件列表。
/// `download_dir` 仅 `Detected::Receive` 时有意义,`files` 仅 `Detected::Send` 时有意义。
pub struct RunCtx<'a, C: ZChannel, P: ProgressSink> {
    pub chan: &'a mut C,
    pub sink: &'a P,
    pub download_dir: std::path::PathBuf,
    pub files: Option<Vec<std::path::PathBuf>>,
}

/// 按探测结果分发到接收或发送状态机的统一入口:探测阶段(Task 7)确定 `mode` 后,
/// 调用方在此处二选一驱动 `receive::receive` 或 `send::send`。
pub async fn run<C: ZChannel, P: ProgressSink>(
    mode: Detected,
    ctx: RunCtx<'_, C, P>,
) -> std::io::Result<()> {
    match mode {
        Detected::Receive => receive::receive(ctx.chan, ctx.sink, &ctx.download_dir)
            .await
            .map(|_| ()),
        Detected::Send => {
            let files = ctx.files.unwrap_or_default();
            send::send(ctx.chan, ctx.sink, &files).await
        }
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
            // 对端(发送半边)已被 drop 且无缓冲字节剩余时,视为流结束:
            // 真实的 ZChannel 实现(子进程 stdio、russh 通道 EOF)用 Ok(0) 表达"读到流尾",
            // 而不是把它当作错误,这里对齐该语义,方便接收/发送状态机统一按 Ok(0) 处理 EOF。
            let first = match self.rx.recv().await {
                Some(b) => b,
                None => return Ok(0),
            };
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
        /// 每次 start 报上来的本地路径(下载=落盘路径,上传=源文件路径),与 starts 一一对应
        pub paths: Mutex<Vec<std::path::PathBuf>>,
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
        fn start(
            &self,
            direction: &str,
            name: &str,
            path: &std::path::Path,
            total: u64,
            index: usize,
            count: usize,
        ) {
            self.paths.lock().unwrap().push(path.to_path_buf());
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
        // sz 发的 ZRQINIT 十六进制头以 **\x18B00 开头;前面 4 字节 "junk" 是干扰前缀,
        // 命中下标应精确落在 "junk" 之后,即 4。
        assert_eq!(
            d.push(b"junk\x2a\x2a\x18B00"),
            Some((Detected::Receive, 4))
        );
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
        // 命中头完全来自上次残留的窗口,本次 data("B00")里并没有 "**\x18" 前缀,
        // 所以命中下标应为 0(saturating_sub(base) 语义:命中点落在窗口起始处)。
        assert_eq!(d.push(b"B00"), Some((Detected::Receive, 0)));
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

    /// 对端整体被 drop(其发送半边随之关闭)且无缓冲字节剩余时,read 应返回 Ok(0)
    /// 表示流结束,而不是 Err —— 真实的子进程 stdio / russh 通道 EOF 都是这个语义,
    /// 接收/发送状态机需要能统一按 Ok(0) 判断"对端读完了"。
    #[tokio::test]
    async fn mem_channel_read_returns_ok_zero_on_peer_drop() {
        let (mut a, b) = MemChannel::pair();
        drop(b);

        let mut buf = [0u8; 8];
        let n = a.read(&mut buf).await.unwrap();
        assert_eq!(n, 0);
    }

    /// 回环测试专用临时目录:同 send.rs/receive.rs 的 TmpDir 写法,Drop 时递归删除,
    /// 避免为测试引入 tempfile 依赖。
    struct TmpDir(std::path::PathBuf);
    impl TmpDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let path = std::env::temp_dir().join(format!("kiteterm-zmodem-loopback-{pid}-{id}"));
            std::fs::create_dir_all(&path).unwrap();
            TmpDir(path)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 生成确定性伪随机内容(xorshift64,按长度播种):有意避免全零/重复模式,
    /// 这样截断、偏移或子包边界类 bug 会体现为具体字节不符,而不是被"巧合正确"掩盖。
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

    /// 端到端回环:`send`/`receive` 状态机通过 `MemChannel::pair()` 真实互通,不借助任何
    /// 脚本化对端,是 send 与 receive 第一次真刀真枪对打——包括中途 ZRPOS 回退在内的完整
    /// 协议路径都要跑通。覆盖 0/1/1023/1024/1025 字节与 10MB 六种大小,逐字节断言收发相等。
    ///
    /// 握手复用说明:`send` 的前置条件是"远端 ZRINIT 已被上游探测阶段(Task 7)消费",
    /// 而 `receive` 一进入就会主动发 ZRINIT。回环里没有探测阶段替 send 吃掉这个 ZRINIT,
    /// 但 `send_impl` 等 ZRPOS/ZSKIP 的循环对非 ZRPOS/ZSKIP 帧一律 `continue` 忽略
    /// (含重复 ZRINIT,见 send.rs 注释"其它帧(重复 ZRINIT 等)忽略,继续等"),
    /// 所以这个"多出来"的 ZRINIT 会被自然吞掉,`run` 无需再额外消费它。
    ///
    /// 结束语义说明:`receive` 收发完 ZFIN/OO 后仍会再读一次通道以确认 EOF(Ok(0))才返回;
    /// 真实传输里这靠对端进程退出/管道关闭触发。回环里让 send/receive 各自把自己那一端
    /// `MemChannel` 的所有权 move 进各自的 async 块(而不是从外层借用),使其在 `run` 返回、
    /// 该 async 块结束时随作用域自然 Drop,从而让对端的 read 拿到 Ok(0)——这与
    /// receive.rs/send.rs 里脚本化 `sender`/`peer` 闭包"归还通道"的写法是同一手法。
    #[tokio::test]
    async fn loopback() {
        for &size in &[0usize, 1, 1023, 1024, 1025, 10 * 1024 * 1024] {
            let (a, b) = MemChannel::pair();

            let src_dir = TmpDir::new();
            let dst_dir = TmpDir::new();
            let content = pattern_bytes(size);
            let src_path = src_dir.path().join("payload.bin");
            std::fs::write(&src_path, &content).unwrap();
            let dst_dir_path = dst_dir.path().to_path_buf();

            let recv_sink = RecordingSink::new();
            let send_sink = RecordingSink::new();

            let recv_task = async move {
                let mut a = a;
                let ctx = RunCtx {
                    chan: &mut a,
                    sink: &recv_sink,
                    download_dir: dst_dir_path,
                    files: None,
                };
                run(Detected::Receive, ctx).await
            };
            let send_task = async move {
                let mut b = b;
                let ctx = RunCtx {
                    chan: &mut b,
                    sink: &send_sink,
                    download_dir: std::path::PathBuf::new(),
                    files: Some(vec![src_path]),
                };
                run(Detected::Send, ctx).await
            };

            let (recv_result, send_result) = tokio::join!(recv_task, send_task);
            recv_result.unwrap_or_else(|e| panic!("size={size}: receive failed: {e}"));
            send_result.unwrap_or_else(|e| panic!("size={size}: send failed: {e}"));

            let got = std::fs::read(dst_dir.path().join("payload.bin")).unwrap();
            assert_eq!(got.len(), size, "size={size}: 长度不符");
            assert_eq!(got, content, "size={size}: 内容不符");
        }
    }
}
