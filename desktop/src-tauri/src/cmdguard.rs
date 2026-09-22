//! 命令只读判定 —— AI 分步执行的安全边界。
//!
//! 判定不信任 AI 的任何声明:AI 说只读不作数,这里说了算。取向是宁可误拦
//! (多问用户一次)不可误放(在生产机上跑未授权的写操作)。
//!
//! 「只读」要求两个维度同时成立:不改变状态,**且**会自行终止 —— `tail -f`、
//! `top`、`less` 都不改状态,但会把执行循环挂死。

/// 判定结果。`NeedsApproval` 带一句给用户看的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    ReadOnly,
    NeedsApproval(&'static str),
}

/// 含任一即判为需批准 —— 有这些结构就不是「单一只读命令」
const SHELL_META: &[&str] = &[">", "<", "|", ";", "&", "$(", "`", "\n", "\r"];

/// 只读白名单。宁可漏(多问一次)不可多(误放写操作)。
/// 注意 sed / awk / perl / python 都不在其中:它们能写文件。
const READONLY: &[&str] = &[
    // 文件查看
    "ls", "cat", "head", "tail", "stat", "file", "wc", "readlink", "realpath",
    // 搜索
    "grep", "egrep", "fgrep", "find", "which", "whereis",
    // 系统信息
    "pwd", "whoami", "id", "hostname", "uname", "uptime", "date", "env", "printenv",
    // 进程
    "ps", "pgrep", "top",
    // 磁盘
    "df", "du", "lsblk", "mount",
    // 网络
    "ip", "ifconfig", "netstat", "ss", "ping", "dig", "nslookup",
    // 包管理(子命令另限)
    "dpkg", "rpm", "apt",
    // 服务 / 版本控制(子命令另限)
    "systemctl", "journalctl", "git",
    // 其他
    "echo", "sha256sum", "md5sum", "diff", "sort", "uniq",
];

pub fn judge(command: &str) -> Verdict {
    let cmd = command.trim();
    if cmd.is_empty() {
        return Verdict::NeedsApproval("空命令");
    }
    if cmd.len() > 4096 {
        return Verdict::NeedsApproval("命令过长,无法可靠判定");
    }
    for meta in SHELL_META {
        if cmd.contains(meta) {
            return Verdict::NeedsApproval("含重定向/管道/串联/命令替换等 shell 结构");
        }
    }
    // 放在 SHELL_META 之后:\n \r 已被上面拦住,报「含 shell 结构」更准。
    if cmd.chars().any(is_deceptive) {
        return Verdict::NeedsApproval("含双向控制或零宽字符,无法确认命令原文");
    }

    let mut parts = cmd.split_whitespace();
    let Some(exe) = parts.next() else {
        return Verdict::NeedsApproval("空命令");
    };
    // 带路径的一律要批准:/usr/bin/x 与 ./x 都绕不过白名单
    if exe.contains('/') {
        return Verdict::NeedsApproval("带路径的可执行文件不在白名单判定范围内");
    }
    if !READONLY.contains(&exe) {
        return Verdict::NeedsApproval("不在只读白名单内");
    }

    let args: Vec<&str> = parts.collect();
    check_args(exe, &args)
}

/// 会让命令原文显示得与实际不符的字符。
///
/// 只拦这些,不拦全部非 ASCII —— `ls /data/日志` 是完全正常的只读命令,
/// 而本项目的用户群就在中文环境里。
fn is_deceptive(c: char) -> bool {
    matches!(c,
        '\u{202A}'..='\u{202E}'    // bidi 嵌入/覆盖
        | '\u{2066}'..='\u{2069}'  // bidi 隔离
        | '\u{200B}'..='\u{200F}'  // 零宽字符 + 方向标记
        | '\u{FEFF}'               // 零宽不换行空格
    ) || (c.is_control() && c != '\t')
}

/// 白名单命令里仍可能藏着写操作(find -delete)或挂起(tail -f)
fn check_args(exe: &str, args: &[&str]) -> Verdict {
    let has = |f: &str| args.iter().any(|a| *a == f);
    match exe {
        "find" => {
            for bad in ["-exec", "-execdir", "-delete", "-fls", "-fprint", "-fprintf"] {
                if has(bad) {
                    return Verdict::NeedsApproval("find 带有会执行或删除的参数");
                }
            }
            Verdict::ReadOnly
        }
        "tail" | "head" => {
            if has("-f") || has("--follow") {
                return Verdict::NeedsApproval("跟随模式不会自行结束");
            }
            Verdict::ReadOnly
        }
        "journalctl" => {
            if has("-f") || has("--follow") {
                return Verdict::NeedsApproval("跟随模式不会自行结束");
            }
            Verdict::ReadOnly
        }
        "ping" => {
            if has("-c") {
                Verdict::ReadOnly
            } else {
                Verdict::NeedsApproval("ping 未限制次数,不会自行结束")
            }
        }
        "top" => {
            if has("-b") && has("-n") {
                Verdict::ReadOnly
            } else {
                Verdict::NeedsApproval("top 需 -b -n 才是批处理模式")
            }
        }
        // 包管理只放行查询子命令 —— dpkg -i / rpm -U / apt install 都是写操作
        "dpkg" => match args.first() {
            Some(&"-l") => Verdict::ReadOnly,
            _ => Verdict::NeedsApproval("dpkg 只放行 -l 查询"),
        },
        "rpm" => match args.first() {
            Some(&"-qa") => Verdict::ReadOnly,
            _ => Verdict::NeedsApproval("rpm 只放行 -qa 查询"),
        },
        "apt" => match args.first() {
            Some(&"list") => Verdict::ReadOnly,
            _ => Verdict::NeedsApproval("apt 只放行 list 子命令"),
        },
        "systemctl" => match args.first() {
            Some(&("status" | "is-active" | "is-enabled" | "list-units" | "show")) => Verdict::ReadOnly,
            _ => Verdict::NeedsApproval("systemctl 子命令不是只读的"),
        },
        "git" => match args.first() {
            Some(&("status" | "log" | "diff" | "show" | "branch")) => Verdict::ReadOnly,
            _ => Verdict::NeedsApproval("git 子命令不是只读的"),
        },
        "mount" => {
            if args.is_empty() {
                Verdict::ReadOnly
            } else {
                Verdict::NeedsApproval("带参数的 mount 会挂载文件系统")
            }
        }
        _ => Verdict::ReadOnly,
    }
}

