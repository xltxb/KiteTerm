//! ZMODEM 发送(远端 rz)状态机:远端跑 `rz`,本机把本地文件发过去。
//!
//! 协议流程(spec 第二部分 2.3),前置条件:调用前远端 ZRINIT 已被探测阶段消费,
//! 故 `send` 直接从第一个 ZFILE 开始:
//! 1. 对每个文件发 ZFILE(bin32,ZF0=ZCBIN 二进制转换)头 + 一个 ZCRCW 文件信息子包
//!    (`basename\0<十进制字节数> <八进制 mtime> <八进制 mode>`,与 receive.rs 的
//!    `parse_file_info` 解析规则兼容:NUL 后第一个空白分隔词按十进制读为字节数)。
//! 2. 等对端回 ZRPOS(起始偏移)或 ZSKIP。ZSKIP 则跳过该文件。
//! 3. 发 ZDATA(pos),按 1024 字节切成 ZCRCG 数据子包流式发送,整份文件的最后一个子包
//!    用 ZCRCE 结束(即便文件恰好是 1024 的整数倍,最后一包仍标记 ZCRCE,使对端解析器
//!    回到帧头模式);随后发 ZEOF(total)。空文件不发 ZDATA,直接 ZEOF(0)。
//! 4. 等对端回 ZRINIT 后进入下一个文件。
//! 5. 全部文件发完后发 ZFIN,收到对端 ZFIN 后发 `OO` 收尾。
//!
//! 流式发送期间,每发完一个子包会用**非阻塞**方式看通道是否来了 ZRPOS(对端请求回退),
//! 命中即 seek 到该偏移、重发 ZDATA(pos) 并从新位置续传。
//!
//! 数据子包一律按 crc32 编码(ZCRCG/ZCRCE, use_crc32=true):探测阶段消费掉的 ZRINIT
//! 已声明 CANFC32,lrzsz 的 rz 会据此接收 crc32;与 receive.rs 的默认解析口径一致。

use std::io;
use std::path::Path;

use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::frame::{self, Event, Parser};
use super::{ProgressSink, ZChannel};

/// ZFILE 头 ZF0 的二进制转换标志(ZCBIN=1):告诉对端按原样二进制接收,不做换行转换。
/// 依 Task 4 确认的约定,帧头 `arg` 第 4 字节(arg[3])即 ZF0。
const ZCBIN: u8 = 1;

/// 数据子包 CRC 选择:默认 crc32,理由见模块文档。
const USE_CRC32: bool = true;

/// 单个数据子包的最大负载字节数。
const SUBPACKET_LEN: usize = 1024;

/// 发送状态机主循环。
///
/// - `chan`:底层字节通道(EOF 语义为 `read` 返回 `Ok(0)`)。
/// - `sink`:进度回调。
/// - `files`:待发送的本地文件路径列表。
pub async fn send<C: ZChannel, P: ProgressSink>(
    chan: &mut C,
    sink: &P,
    files: &[std::path::PathBuf],
) -> io::Result<()> {
    match send_impl(chan, sink, files).await {
        Ok(()) => {
            sink.finish(true, None);
            Ok(())
        }
        Err(e) => {
            sink.finish(false, Some(e.to_string()));
            Err(e)
        }
    }
}

async fn send_impl<C: ZChannel, P: ProgressSink>(
    chan: &mut C,
    sink: &P,
    files: &[std::path::PathBuf],
) -> io::Result<()> {
    let mut parser = Parser::new();
    let mut rbuf = vec![0u8; 64 * 1024];
    let count = files.len();

    for (index, path) in files.iter().enumerate() {
        let meta = tokio::fs::metadata(path).await?;
        let total = meta.len();
        let mtime = mtime_secs(&meta);
        let mode = file_mode(&meta);
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());

        // 发 ZFILE(ZCBIN 二进制转换)头 + 文件信息子包(ZCRCW)
        chan.write_all(&frame::encode_bin32_header(frame::ZFILE, [0, 0, 0, ZCBIN]))
            .await?;
        let info = build_file_info(&name, total, mtime, mode);
        chan.write_all(&frame::encode_data_subpacket(&info, frame::ZCRCW, USE_CRC32))
            .await?;

        sink.start("upload", &name, total, index, count);

        // 等 ZRPOS(起始偏移)或 ZSKIP
        let start_offset = loop {
            let (kind, arg) = recv_header(chan, &mut parser, &mut rbuf).await?;
            match kind {
                frame::ZRPOS => break Some(frame::arg_to_pos(arg) as u64),
                frame::ZSKIP => break None,
                // 其它帧(重复 ZRINIT 等)忽略,继续等
                _ => continue,
            }
        };

        let offset = match start_offset {
            Some(o) => o,
            None => continue, // ZSKIP:跳过该文件,下一个文件直接发 ZFILE
        };

        // 流式发送文件内容(空区间不发 ZDATA)
        if total > offset {
            stream_file(chan, sink, &mut parser, &mut rbuf, path, offset, total).await?;
        }

        // 发 ZEOF(total),等对端 ZRINIT 征询下一个文件
        chan.write_all(&frame::encode_bin32_header(
            frame::ZEOF,
            frame::pos_to_arg(total as u32),
        ))
        .await?;
        wait_for(chan, &mut parser, &mut rbuf, frame::ZRINIT).await?;
    }

    // 全部完成:发 ZFIN,等对端 ZFIN,再发 OO
    chan.write_all(&frame::encode_hex_header(frame::ZFIN, [0; 4]))
        .await?;
    wait_for(chan, &mut parser, &mut rbuf, frame::ZFIN).await?;
    chan.write_all(b"OO").await?;

    Ok(())
}

