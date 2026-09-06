//! ZMODEM 接收(rz)状态机:远端跑 `sz`,本机接收文件并落盘。
//!
//! 协议流程(spec 第二部分 2.2):
//! 1. 发 ZRINIT 声明接收能力(CANFDX|CANOVIO|CANFC32),用 `Parser` 读对端帧。
//! 2. 收 ZFILE(bin32)后进入数据模式读文件信息子包(`name\0size mtime mode ...`),
//!    在 `dir` 下按同名去重规则建文件,发 ZRPOS(0),回到帧头模式。
//! 3. 收 ZDATA 后读数据子包写盘,按子包结束符(ZCRCQ/ZCRCW)回 ZACK。
//! 4. 收 ZEOF 关闭当前文件、再发 ZRINIT 征询下一个文件。
//! 5. 收 ZFIN 回 ZFIN 并吞掉尾部 `OO`。
//! 6. 遇到 `Event::GarbledCrc` 发 ZRPOS(当前偏移)请求重传;连续 10 次中止返回 `Err`。

use std::io;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use super::frame::{self, Event, Parser};
use super::{ProgressSink, ZChannel};

/// 声明的接收能力位:CANFDX(0x01) | CANOVIO(0x02) | CANFC32(0x20) = 0x23。
/// 放在 ZF0(帧头 arg 的第 4 字节)——`sz` 从 ZF0 读取能力位来决定是否用 crc32,
/// 而 ZP0/ZP1(arg 低两字节)是接收缓冲长度,置 0 表示不限(流式)。
const RX_CAPS: [u8; 4] = [0, 0, 0, 0x23];

/// 连续 CRC 错误上限,超过则放弃本次传输。
const MAX_CRC_ERRORS: u32 = 10;

/// 当前 Data 事件的语义:决定收到数据子包后如何处理。
enum DataCtx {
    /// 帧模式,不期待数据子包。
    None,
    /// ZFILE 之后的文件信息子包。
    FileInfo,
    /// ZSINIT 之后的 Attn 字符串子包(读取后丢弃)。
    ZsinitAttn,
    /// ZDATA 之后的文件内容子包。
    FileData,
}

