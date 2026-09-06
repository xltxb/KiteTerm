//! ZMODEM 帧的纯函数编解码:CRC、ZDLE 转义、帧头与数据子包、增量解析器。不依赖 Tauri。

pub const ZPAD: u8 = b'*';
pub const ZDLE: u8 = 0x18;
pub const ZBIN32: u8 = b'C';
pub const ZHEX: u8 = b'B';

pub const ZRQINIT: u8 = 0;
pub const ZRINIT: u8 = 1;
pub const ZSINIT: u8 = 2;
pub const ZACK: u8 = 3;
pub const ZFILE: u8 = 4;
pub const ZSKIP: u8 = 5;
pub const ZNAK: u8 = 6;
pub const ZFIN: u8 = 8;
pub const ZRPOS: u8 = 9;
pub const ZDATA: u8 = 10;
pub const ZEOF: u8 = 11;
pub const ZFERR: u8 = 12;
pub const ZCAN: u8 = 16;

pub const ZCRCE: u8 = 0x68;
pub const ZCRCG: u8 = 0x69;
pub const ZCRCQ: u8 = 0x6a;
pub const ZCRCW: u8 = 0x6b;

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    crc ^ 0xFFFF_FFFF
}

/// 需要转义的字节:ZDLE 本身与常见流控/telnet 敏感字节
fn needs_escape(b: u8) -> bool {
    matches!(b, 0x18 | 0x10 | 0x11 | 0x13 | 0x90 | 0x91 | 0x93)
}

pub fn zdle_escape(out: &mut Vec<u8>, data: &[u8]) {
    for &b in data {
        if needs_escape(b) {
            out.push(ZDLE);
            out.push(b ^ 0x40);
        } else {
            out.push(b);
        }
    }
}

pub fn pos_to_arg(pos: u32) -> [u8; 4] {
    pos.to_le_bytes()
}
pub fn arg_to_pos(arg: [u8; 4]) -> u32 {
    u32::from_le_bytes(arg)
}

/// 十六进制帧头:`**` ZDLE 'B' + 7 字节(kind+arg4+crc16)转小写 hex + CR LF(0x8a)
pub fn encode_hex_header(kind: u8, arg: [u8; 4]) -> Vec<u8> {
    let body = [kind, arg[0], arg[1], arg[2], arg[3]];
    let crc = crc16(&body);
    let mut raw = body.to_vec();
    raw.push((crc >> 8) as u8);
    raw.push((crc & 0xff) as u8);
    let mut out = vec![ZPAD, ZPAD, ZDLE, ZHEX];
    for b in raw {
        out.push(hex_nibble(b >> 4));
        out.push(hex_nibble(b & 0xf));
    }
    out.push(0x0d);
    out.push(0x8a); // LF|0x80,lrzsz 惯例
    out
}

fn hex_nibble(n: u8) -> u8 {
    if n < 10 { b'0' + n } else { b'a' + (n - 10) }
}

/// 十六进制 ascii -> nibble,失败返回 None
fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// 二进制 CRC32 帧头:`*` ZDLE 'C' + [kind+arg4] + crc32(LE),body 与 crc 经 ZDLE 转义
pub fn encode_bin32_header(kind: u8, arg: [u8; 4]) -> Vec<u8> {
    let body = [kind, arg[0], arg[1], arg[2], arg[3]];
    let crc = crc32(&body);
    let mut payload = body.to_vec();
    payload.extend_from_slice(&crc.to_le_bytes());
    let mut out = vec![ZPAD, ZDLE, ZBIN32];
    zdle_escape(&mut out, &payload);
    out
}

/// 数据子包:data 转义 + ZDLE + end,再 crc(覆盖 data 原始字节与 end)转义
pub fn encode_data_subpacket(data: &[u8], end: u8, use_crc32: bool) -> Vec<u8> {
    let mut out = Vec::new();
    zdle_escape(&mut out, data);
    out.push(ZDLE);
    out.push(end);
    if use_crc32 {
        let mut crc_input = data.to_vec();
        crc_input.push(end);
        let crc = crc32(&crc_input);
        zdle_escape(&mut out, &crc.to_le_bytes());
    } else {
        let mut crc_input = data.to_vec();
        crc_input.push(end);
        let crc = crc16(&crc_input);
        zdle_escape(&mut out, &[(crc >> 8) as u8, (crc & 0xff) as u8]);
    }
    out
}