/// 从 `offset` 起流式读发一个文件:1024 字节一个 ZCRCG 子包,整份文件的最后一包用 ZCRCE。
/// 每发完一包非阻塞地检查是否来了 ZRPOS,命中则回退偏移并重发 ZDATA 后续传。
async fn stream_file<C: ZChannel, P: ProgressSink>(
    chan: &mut C,
    sink: &P,
    parser: &mut Parser,
    rbuf: &mut [u8],
    path: &Path,
    offset: u64,
    total: u64,
) -> io::Result<()> {
    let mut file = tokio::fs::File::open(path).await?;
    if offset > 0 {
        file.seek(io::SeekFrom::Start(offset)).await?;
    }
    // 发 ZDATA(offset)
    chan.write_all(&frame::encode_bin32_header(
        frame::ZDATA,
        frame::pos_to_arg(offset as u32),
    ))
    .await?;

    let mut fbuf = vec![0u8; 64 * 1024];
    let mut pos = offset; // 已发送到的绝对文件偏移

    loop {
        let got = read_up_to(&mut file, &mut fbuf).await?;
        let file_eof = got < fbuf.len(); // 本次读到文件尾
        let mut i = 0;
        let mut rewound = false;
        while i < got {
            let end_idx = (i + SUBPACKET_LEN).min(got);
            let chunk = &fbuf[i..end_idx];
            pos += chunk.len() as u64;
            // 整份文件的最后一个子包:本次读到 EOF 且这是该次读的最后一块,或已发满 total
            let is_last = (end_idx == got && file_eof) || pos >= total;
            let marker = if is_last { frame::ZCRCE } else { frame::ZCRCG };
            chan.write_all(&frame::encode_data_subpacket(chunk, marker, USE_CRC32))
                .await?;
            sink.advance(pos);
            i = end_idx;

            if is_last {
                return Ok(());
            }

            // 非阻塞检查对端是否请求回退(ZRPOS)
            if let Some((kind, arg)) = poll_incoming(chan, parser, rbuf).await? {
                if kind == frame::ZRPOS {
                    let rewind = frame::arg_to_pos(arg) as u64;
                    file.seek(io::SeekFrom::Start(rewind)).await?;
                    pos = rewind;
                    chan.write_all(&frame::encode_bin32_header(
                        frame::ZDATA,
                        frame::pos_to_arg(rewind as u32),
                    ))
                    .await?;
                    rewound = true;
                    break; // 跳出本次缓冲的切包循环,从新位置重新读发
                }
            }
        }

        if rewound {
            continue;
        }
        if pos >= total || got == 0 {
            break;
        }
    }

    Ok(())
}

/// 尽力把 `buf` 填满,读到文件尾则提前返回已填字节数(< buf.len() 表示到达 EOF)。
async fn read_up_to<R: AsyncReadExt + Unpin>(file: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = file.read(&mut buf[filled..]).await?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

/// 阻塞读到一个帧头,返回 (kind, arg)。
async fn recv_header<C: ZChannel>(
    chan: &mut C,
    parser: &mut Parser,
    buf: &mut [u8],
) -> io::Result<(u8, [u8; 4])> {
    loop {
        if let Some(h) = next_header(parser) {
            return Ok(h);
        }
        let n = chan.read(buf).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "zmodem: peer closed while awaiting header",
            ));
        }
        parser.feed(&buf[..n]);
    }
}