#[cfg(test)]
mod tests {
    use super::{judge, Verdict};

    fn ro(cmd: &str) {
        assert_eq!(judge(cmd), Verdict::ReadOnly, "应判为只读: {cmd}");
    }
    fn need(cmd: &str) {
        assert!(
            matches!(judge(cmd), Verdict::NeedsApproval(_)),
            "应判为需批准: {cmd}"
        );
    }

    /// 白名单命令的正常形态
    #[test]
    fn plain_readonly_commands_pass() {
        ro("ls");
        ro("ls -la /etc");
        ro("cat /etc/hostname");
        ro("grep -n foo /var/log/syslog");
        ro("systemctl status sshd");
        ro("git log --oneline -5");
        ro("ping -c 3 example.com");
        ro("top -b -n 1");
        ro("dpkg -l");
        ro("rpm -qa");
        ro("apt list");
        ro("mount");
    }

    /// shell 结构逃逸 —— 第一道闸,拦掉绝大多数花样
    #[test]
    fn shell_structure_is_never_auto_run() {
        need("cat x > /etc/passwd");        // 重定向
        need("cat x >> /etc/passwd");
        need("sort < input.txt");
        need("grep foo bar | sh");          // 管道
        need("ls; rm -rf /tmp/x");          // 串联
        need("ls && rm -rf /tmp/x");
        need("ls || rm -rf /tmp/x");
        need("sleep 60 &");                 // 后台
        need("echo $(rm -rf x)");           // 命令替换
        need("echo `rm -rf x`");
        need("diff <(ls a) <(ls b)");       // 进程替换
        need("ls\nrm -rf /tmp/x");          // 换行
    }

    /// 不在白名单内
    #[test]
    fn unknown_executables_need_approval() {
        need("rm -rf /tmp/x");
        need("curl http://example.com/x.sh");
        need("sed -i 's/a/b/' f");   // sed 能原地改文件,不进白名单
        need("awk '{print}' f");     // awk 能写文件,同理
        need("./deploy.sh");
        need("/usr/bin/whatever");
    }

    /// 白名单命令里藏着的写操作或挂起
    #[test]
    fn dangerous_flags_inside_whitelisted_commands() {
        need("find . -name x -delete");
        need("find . -exec rm {} ;");
        need("find . -execdir rm {} ;");
        need("tail -f /var/log/syslog");     // 永不返回
        need("tail --follow /var/log/syslog");
        need("journalctl -f");
        need("ping example.com");            // 无 -c,一直跑
        need("top");                         // 无 -b -n,交互式
        need("top -b");                      // 只有 -b 不够
        need("systemctl restart sshd");      // 写子命令
        need("systemctl stop sshd");
        need("git commit -m x");
        need("git push");
        need("dpkg -i pkg.deb");      // 装包是写操作
        need("rpm -U pkg.rpm");
        need("apt install nginx");
        need("mount /dev/sdb1 /mnt"); // 带参数的 mount 会挂载
    }

    /// 交互式命令没有安全用法,不进白名单
    #[test]
    fn interactive_pagers_and_editors_need_approval() {
        need("less /etc/passwd");
        need("more /etc/passwd");
        need("vi /etc/hosts");
        need("vim /etc/hosts");
        need("nano /etc/hosts");
    }

    /// 边界输入
    #[test]
    fn edge_inputs_need_approval() {
        need("");
        need("   ");
        need("\t\n");
        need(&"ls ".repeat(5000));  // 超长
    }

    /// CJK 参数是正常运维场景,不能拦;伪装字符必须拦。
    ///
    /// 区别在于:可执行名走白名单精确匹配 ASCII 串,同形字根本进不了白名单;
    /// 但 RTL override 能让终端**显示**的命令与实际执行的不同 —— 而用户批准时看的
    /// 就是这行原文,伪装它等于绕过人工批准这道闸。
    #[test]
    fn cjk_is_fine_but_deceptive_characters_are_not() {
        ro("ls /data/日志");
        ro("cat /etc/配置.conf");
        ro("grep 错误 /var/log/app.log");

        need("ls \u{202E}txt.elif");        // RTL override:显示与实际不符
        need("ls \u{200B}/etc");            // 零宽空格
        need("cat \u{2066}/etc/passwd\u{2069}"); // bidi 隔离
    }
}