#[derive(Debug)]
pub enum Event {
    Header { kind: u8, arg: [u8; 4] },
    Data { bytes: Vec<u8>, end: u8 },
    GarbledCrc,
}

pub struct Parser {
    buf: Vec<u8>,
    /// 收到 ZFILE/ZDATA 头后置为 Some(use_crc32),表示随后是数据子包流
    expect_data: Option<bool>,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self { buf: Vec::new(), expect_data: None }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// 收方在收到数据帧头后调用,切换到数据子包解析
    pub fn set_data_mode(&mut self, use_crc32: bool) {
        self.expect_data = Some(use_crc32);
    }

    pub fn set_header_mode(&mut self) {
        self.expect_data = None;
    }

    pub fn next(&mut self) -> Option<Event> {
        match self.expect_data {
            // 已显式切换到数据模式(收到 ZDATA/ZFILE 头之后):按其 crc 位解析子包
            Some(use_crc32) => self.parse_subpacket(use_crc32),
            // 帧模式:严格只解析帧头。子包解析必须经 set_data_mode 显式开启,
            // 不允许在帧模式下靠"首字节是否 ZPAD"猜测性地退化为子包解析。
            None => self.parse_header(),
        }
    }

    /// 解析一个帧头:在 buf 中定位 ZDLE,按其后一字节(`B`/`C`)分派到具体的
    /// 帧头解析函数;未知类型或缓冲区暂无 ZDLE 时按需跳过/等待更多数据。
    fn parse_header(&mut self) -> Option<Event> {
        // 寻找起始:ZPAD(可一个或两个)后跟 ZDLE
        let buf = &self.buf;
        let mut i = 0;
        // 跳过前导垃圾字节直到找到 ZDLE,记录 ZDLE 的位置
        while i < buf.len() && buf[i] != ZDLE {
            i += 1;
        }
        if i >= buf.len() {
            // 缓冲区里没有 ZDLE:不能直接清空整个缓冲区,否则跨 feed 到达的
            // ZPAD 前导(例如 [ZPAD] 后跟 [ZPAD, ZDLE, ...] 分两次 feed)会被
            // 提前丢弃,导致后续帧头永远拼不完整。只丢弃尾部 ZPAD 连续段
            // 之前的垃圾字节,保留这段 ZPAD 供下一次 feed 续接。
            let mut keep_from = buf.len();
            while keep_from > 0 && buf[keep_from - 1] == ZPAD {
                keep_from -= 1;
            }
            self.buf.drain(0..keep_from);
            return None;
        }
        // 需要至少 ZDLE + 类型字节
        if i + 1 >= buf.len() {
            return None;
        }
        let kind_byte = buf[i + 1];
        match kind_byte {
            ZHEX => self.parse_hex_header(i),
            ZBIN32 => self.parse_bin32_header(i),
            _ => {
                // 未知/暂不支持的类型(如 bin16 'A'),跳过这个 ZDLE 以重同步
                self.buf.drain(0..=i);
                None
            }
        }
    }

    /// 解析从 buf[start] == ZDLE 开始的十六进制头:ZDLE 'B' + 14 hex + CR + LF
    fn parse_hex_header(&mut self, start: usize) -> Option<Event> {
        let need = start + 2 + 14 + 2; // ZDLE 'B' + 14 hex chars + CR + LF
        if self.buf.len() < need {
            return None;
        }
        let hex_start = start + 2;
        let mut raw = [0u8; 7];
        for (idx, chunk) in raw.iter_mut().enumerate() {
            let hi = hex_val(self.buf[hex_start + idx * 2]);
            let lo = hex_val(self.buf[hex_start + idx * 2 + 1]);
            match (hi, lo) {
                (Some(h), Some(l)) => *chunk = (h << 4) | l,
                _ => {
                    // 非法 hex 字符,当作 CRC 错误处理并跳过起始字节重同步
                    self.buf.drain(0..=start);
                    return Some(Event::GarbledCrc);
                }
            }
        }
        let body = &raw[0..5];
        let crc = ((raw[5] as u16) << 8) | (raw[6] as u16);
        if crc16(body) != crc {
            // CRC 不符:只跳过起始的 ZPAD..ZDLE 重同步,不吞掉整帧长度,
            // 以免误伤紧随其后可能已经合法的下一帧。
            self.buf.drain(0..=start);
            return Some(Event::GarbledCrc);
        }
        // CRC 校验通过后才消费整帧(含结尾 CR LF)
        self.buf.drain(0..need);
        let kind = raw[0];
        let arg = [raw[1], raw[2], raw[3], raw[4]];
        Some(Event::Header { kind, arg })
    }