/// 接收状态机主循环。收完返回所有已落盘文件的路径。
///
/// - `chan`:底层字节通道(EOF 语义为 `read` 返回 `Ok(0)`)。
/// - `sink`:进度回调。
/// - `dir`:下载目录。
pub async fn receive<C: ZChannel, P: ProgressSink>(
    chan: &mut C,
    sink: &P,
    dir: &Path,
) -> io::Result<Vec<PathBuf>> {
    let mut parser = Parser::new();
    let mut read_buf = vec![0u8; 64 * 1024];
    let mut received: Vec<PathBuf> = Vec::new();

    // 当前文件状态
    let mut writer: Option<tokio::io::BufWriter<tokio::fs::File>> = None;
    let mut cur_path: Option<PathBuf> = None;
    let mut cur_written: u64 = 0;
    let mut ctx = DataCtx::None;
    let mut crc_errors: u32 = 0;
    let mut finished = false; // 已收到 ZFIN,进入收尾(吞 OO 等 EOF)

    // 数据子包一律按 crc32 解析:我们通过 ZRINIT 协商了 CANFC32,`sz` 会用 bin32/crc32;
    // 且 `Parser::Event` 不携带帧的 crc 格式信息,故按协商结果默认 crc32。
    const USE_CRC32: bool = true;

    // 先发 ZRINIT 征询发送端
    chan.write_all(&frame::encode_hex_header(frame::ZRINIT, RX_CAPS))
        .await?;

    loop {
        match parser.next() {
            None => {
                let n = chan.read(&mut read_buf).await?;
                if n == 0 {
                    // 流结束。若仍有未收完的文件,报告失败。
                    if !finished && writer.is_some() {
                        sink.finish(false, Some("unexpected EOF".to_string()));
                    }
                    break;
                }
                parser.feed(&read_buf[..n]);
            }
            Some(Event::Header { kind, arg }) => {
                // 注意:不在此重置 crc_errors。重传帧同样带 ZDATA 头,
                // 若在收到帧头时清零,连续 CRC 错误将永远数不到上限。
                // 仅在成功收到数据子包(真实进度)时清零,见下方 Data 分支。
                match kind {
                    frame::ZRQINIT => {
                        // 发送端在握手期重复征询,重发 ZRINIT
                        chan.write_all(&frame::encode_hex_header(frame::ZRINIT, RX_CAPS))
                            .await?;
                    }
                    frame::ZSINIT => {
                        // ZSINIT 头后跟一个包含 Attn 字符串的数据子包,读取后丢弃再回 ZACK
                        parser.set_data_mode(USE_CRC32);
                        ctx = DataCtx::ZsinitAttn;
                    }
                    frame::ZFILE => {
                        // 文件信息子包随后到来
                        parser.set_data_mode(USE_CRC32);
                        ctx = DataCtx::FileInfo;
                    }
                    frame::ZDATA => {
                        // 数据子包随后到来;按帧头偏移定位写入位置
                        let offset = frame::arg_to_pos(arg) as u64;
                        if let Some(w) = writer.as_mut() {
                            if offset != cur_written {
                                w.seek(io::SeekFrom::Start(offset)).await?;
                                cur_written = offset;
                            }
                        }
                        parser.set_data_mode(USE_CRC32);
                        ctx = DataCtx::FileData;
                    }
                    frame::ZEOF => {
                        // 关闭当前文件,征询下一个
                        if let Some(mut w) = writer.take() {
                            w.flush().await?;
                            if let Some(path) = cur_path.take() {
                                received.push(path);
                            }
                            sink.finish(true, None);
                        }
                        ctx = DataCtx::None;
                        chan.write_all(&frame::encode_hex_header(frame::ZRINIT, RX_CAPS))
                            .await?;
                    }
                    frame::ZFIN => {
                        // 回 ZFIN,随后吞掉尾部 OO 直到 EOF
                        chan.write_all(&frame::encode_hex_header(frame::ZFIN, [0; 4]))
                            .await?;
                        finished = true;
                        ctx = DataCtx::None;
                    }
                    _ => {
                        // ZSKIP/ZNAK/未知帧:忽略
                    }
                }
            }
            Some(Event::Data { bytes, end }) => {
                crc_errors = 0;
                match ctx {
                    DataCtx::FileInfo => {
                        let (name, total) = parse_file_info(&bytes);
                        let path = dedup_path(dir, &name);
                        let file = tokio::fs::File::create(&path).await?;
                        writer = Some(tokio::io::BufWriter::with_capacity(64 * 1024, file));
                        cur_path = Some(path);
                        cur_written = 0;
                        sink.start("download", &name, total, received.len(), 0);
                        // 请求从偏移 0 开始发送数据
                        chan.write_all(&frame::encode_hex_header(
                            frame::ZRPOS,
                            frame::pos_to_arg(0),
                        ))
                        .await?;
                        parser.set_header_mode();
                        ctx = DataCtx::None;
                    }
                    DataCtx::ZsinitAttn => {
                        // 丢弃 Attn 字符串,回 ZACK
                        chan.write_all(&frame::encode_hex_header(frame::ZACK, [0; 4]))
                            .await?;
                        parser.set_header_mode();
                        ctx = DataCtx::None;
                    }
                    DataCtx::FileData => {
                        if let Some(w) = writer.as_mut() {
                            w.write_all(&bytes).await?;
                            cur_written += bytes.len() as u64;
                            sink.advance(cur_written);
                        }
                        // 按结束符决定是否回 ZACK、是否回帧头模式
                        match end {
                            frame::ZCRCQ => {
                                // 期待 ACK,后续仍有数据子包
                                chan.write_all(&frame::encode_hex_header(
                                    frame::ZACK,
                                    frame::pos_to_arg(cur_written as u32),
                                ))
                                .await?;
                            }
                            frame::ZCRCW => {
                                // 期待 ACK,帧结束
                                chan.write_all(&frame::encode_hex_header(
                                    frame::ZACK,
                                    frame::pos_to_arg(cur_written as u32),
                                ))
                                .await?;
                                parser.set_header_mode();
                                ctx = DataCtx::None;
                            }
                            frame::ZCRCE => {
                                // 帧结束,无需 ACK
                                parser.set_header_mode();
                                ctx = DataCtx::None;
                            }
                            // ZCRCG:后续仍有数据子包,无需 ACK,保持数据模式
                            _ => {}
                        }
                    }
                    DataCtx::None => {
                        // 帧模式下不应出现数据子包,忽略
                    }
                }
            }
            Some(Event::GarbledCrc) => {
                crc_errors += 1;
                if crc_errors >= MAX_CRC_ERRORS {
                    sink.finish(false, Some("too many CRC errors".to_string()));
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "zmodem: too many consecutive CRC errors",
                    ));
                }
                // 请求从当前偏移重传,并重置解析器丢弃损坏帧残留字节,回到帧头模式。
                // 发送端在收到 ZRPOS 前不会继续发送,故此时缓冲中只有损坏帧残余,可安全丢弃。
                chan.write_all(&frame::encode_hex_header(
                    frame::ZRPOS,
                    frame::pos_to_arg(cur_written as u32),
                ))
                .await?;
                parser = Parser::new();
                ctx = DataCtx::None;
            }
        }
    }

    Ok(received)
}

