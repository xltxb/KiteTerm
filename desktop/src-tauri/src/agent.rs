//! AI 分步执行:输出哨兵与单步编排。
//!
//! 命令尾部挂一个 APC 序列(`ESC _ … ESC \`)包裹的退出码 —— xterm.js 会整段
//! 忽略它,故用户在终端上看不见这个标记。手法与 ssh.rs 的 DROP_RZ_COMMAND 相同,
//! 那里用它探测远端 cwd;同样用 `\033` 八进制而非裸 ESC 字节,shell 回显命令行
//! 时才不会夹带控制字符。

pub const END_MARK_HEAD: &[u8] = b"\x1b_KTEND:";
pub const END_MARK_TAIL: &[u8] = b"\x1b\\";

/// 缓冲上限:远超任何合理的单步输出,涨到这里说明流里根本没有标记
const PROBE_CAP: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepOutput {
    pub output: String,
    pub exit_code: i32,
}

/// 给命令挂上隐形哨兵。`$?` 取的是前一条命令的退出码,故必须用 `;` 分隔
/// (而非 `&&` —— 那样失败时哨兵根本不会执行,循环会一直等到超时)。
///
/// `auto` 为真时额外做非交互加固。执行走的是真实 PTY,而自动执行的命令
/// 没人盯着 —— `git log` 长输出会进 `less` 等一个永远不会来的按键,`cat`
/// 不带文件名会读 stdin 等一个永远不会来的 EOF。这两类挂死不该在 cmdguard
/// 里逐条打地鼠:判定管「能不能自动跑」,这里管「自动跑的能跑完」。
///
/// 批准执行的命令**不加** —— `sudo` 要从 tty 读密码,断掉 stdin 它就直接
/// 失败了;而用户正看着屏幕,交互他自己能处理。
pub fn wrap_command(cmd: &str, auto: bool) -> String {
    let body = if auto {
        // 前缀赋值只对这条命令生效;三个分页变量分别管通用、git、systemd
        format!("PAGER=cat GIT_PAGER=cat SYSTEMD_PAGER=cat {cmd} < /dev/null")
    } else {
        cmd.to_string()
    };
    format!("{body}; printf '\\033_KTEND:%s\\033\\\\' \"$?\"\r")
}

#[derive(Default)]
pub struct EndProbe {
    buf: Vec<u8>,
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

impl EndProbe {
    /// 喂入一块远端输出;凑齐完整标记时返回这一步的输出与退出码(并清空缓冲)。
    pub fn push(&mut self, data: &[u8]) -> Option<StepOutput> {
        self.buf.extend_from_slice(data);
        if self.buf.len() > PROBE_CAP {
            // 涨到这里说明流里根本没有标记(比如用户手动 Ctrl-C 掉了命令),
            // 丢掉前半截防止无界增长,保留尾部以便后续标记仍能被认出
            let keep = self.buf.len() - PROBE_CAP / 2;
            self.buf.drain(..keep);
        }
        let head = find_sub(&self.buf, END_MARK_HEAD)?;
        let from = head + END_MARK_HEAD.len();
        let tail = find_sub(&self.buf[from..], END_MARK_TAIL)?;

        let output = String::from_utf8_lossy(&self.buf[..head]).into_owned();
        let code_raw = String::from_utf8_lossy(&self.buf[from..from + tail]).into_owned();
        // 解析不出来就当失败:宁可报错也不要谎报成功
        let exit_code = code_raw.trim().parse::<i32>().unwrap_or(-1);
        self.buf.clear();
        Some(StepOutput { output, exit_code })
    }
}

#[cfg(test)]
mod tests {
    use super::{EndProbe, StepOutput, wrap_command};

    #[test]
    fn wrap_appends_apc_sentinel() {
        let w = wrap_command("ls -la", false);
        assert!(w.starts_with("ls -la; printf "), "原命令须在前: {w}");
        assert!(w.contains("_KTEND:"), "须带哨兵标记: {w}");
        assert!(w.contains("\\033"), "须用八进制 ESC 而非裸字节: {w}");
        assert!(w.ends_with('\r'), "须以回车结尾,否则 shell 不会执行");
    }

    /// 自动执行的命令必须完全非交互 —— 没人盯着它。
    ///
    /// 执行走的是真实 PTY(ssh.rs 的 request_pty),所以:`git log` 这类
    /// 长输出会进 `less` 等按键,而按键永远不会来;`cat` 不带文件名会读
    /// stdin,在活的会话里等不到 EOF。两者都把循环挂到 30 秒超时。
    #[test]
    fn auto_commands_are_made_non_interactive() {
        let w = wrap_command("git log", true);
        assert!(w.contains("PAGER=cat"), "须禁掉分页器: {w}");
        assert!(w.contains("GIT_PAGER=cat"), "git 有自己的分页变量: {w}");
        assert!(w.contains("SYSTEMD_PAGER=cat"), "systemctl 也有: {w}");
        assert!(w.contains("< /dev/null"), "须把 stdin 接到 /dev/null: {w}");
    }

    /// 但用户批准的命令不能加这些。
    ///
    /// spec 的示例场景就是 `sudo sed -i ...` —— sudo 要从 tty 读密码,
    /// `< /dev/null` 会让它直接失败。批准的命令用户自己盯着,能处理交互。
    #[test]
    fn approved_commands_keep_their_tty() {
        let w = wrap_command("sudo sed -i 's/a/b/' /etc/hosts", false);
        assert!(!w.contains("/dev/null"), "批准的命令不能断掉 stdin: {w}");
        assert!(!w.contains("PAGER"), "也不必改分页器: {w}");
        assert!(w.contains("_KTEND:"), "但哨兵照旧要有: {w}");
    }

    #[test]
    fn probe_returns_output_and_code_in_one_chunk() {
        let mut p = EndProbe::default();
        let got = p.push(b"total 8\nfile.txt\n\x1b_KTEND:0\x1b\\");
        assert_eq!(
            got,
            Some(StepOutput { output: "total 8\nfile.txt\n".into(), exit_code: 0 })
        );
    }

    #[test]
    fn probe_joins_chunks_split_mid_marker() {
        // 标记被切成两半 —— 终端流按任意边界分块,这是常态而非边缘情况
        let mut p = EndProbe::default();
        assert_eq!(p.push(b"out\n\x1b_KTE"), None);
        let got = p.push(b"ND:1\x1b\\");
        assert_eq!(got, Some(StepOutput { output: "out\n".into(), exit_code: 1 }));
    }

    #[test]
    fn probe_keeps_accumulating_until_marker() {
        let mut p = EndProbe::default();
        assert_eq!(p.push(b"line1\n"), None);
        assert_eq!(p.push(b"line2\n"), None);
        let got = p.push(b"\x1b_KTEND:127\x1b\\").unwrap();
        assert_eq!(got.output, "line1\nline2\n");
        assert_eq!(got.exit_code, 127);
    }

    #[test]
    fn probe_resets_after_yielding() {
        let mut p = EndProbe::default();
        p.push(b"a\x1b_KTEND:0\x1b\\");
        // 第二步的输出不应带上第一步的残留
        let got = p.push(b"b\x1b_KTEND:0\x1b\\").unwrap();
        assert_eq!(got.output, "b");
    }

    #[test]
    fn probe_tolerates_unparsable_code() {
        // 退出码位置出现非数字:不 panic,当作失败处理
        let mut p = EndProbe::default();
        let got = p.push(b"x\x1b_KTEND:abc\x1b\\").unwrap();
        assert_eq!(got.exit_code, -1);
    }
}