/// 阻塞等待指定类型的帧头(跳过其它帧)。
async fn wait_for<C: ZChannel>(
    chan: &mut C,
    parser: &mut Parser,
    buf: &mut [u8],
    want: u8,
) -> io::Result<[u8; 4]> {
    loop {
        let (kind, arg) = recv_header(chan, parser, buf).await?;
        if kind == want {
            return Ok(arg);
        }
    }
}

/// 非阻塞地看通道是否已有一个可解析的帧头:先看解析器缓冲,再对通道做一次非阻塞 poll。
/// 无数据或半个帧头时返回 None(残留字节留在解析器里等下次)。
async fn poll_incoming<C: ZChannel>(
    chan: &mut C,
    parser: &mut Parser,
    buf: &mut [u8],
) -> io::Result<Option<(u8, [u8; 4])>> {
    if let Some(h) = next_header(parser) {
        return Ok(Some(h));
    }
    // biased select:先 poll 一次 read,若挂起则立刻由 ready 分支落地,达到非阻塞效果。
    let n = tokio::select! {
        biased;
        r = chan.read(buf) => r?,
        _ = std::future::ready(()) => return Ok(None),
    };
    if n == 0 {
        return Ok(None);
    }
    parser.feed(&buf[..n]);
    Ok(next_header(parser))
}

/// 从解析器里取下一个帧头事件(丢弃期间的 Data/GarbledCrc)。
fn next_header(parser: &mut Parser) -> Option<(u8, [u8; 4])> {
    while let Some(ev) = parser.next() {
        if let Event::Header { kind, arg } = ev {
            return Some((kind, arg));
        }
    }
    None
}

/// 构造文件信息子包:`basename\0<十进制字节数> <八进制 mtime> <八进制 mode>`。
/// 与 lrzsz 的字段顺序一致,receive.rs 的 `parse_file_info` 可正确解析出名字与字节数。
fn build_file_info(name: &str, size: u64, mtime: u64, mode: u32) -> Vec<u8> {
    let mut v = name.as_bytes().to_vec();
    v.push(0);
    v.extend_from_slice(format!("{size} {mtime:o} {mode:o}").as_bytes());
    v
}

/// 从元数据取修改时间(自 UNIX 纪元的秒数),不可用时返回 0。
fn mtime_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 取文件权限位(unix 下为 `mode & 0o7777`,其它平台回退 0o644)。
fn file_mode(meta: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0o644
    }
}

#[cfg(test)]
mod tests {
    use super::send;
    use super::{build_file_info, read_up_to};
    use crate::zmodem::frame::{self, Event, Parser};
    use crate::zmodem::testutil::{MemChannel, RecordingSink};
    use crate::zmodem::ZChannel;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 测试用临时目录:系统临时目录下建唯一子目录,Drop 时递归删除(避免引入 tempfile 依赖)。
    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let path = std::env::temp_dir().join(format!("kiteterm-zmodem-send-{pid}-{id}"));
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