/// 解析文件信息子包:`name\0size mtime mode ...`,返回(文件名, 字节数)。
/// `pub(crate)` 以便 send.rs 的测试做「信息子包格式互通」交叉校验。
pub(crate) fn parse_file_info(bytes: &[u8]) -> (String, u64) {
    let nul = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let name = String::from_utf8_lossy(&bytes[..nul]).into_owned();
    let rest: &[u8] = if nul < bytes.len() {
        &bytes[nul + 1..]
    } else {
        &[]
    };
    let rest_str = String::from_utf8_lossy(rest);
    let size = rest_str
        .split_whitespace()
        .next()
        .and_then(|t| t.parse::<u64>().ok())
        .unwrap_or(0);
    (name, size)
}

/// 同名去重:`foo.txt` 已存在则用 `foo (1).txt`、`foo (2).txt` ……
fn dedup_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(name);
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    let ext = p.extension().and_then(|s| s.to_str());
    let mut n: u32 = 1;
    loop {
        let fname = match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let c = dir.join(fname);
        if !c.exists() {
            return c;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::receive;
    use super::{dedup_path, parse_file_info};
    use crate::zmodem::frame::{self, Event, Parser};
    use crate::zmodem::testutil::{MemChannel, RecordingSink};
    use crate::zmodem::ZChannel;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 测试用临时目录:在系统临时目录下建唯一子目录,Drop 时递归删除。
    /// 避免为测试引入 `tempfile` 依赖。
    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let path = std::env::temp_dir().join(format!("kiteterm-zmodem-test-{pid}-{id}"));
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

    /// 从通道上读取,直到解析出一个帧头,返回 (kind, arg)。用于测试里的「发送端」等待本机回帧。
    async fn recv_header(chan: &mut MemChannel, parser: &mut Parser) -> (u8, [u8; 4]) {
        let mut buf = [0u8; 4096];
        loop {
            if let Some(ev) = parser.next() {
                match ev {
                    Event::Header { kind, arg } => return (kind, arg),
                    _ => continue,
                }
            }
            let n = chan.read(&mut buf).await.unwrap();
            assert!(n > 0, "通道意外 EOF,未等到期望的帧头");
            parser.feed(&buf[..n]);
        }
    }

    /// 等待收到指定类型的帧头(跳过其它帧),返回其 arg。
    async fn wait_for(chan: &mut MemChannel, parser: &mut Parser, want_kind: u8) -> [u8; 4] {
        loop {
            let (kind, arg) = recv_header(chan, parser).await;
            if kind == want_kind {
                return arg;
            }
        }
    }

    /// 构造文件信息子包字节:`name\0size 0 0`。
    fn file_info(name: &str, size: u64) -> Vec<u8> {
        let mut v = name.as_bytes().to_vec();
        v.push(0);
        v.extend_from_slice(format!("{size} 0 0").as_bytes());
        v
    }

    #[test]
    fn parse_file_info_splits_name_and_size() {
        let (name, size) = parse_file_info(b"foo.txt\x005 0 0");
        assert_eq!(name, "foo.txt");
        assert_eq!(size, 5);
    }

    #[test]
    fn dedup_path_appends_counter() {
        let dir = TmpDir::new();
        // 首次不冲突
        let p0 = dedup_path(dir.path(), "foo.txt");
        assert_eq!(p0, dir.path().join("foo.txt"));
        std::fs::write(&p0, b"x").unwrap();
        // 冲突后加 (1)
        let p1 = dedup_path(dir.path(), "foo.txt");
        assert_eq!(p1, dir.path().join("foo (1).txt"));
        std::fs::write(&p1, b"x").unwrap();
        // 再冲突加 (2)
        let p2 = dedup_path(dir.path(), "foo.txt");
        assert_eq!(p2, dir.path().join("foo (2).txt"));
        // 无扩展名
        let n = dedup_path(dir.path(), "README");
        assert_eq!(n, dir.path().join("README"));
    }

    /// Step 1:脚本化发送端喂帧,断言落盘 + 回帧 + 进度回调。
    #[tokio::test]
    async fn receives_single_file() {
        let (mut local, mut peer) = MemChannel::pair();
        let dir = TmpDir::new();
        let dir_path = dir.path().to_path_buf();
        let sink = RecordingSink::new();

        let sender = async move {
            let mut p = Parser::new();
            wait_for(&mut peer, &mut p, frame::ZRINIT).await;
            peer.write_all(&frame::encode_bin32_header(frame::ZFILE, [0; 4]))
                .await
                .unwrap();
            peer.write_all(&frame::encode_data_subpacket(
                &file_info("foo.txt", 5),
                frame::ZCRCW,
                true,
            ))
            .await
            .unwrap();

            let rpos = wait_for(&mut peer, &mut p, frame::ZRPOS).await;
            assert_eq!(frame::arg_to_pos(rpos), 0);

            peer.write_all(&frame::encode_bin32_header(frame::ZDATA, frame::pos_to_arg(0)))
                .await
                .unwrap();
            peer.write_all(&frame::encode_data_subpacket(b"hello", frame::ZCRCE, true))
                .await
                .unwrap();
            peer.write_all(&frame::encode_bin32_header(frame::ZEOF, frame::pos_to_arg(5)))
                .await
                .unwrap();

            wait_for(&mut peer, &mut p, frame::ZRINIT).await;
            peer.write_all(&frame::encode_hex_header(frame::ZFIN, [0; 4]))
                .await
                .unwrap();
            wait_for(&mut peer, &mut p, frame::ZFIN).await;
            peer.write_all(b"OO").await.unwrap();
            // 归还通道:drop 后本机 read 得到 Ok(0)
        };

        let recv = receive(&mut local, &sink, &dir_path);
        let (result, ()) = tokio::join!(recv, sender);
        let files = result.unwrap();

        // (a) 内容正确
        let path = dir_path.join("foo.txt");
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        // (b) 返回该路径
        assert_eq!(files, vec![path]);
        // (d) 进度回调
        let starts = sink.starts.lock().unwrap();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].0, "download");
        assert_eq!(starts[0].1, "foo.txt");
        assert_eq!(starts[0].2, 5);
        let finishes = sink.finishes.lock().unwrap();
        assert_eq!(finishes.len(), 1);
        assert_eq!(finishes[0], (true, None));
        // advance 反映真实进度:写入 5 字节
        let advances = sink.advances.lock().unwrap();
        assert_eq!(*advances.last().unwrap(), 5);
    }

    /// 空文件:size 0,无 ZDATA,直接 ZEOF(0)。
    #[tokio::test]
    async fn receives_empty_file() {
        let (mut local, mut peer) = MemChannel::pair();
        let dir = TmpDir::new();
        let dir_path = dir.path().to_path_buf();
        let sink = RecordingSink::new();

        let sender = async move {
            let mut p = Parser::new();
            wait_for(&mut peer, &mut p, frame::ZRINIT).await;
            peer.write_all(&frame::encode_bin32_header(frame::ZFILE, [0; 4]))
                .await
                .unwrap();
            peer.write_all(&frame::encode_data_subpacket(
                &file_info("empty.txt", 0),
                frame::ZCRCW,
                true,
            ))
            .await
            .unwrap();
            wait_for(&mut peer, &mut p, frame::ZRPOS).await;
            peer.write_all(&frame::encode_bin32_header(frame::ZEOF, frame::pos_to_arg(0)))
                .await
                .unwrap();
            wait_for(&mut peer, &mut p, frame::ZRINIT).await;
            peer.write_all(&frame::encode_hex_header(frame::ZFIN, [0; 4]))
                .await
                .unwrap();
            wait_for(&mut peer, &mut p, frame::ZFIN).await;
            peer.write_all(b"OO").await.unwrap();
        };

        let (result, ()) = tokio::join!(receive(&mut local, &sink, &dir_path), sender);
        let files = result.unwrap();

        let path = dir_path.join("empty.txt");
        assert!(path.exists());
        assert_eq!(std::fs::read(&path).unwrap(), b"");
        assert_eq!(files, vec![path]);
        assert_eq!(sink.finishes.lock().unwrap()[0], (true, None));
    }

    /// 多文件:两个 ZFILE 顺序传输。
    #[tokio::test]
    async fn receives_two_files() {
        let (mut local, mut peer) = MemChannel::pair();
        let dir = TmpDir::new();
        let dir_path = dir.path().to_path_buf();
        let sink = RecordingSink::new();

        let sender = async move {
            let mut p = Parser::new();
            // 文件 1
            wait_for(&mut peer, &mut p, frame::ZRINIT).await;
            peer.write_all(&frame::encode_bin32_header(frame::ZFILE, [0; 4]))
                .await
                .unwrap();
            peer.write_all(&frame::encode_data_subpacket(
                &file_info("a.txt", 3),
                frame::ZCRCW,
                true,
            ))
            .await
            .unwrap();
            wait_for(&mut peer, &mut p, frame::ZRPOS).await;
            peer.write_all(&frame::encode_bin32_header(frame::ZDATA, frame::pos_to_arg(0)))
                .await
                .unwrap();
            peer.write_all(&frame::encode_data_subpacket(b"aaa", frame::ZCRCE, true))
                .await
                .unwrap();
            peer.write_all(&frame::encode_bin32_header(frame::ZEOF, frame::pos_to_arg(3)))
                .await
                .unwrap();
            // 文件 2
            wait_for(&mut peer, &mut p, frame::ZRINIT).await;
            peer.write_all(&frame::encode_bin32_header(frame::ZFILE, [0; 4]))
                .await
                .unwrap();
            peer.write_all(&frame::encode_data_subpacket(
                &file_info("b.txt", 4),
                frame::ZCRCW,
                true,
            ))
            .await
            .unwrap();
            wait_for(&mut peer, &mut p, frame::ZRPOS).await;
            peer.write_all(&frame::encode_bin32_header(frame::ZDATA, frame::pos_to_arg(0)))
                .await
                .unwrap();
            peer.write_all(&frame::encode_data_subpacket(b"bbbb", frame::ZCRCE, true))
                .await
                .unwrap();
            peer.write_all(&frame::encode_bin32_header(frame::ZEOF, frame::pos_to_arg(4)))
                .await
                .unwrap();
            // 收尾
            wait_for(&mut peer, &mut p, frame::ZRINIT).await;
            peer.write_all(&frame::encode_hex_header(frame::ZFIN, [0; 4]))
                .await
                .unwrap();
            wait_for(&mut peer, &mut p, frame::ZFIN).await;
            peer.write_all(b"OO").await.unwrap();
        };

        let (result, ()) = tokio::join!(receive(&mut local, &sink, &dir_path), sender);
        let files = result.unwrap();

        assert_eq!(std::fs::read(dir_path.join("a.txt")).unwrap(), b"aaa");
        assert_eq!(std::fs::read(dir_path.join("b.txt")).unwrap(), b"bbbb");
        assert_eq!(files, vec![dir_path.join("a.txt"), dir_path.join("b.txt")]);
        assert_eq!(sink.starts.lock().unwrap().len(), 2);
        assert_eq!(sink.finishes.lock().unwrap().len(), 2);
    }

    /// CRC 错误重传:发送端先发坏 CRC 子包,收到 ZRPOS 后重发正确子包。
    #[tokio::test]
    async fn retransmits_on_crc_error() {
        let (mut local, mut peer) = MemChannel::pair();
        let dir = TmpDir::new();
        let dir_path = dir.path().to_path_buf();
        let sink = RecordingSink::new();

        let sender = async move {
            let mut p = Parser::new();
            wait_for(&mut peer, &mut p, frame::ZRINIT).await;
            peer.write_all(&frame::encode_bin32_header(frame::ZFILE, [0; 4]))
                .await
                .unwrap();
            peer.write_all(&frame::encode_data_subpacket(
                &file_info("data.bin", 5),
                frame::ZCRCW,
                true,
            ))
            .await
            .unwrap();
            // 初次 ZRPOS(0)
            wait_for(&mut peer, &mut p, frame::ZRPOS).await;
            // 发坏 CRC 的数据子包:破坏一个数据字节使 CRC 不符
            peer.write_all(&frame::encode_bin32_header(frame::ZDATA, frame::pos_to_arg(0)))
                .await
                .unwrap();
            let mut bad = frame::encode_data_subpacket(b"hello", frame::ZCRCE, true);
            bad[0] ^= 0xff; // 'h' -> 0x97,仍是普通数据字节,只是内容错,CRC 不符
            peer.write_all(&bad).await.unwrap();
            // 收到重传请求 ZRPOS(0)
            let rpos = wait_for(&mut peer, &mut p, frame::ZRPOS).await;
            assert_eq!(frame::arg_to_pos(rpos), 0);
            // 重发正确子包
            peer.write_all(&frame::encode_bin32_header(frame::ZDATA, frame::pos_to_arg(0)))
                .await
                .unwrap();
            peer.write_all(&frame::encode_data_subpacket(b"hello", frame::ZCRCE, true))
                .await
                .unwrap();
            peer.write_all(&frame::encode_bin32_header(frame::ZEOF, frame::pos_to_arg(5)))
                .await
                .unwrap();
            wait_for(&mut peer, &mut p, frame::ZRINIT).await;
            peer.write_all(&frame::encode_hex_header(frame::ZFIN, [0; 4]))
                .await
                .unwrap();
            wait_for(&mut peer, &mut p, frame::ZFIN).await;
            peer.write_all(b"OO").await.unwrap();
        };

        let (result, ()) = tokio::join!(receive(&mut local, &sink, &dir_path), sender);
        let files = result.unwrap();

        let path = dir_path.join("data.bin");
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        assert_eq!(files, vec![path]);
        assert_eq!(sink.finishes.lock().unwrap()[0], (true, None));
    }

    /// 连续 10 次 CRC 错误后中止:用一个总是返回坏帧的通道桩件驱动。
    #[tokio::test]
    async fn aborts_after_ten_crc_errors() {
        // 每次 read 返回一个 ZDATA 头 + 坏 CRC 数据子包;write 全部丢弃并计数。
        struct AbortChannel {
            chunk: Vec<u8>,
            writes: usize,
        }
        impl ZChannel for AbortChannel {
            async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = self.chunk.len().min(buf.len());
                buf[..n].copy_from_slice(&self.chunk[..n]);
                Ok(n)
            }
            async fn write_all(&mut self, _data: &[u8]) -> std::io::Result<()> {
                self.writes += 1;
                Ok(())
            }
        }

        let mut chunk = frame::encode_bin32_header(frame::ZDATA, frame::pos_to_arg(0));
        let mut bad = frame::encode_data_subpacket(b"hello", frame::ZCRCE, true);
        bad[0] ^= 0xff;
        chunk.extend_from_slice(&bad);
        // 断言 chunk 足够小,能被一次 read 装下(64KB 缓冲)
        assert!(chunk.len() < 64 * 1024);

        let mut chan = AbortChannel { chunk, writes: 0 };
        let dir = TmpDir::new();
        let sink = RecordingSink::new();

        let err = receive(&mut chan, &sink, dir.path()).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        // 最后一次进度回调为失败
        let finishes = sink.finishes.lock().unwrap();
        assert_eq!(finishes.last().unwrap().0, false);
    }
}