    /// 解析从 buf[start] == ZDLE 开始的 bin32 头:ZDLE 'C' + zdle_escape(5 字节 + crc32 LE)
    fn parse_bin32_header(&mut self, start: usize) -> Option<Event> {
        let payload_start = start + 2;
        match unescape_upto(&self.buf[payload_start..], 9) {
            UnescapeResult::Incomplete => None,
            UnescapeResult::Ok { unescaped, consumed } => {
                let body = &unescaped[0..5];
                let crc = u32::from_le_bytes([unescaped[5], unescaped[6], unescaped[7], unescaped[8]]);
                if crc32(body) != crc {
                    // CRC 不符:只跳过起始的 ZPAD..ZDLE 重同步,不吞掉整段 payload。
                    self.buf.drain(0..=start);
                    return Some(Event::GarbledCrc);
                }
                self.buf.drain(0..payload_start + consumed);
                let kind = body[0];
                let arg = [body[1], body[2], body[3], body[4]];
                Some(Event::Header { kind, arg })
            }
        }
    }

    /// 解析数据子包:反转义直到遇到 ZDLE + end(end ∈ ZCRCE/G/Q/W),
    /// 之后紧跟 crc(use_crc32 决定 4 字节还是 2 字节),同样经转义。
    fn parse_subpacket(&mut self, use_crc32: bool) -> Option<Event> {
        let crc_len = if use_crc32 { 4 } else { 2 };
        let mut unescaped = Vec::new();
        let mut i = 0;
        let buf_len = self.buf.len();
        loop {
            if i >= buf_len {
                return None; // 数据不足
            }
            let b = self.buf[i];
            if b == ZDLE {
                if i + 1 >= buf_len {
                    return None;
                }
                let next = self.buf[i + 1];
                if matches!(next, ZCRCE | ZCRCG | ZCRCQ | ZCRCW) {
                    // 找到终止符,接着读取 crc(经转义)
                    let end = next;
                    let crc_start = i + 2;
                    match unescape_upto(&self.buf[crc_start..], crc_len) {
                        UnescapeResult::Incomplete => return None,
                        UnescapeResult::Ok { unescaped: crc_bytes, consumed } => {
                            let total_consumed = crc_start + consumed;
                            let mut crc_input = unescaped.clone();
                            crc_input.push(end);
                            let crc_ok = if use_crc32 {
                                let crc = u32::from_le_bytes([
                                    crc_bytes[0],
                                    crc_bytes[1],
                                    crc_bytes[2],
                                    crc_bytes[3],
                                ]);
                                crc32(&crc_input) == crc
                            } else {
                                let crc = ((crc_bytes[0] as u16) << 8) | (crc_bytes[1] as u16);
                                crc16(&crc_input) == crc
                            };
                            if crc_ok {
                                self.buf.drain(0..total_consumed);
                                return Some(Event::Data { bytes: unescaped, end });
                            }
                            // CRC 不符:数据子包没有 ZPAD/ZDLE 式的帧起始标记可对齐,
                            // 只跳过缓冲区最前一个字节做最小重同步,避免吞掉紧随其后
                            // 可能仍然合法的数据。
                            self.buf.drain(0..1);
                            return Some(Event::GarbledCrc);
                        }
                    }
                } else {
                    // 普通转义字节
                    unescaped.push(next ^ 0x40);
                    i += 2;
                }
            } else {
                unescaped.push(b);
                i += 1;
            }
        }
    }
}

/// 反转义结果:成功时给出反转义后的字节与消费的原始字节数;数据不足时返回 Incomplete。
enum UnescapeResult {
    Ok { unescaped: Vec<u8>, consumed: usize },
    Incomplete,
}