    /// 在临时目录里写一个文件,返回其路径。
    fn write_file(dir: &TmpDir, name: &str, content: &[u8]) -> PathBuf {
        let p = dir.path().join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    /// 从通道读到一个帧头(帧头模式)。
    async fn recv_hdr(chan: &mut MemChannel, p: &mut Parser, buf: &mut [u8]) -> (u8, [u8; 4]) {
        loop {
            while let Some(ev) = p.next() {
                if let Event::Header { kind, arg } = ev {
                    return (kind, arg);
                }
            }
            let n = chan.read(buf).await.unwrap();
            assert!(n > 0, "通道意外 EOF,未等到帧头");
            p.feed(&buf[..n]);
        }
    }

    /// 从通道读到一个数据子包(需先 set_data_mode),返回 (bytes, end)。
    async fn recv_data(chan: &mut MemChannel, p: &mut Parser, buf: &mut [u8]) -> (Vec<u8>, u8) {
        loop {
            while let Some(ev) = p.next() {
                if let Event::Data { bytes, end } = ev {
                    return (bytes, end);
                }
            }
            let n = chan.read(buf).await.unwrap();
            assert!(n > 0, "通道意外 EOF,未等到数据子包");
            p.feed(&buf[..n]);
        }
    }

    /// 扮演 rz 接收端处理**单个**文件:读 ZFILE+信息子包,回 ZRPOS(0)(或按 skip 回 ZSKIP),
    /// 累积 ZDATA 子包直到 ZEOF,最后回 ZRINIT。返回收到的文件内容。
    async fn rz_one_file(
        peer: &mut MemChannel,
        p: &mut Parser,
        buf: &mut [u8],
        skip: bool,
    ) -> (Vec<u8>, Vec<u8>) {
        // 1. ZFILE 头
        let (kind, _) = recv_hdr(peer, p, buf).await;
        assert_eq!(kind, frame::ZFILE, "期望 ZFILE 头");
        // 2. 文件信息子包
        p.set_data_mode(true);
        let (info, _end) = recv_data(peer, p, buf).await;
        p.set_header_mode();

        if skip {
            peer.write_all(&frame::encode_hex_header(frame::ZSKIP, [0; 4]))
                .await
                .unwrap();
            return (Vec::new(), info);
        }

        // 3. 回 ZRPOS(0)
        peer.write_all(&frame::encode_hex_header(frame::ZRPOS, frame::pos_to_arg(0)))
            .await
            .unwrap();

        // 4. 读数据帧直到 ZEOF
        let mut data = Vec::new();
        loop {
            let (kind, _arg) = recv_hdr(peer, p, buf).await;
            if kind == frame::ZEOF {
                break;
            }
            assert_eq!(kind, frame::ZDATA, "期望 ZDATA 或 ZEOF");
            p.set_data_mode(true);
            loop {
                let (bytes, end) = recv_data(peer, p, buf).await;
                data.extend_from_slice(&bytes);
                if end == frame::ZCRCE {
                    break;
                }
                // ZCRCG:后面还有子包
            }
            p.set_header_mode();
        }

        // 5. 回 ZRINIT 征询下一个文件
        peer.write_all(&frame::encode_hex_header(frame::ZRINIT, [0; 4]))
            .await
            .unwrap();

        (data, info)
    }

    /// 完整扮演 rz:依次处理每个文件,末尾完成 ZFIN/OO 握手。
    /// `skips[i]` 为 true 时对第 i 个文件回 ZSKIP。返回每个文件收到的内容(跳过的为空)。
    async fn rz_peer(peer: &mut MemChannel, skips: &[bool]) -> Vec<Vec<u8>> {
        let mut p = Parser::new();
        let mut buf = [0u8; 8192];
        let mut out = Vec::new();
        for &skip in skips {
            let (data, _info) = rz_one_file(peer, &mut p, &mut buf, skip).await;
            out.push(data);
        }
        // ZFIN 握手:此刻发送端只发了 ZFIN 并阻塞等待我方 ZFIN,尚未发 OO
        let (kind, _) = recv_hdr(peer, &mut p, &mut buf).await;
        assert_eq!(kind, frame::ZFIN, "期望 ZFIN");
        peer.write_all(&frame::encode_hex_header(frame::ZFIN, [0; 4]))
            .await
            .unwrap();
        // 我方回 ZFIN 后,发送端才发原始 "OO";解析器此时无残留,直接读通道即可
        let mut oo = Vec::new();
        while oo.len() < 2 {
            let n = peer.read(&mut buf).await.unwrap();
            assert!(n > 0, "等待 OO 时通道意外 EOF");
            oo.extend_from_slice(&buf[..n]);
        }
        assert_eq!(&oo[..2], b"OO", "收尾应为 OO");
        out
    }

    #[test]
    fn build_file_info_matches_receive_parser() {
        // 与 receive.rs 的 parse_file_info 兼容:NUL 前是名字,NUL 后第一个词是十进制字节数
        let info = build_file_info("foo.txt", 5, 0o1234, 0o644);
        let (name, size) = crate::zmodem::receive::parse_file_info(&info);
        assert_eq!(name, "foo.txt");
        assert_eq!(size, 5);
    }

    #[tokio::test]
    async fn read_up_to_reads_until_eof() {
        let dir = TmpDir::new();
        let p = write_file(&dir, "x.bin", &[7u8; 10]);
        let mut f = tokio::fs::File::open(&p).await.unwrap();
        let mut buf = [0u8; 64];
        let n = read_up_to(&mut f, &mut buf).await.unwrap();
        assert_eq!(n, 10);
        assert!(buf[..10].iter().all(|&b| b == 7));
    }

    /// Step 1/4:核心场景——发一个 "hello" 文件,对端收齐、进度回调正确。
    #[tokio::test]
    async fn sends_single_file() {
        let (mut local, mut peer) = MemChannel::pair();
        let dir = TmpDir::new();
        let path = write_file(&dir, "hello.txt", b"hello");
        let sink = RecordingSink::new();

        let files = [path];
        let peer_task = async move { rz_peer(&mut peer, &[false]).await };
        let send_task = send(&mut local, &sink, &files);

        let (result, received) = tokio::join!(send_task, peer_task);
        result.unwrap();

        assert_eq!(received.len(), 1);
        assert_eq!(received[0], b"hello");

        let starts = sink.starts.lock().unwrap();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].0, "upload");
        assert_eq!(starts[0].1, "hello.txt");
        assert_eq!(starts[0].2, 5);
        assert_eq!(starts[0].3, 0); // index
        assert_eq!(starts[0].4, 1); // count
        let finishes = sink.finishes.lock().unwrap();
        assert_eq!(finishes.len(), 1);
        assert_eq!(finishes[0], (true, None));
        assert_eq!(*sink.advances.lock().unwrap().last().unwrap(), 5);
    }

    /// 空文件:size 0,不发 ZDATA,直接 ZEOF(0)。
    #[tokio::test]
    async fn sends_empty_file() {
        let (mut local, mut peer) = MemChannel::pair();
        let dir = TmpDir::new();
        let path = write_file(&dir, "empty.txt", b"");
        let sink = RecordingSink::new();

        let files = [path];
        let peer_task = async move { rz_peer(&mut peer, &[false]).await };
        let (result, received) = tokio::join!(send(&mut local, &sink, &files), peer_task);
        result.unwrap();

        assert_eq!(received.len(), 1);
        assert_eq!(received[0], b"");
        assert_eq!(sink.starts.lock().unwrap()[0].2, 0);
        assert_eq!(sink.finishes.lock().unwrap()[0], (true, None));
    }

    /// 恰好 1024 字节:单个子包,最后一包必须是 ZCRCE(否则对端解析器留在数据模式)。
    #[tokio::test]
    async fn sends_exactly_1024_bytes() {
        let (mut local, mut peer) = MemChannel::pair();
        let dir = TmpDir::new();
        let content = vec![0xABu8; 1024];
        let path = write_file(&dir, "k1.bin", &content);
        let sink = RecordingSink::new();

        let files = [path];
        let peer_task = async move { rz_peer(&mut peer, &[false]).await };
        let (result, received) = tokio::join!(send(&mut local, &sink, &files), peer_task);
        result.unwrap();

        assert_eq!(received[0], content);
        assert_eq!(*sink.advances.lock().unwrap().last().unwrap(), 1024);
    }

    /// 1025 字节:跨两个子包(1024 ZCRCG + 1 ZCRCE)。
    #[tokio::test]
    async fn sends_1025_bytes() {
        let (mut local, mut peer) = MemChannel::pair();
        let dir = TmpDir::new();
        let mut content = vec![0x11u8; 1024];
        content.push(0x22);
        let path = write_file(&dir, "k1p.bin", &content);
        let sink = RecordingSink::new();

        let files = [path];
        let peer_task = async move { rz_peer(&mut peer, &[false]).await };
        let (result, received) = tokio::join!(send(&mut local, &sink, &files), peer_task);
        result.unwrap();

        assert_eq!(received[0], content);
        assert_eq!(*sink.advances.lock().unwrap().last().unwrap(), 1025);
    }

    /// 多文件 + 中间一个被 ZSKIP 跳过。
    #[tokio::test]
    async fn skips_file_on_zskip() {
        let (mut local, mut peer) = MemChannel::pair();
        let dir = TmpDir::new();
        let a = write_file(&dir, "a.txt", b"aaa");
        let b = write_file(&dir, "b.txt", b"bbbb");
        let c = write_file(&dir, "c.txt", b"ccccc");
        let sink = RecordingSink::new();

        // 对端跳过第二个文件
        let files = [a, b, c];
        let peer_task = async move { rz_peer(&mut peer, &[false, true, false]).await };
        let (result, received) = tokio::join!(send(&mut local, &sink, &files), peer_task);
        result.unwrap();

        assert_eq!(received.len(), 3);
        assert_eq!(received[0], b"aaa");
        assert_eq!(received[1], b""); // 被跳过
        assert_eq!(received[2], b"ccccc");
        // 三个文件都调用了 start
        assert_eq!(sink.starts.lock().unwrap().len(), 3);
        assert_eq!(sink.finishes.lock().unwrap()[0], (true, None));
    }
}