/// 从 `data` 开头反转义,直到凑够 `want` 个反转义后的字节。
fn unescape_upto(data: &[u8], want: usize) -> UnescapeResult {
    let mut unescaped = Vec::with_capacity(want);
    let mut i = 0;
    while unescaped.len() < want {
        if i >= data.len() {
            return UnescapeResult::Incomplete;
        }
        let b = data[i];
        if b == ZDLE {
            if i + 1 >= data.len() {
                return UnescapeResult::Incomplete;
            }
            unescaped.push(data[i + 1] ^ 0x40);
            i += 2;
        } else {
            unescaped.push(b);
            i += 1;
        }
    }
    UnescapeResult::Ok { unescaped, consumed: i }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_known_vectors() {
        assert_eq!(crc16(b"123456789"), 0x31C3);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn zdle_escapes_control_bytes() {
        let mut out = Vec::new();
        zdle_escape(&mut out, &[0x18, b'A', 0x10]);
        // 0x18 -> ZDLE, 0x58 ; A 原样 ; 0x10 -> ZDLE, 0x50
        assert_eq!(out, vec![ZDLE, 0x58, b'A', ZDLE, 0x50]);
    }

    #[test]
    fn pos_arg_roundtrip() {
        for p in [0u32, 1, 1024, 0x0102_0304, u32::MAX] {
            assert_eq!(arg_to_pos(pos_to_arg(p)), p);
        }
    }

    #[test]
    fn hex_header_has_star_zdle_b_prefix() {
        let h = encode_hex_header(ZRINIT, [0, 0, 0, 0x23]);
        assert_eq!(&h[0..4], &[ZPAD, ZPAD, ZDLE, ZHEX]);
    }

    #[test]
    fn parser_roundtrips_bin32_header() {
        let bytes = encode_bin32_header(ZDATA, pos_to_arg(0));
        let mut p = Parser::new();
        p.feed(&bytes);
        match p.next() {
            Some(Event::Header { kind, arg }) => {
                assert_eq!(kind, ZDATA);
                assert_eq!(arg_to_pos(arg), 0);
            }
            other => panic!("expected header, got {other:?}"),
        }
    }

    #[test]
    fn parser_roundtrips_data_subpacket() {
        let bytes = encode_data_subpacket(b"hello", ZCRCE, true);
        let mut p = Parser::new();
        // 子包解析必须经 set_data_mode 显式开启,帧模式下不会退化猜测。
        p.set_data_mode(true);
        p.feed(&bytes);
        match p.next() {
            Some(Event::Data { bytes, end }) => {
                assert_eq!(bytes, b"hello");
                assert_eq!(end, ZCRCE);
            }
            other => panic!("expected data, got {other:?}"),
        }
    }

    #[test]
    fn parser_handles_split_feeds() {
        let bytes = encode_hex_header(ZRPOS, pos_to_arg(1024));
        let mut p = Parser::new();
        p.feed(&bytes[..3]);
        assert!(matches!(p.next(), None)); // 不足一帧
        p.feed(&bytes[3..]);
        match p.next() {
            Some(Event::Header { kind, arg }) => {
                assert_eq!(kind, ZRPOS);
                assert_eq!(arg_to_pos(arg), 1024);
            }
            other => panic!("expected header, got {other:?}"),
        }
    }

    #[test]
    fn parser_handles_split_feed_inside_zpad_run() {
        // 十六进制头以 `** ZDLE 'B'` 开头(两个 ZPAD)。分两次 feed,断点恰好落在
        // ZPAD 连续段内部(第 1 字节、第 2 字节处),验证前导 ZPAD 不会被
        // parse_header 提前丢弃,导致后续帧头永久拼不完整。
        for split in [1usize, 2] {
            let bytes = encode_hex_header(ZRPOS, pos_to_arg(1024));
            let mut p = Parser::new();
            p.feed(&bytes[..split]);
            assert!(matches!(p.next(), None), "split={split}: 不足一帧应返回 None");
            p.feed(&bytes[split..]);
            match p.next() {
                Some(Event::Header { kind, arg }) => {
                    assert_eq!(kind, ZRPOS, "split={split}");
                    assert_eq!(arg_to_pos(arg), 1024, "split={split}");
                }
                other => panic!("split={split}: expected header, got {other:?}"),
            }
        }
    }

    #[test]
    fn parser_reports_garbled_crc() {
        let mut bytes = encode_bin32_header(ZDATA, pos_to_arg(0));
        *bytes.last_mut().unwrap() ^= 0xff; // 破坏 CRC
        let mut p = Parser::new();
        p.feed(&bytes);
        assert!(matches!(p.next(), Some(Event::GarbledCrc)));
    }
}
