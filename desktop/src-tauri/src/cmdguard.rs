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

/// 含任一即判为需批准 —— 有这些结构就不是「单一只读命令」。
/// `#` 在列是因为它会把后面追加的哨兵整段变成注释,那一步就永远等不到结束。
const SHELL_META: &[&str] = &[">", "<", "|", ";", "&", "$(", "`", "\n", "\r", "#"];

/// 只读白名单。宁可漏(多问一次)不可多(误放写操作)。
/// 注意 sed / awk / perl / python 都不在其中:它们能写文件。
const READONLY: &[&str] = &[
    // 文件查看
    "ls", "cat", "head", "tail", "stat", "file", "wc", "readlink", "realpath",
    // 搜索
    "grep", "egrep", "fgrep", "find", "which", "whereis",
    // 系统信息(env 不在此列:它是命令执行器,env rm -rf x 会真的删)
    "pwd", "whoami", "id", "hostname", "uname", "uptime", "date", "printenv",
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
        | '\u{2028}' | '\u{2029}'  // 行/段分隔符 —— 是 Zl/Zp,is_control() 不认
        | '\u{FEFF}'               // 零宽不换行空格
    ) || (c.is_control() && c != '\t')
}

/// 参数里是否出现某个短选项。
///
/// Unix 短选项可以粘连:`tail -vf` 与 `tail -v -f` 等价。用精确相等判断会漏掉
/// 粘连形式 —— `tail -vf` 被当成没有 -f,于是自动跑一条永不返回的命令。
fn has_short(args: &[&str], flag: char) -> bool {
    args.iter().any(|a| {
        a.starts_with('-') && !a.starts_with("--") && a.chars().skip(1).any(|c| c == flag)
    })
}

/// 参数里是否出现某个长选项,含 `--opt=value` 粘连形式
fn has_long(args: &[&str], name: &str) -> bool {
    args.iter().any(|a| *a == name || a.starts_with(&format!("{name}=")))
}

/// 取 `-n <值>` 或 `-n<值>` 的数值,拿不到就 None
fn n_value(args: &[&str]) -> Option<u32> {
    for (i, a) in args.iter().enumerate() {
        if *a == "-n" {
            return args.get(i + 1)?.parse().ok();
        }
        if let Some(rest) = a.strip_prefix("-n") {
            if !rest.is_empty() && !rest.starts_with('-') {
                return rest.parse().ok();
            }
        }
    }
    None
}

/// 白名单命令里仍可能藏着写操作(sort -o)、执行(find -exec)或挂起(tail -f)
fn check_args(exe: &str, args: &[&str]) -> Verdict {
    match exe {
        "find" => {
            // -fprint0 / -ok / -okdir 都是真实原语:前者写文件,后两者是交互式 -exec
            for bad in [
                "-exec", "-execdir", "-ok", "-okdir", "-delete",
                "-fls", "-fprint", "-fprint0", "-fprintf",
            ] {
                if args.contains(&bad) {
                    return Verdict::NeedsApproval("find 带有会执行、删除或写文件的参数");
                }
            }
            Verdict::ReadOnly
        }
        // -F 是 follow with retry,和 -f 一样不会结束
        "tail" | "head" | "journalctl" => {
            if has_short(args, 'f') || has_short(args, 'F') || has_long(args, "--follow") {
                return Verdict::NeedsApproval("跟随模式不会自行结束");
            }
            Verdict::ReadOnly
        }
        "ping" => {
            if has_short(args, 'c') {
                Verdict::ReadOnly
            } else {
                Verdict::NeedsApproval("ping 未限制次数,不会自行结束")
            }
        }
        "top" => {
            if !has_short(args, 'b') {
                return Verdict::NeedsApproval("top 需 -b 批处理模式");
            }
            // -n 的值必须是正整数:-n 0 的行为依实现而定,不赌
            match n_value(args) {
                Some(n) if n >= 1 => Verdict::ReadOnly,
                _ => Verdict::NeedsApproval("top 需 -n 指定正整数轮数才会自行结束"),
            }
        }
        // date -s / --set 会改系统时钟
        "date" => {
            if has_short(args, 's') || has_long(args, "--set") {
                return Verdict::NeedsApproval("date 带 -s/--set 会修改系统时钟");
            }
            Verdict::ReadOnly
        }
        // sort -o / --output 把结果写进任意文件
        "sort" => {
            if has_short(args, 'o') || has_long(args, "--output") {
                return Verdict::NeedsApproval("sort 带 -o/--output 会写文件");
            }
            Verdict::ReadOnly
        }
        // uniq 的第二个位置参数就是输出文件
        "uniq" => {
            if args.iter().filter(|a| !a.starts_with('-')).count() > 1 {
                return Verdict::NeedsApproval("uniq 的第二个位置参数是输出文件");
            }
            Verdict::ReadOnly
        }
        // hostname 带名字是改主机名;-F/--file 从文件读取再设置,同样是写操作,
        // 而且 -Ffile 粘连写法整个 token 以 - 开头,躲得过下面那条位置参数检查
        "hostname" => {
            if has_short(args, 'F') || has_long(args, "--file") {
                return Verdict::NeedsApproval("hostname -F/--file 会修改主机名");
            }
            if args.iter().any(|a| !a.starts_with('-')) {
                return Verdict::NeedsApproval("带参数的 hostname 会修改主机名");
            }
            Verdict::ReadOnly
        }
        // ip 能改网络;-b/-batch 还是个命令执行通道(从输入流读 ip 命令逐条执行,
        // 没有输入时挂起)。动词只看对象紧后那一位,不扫全表 ——
        // `ip addr add ... label list` 的 label 值可以是任意字符串,扫全表会被骗。
        "ip" => {
            if args.iter().any(|a| a.starts_with("-b")) {
                return Verdict::NeedsApproval("ip -b/-batch 会从输入流执行任意 ip 命令");
            }
            // iproute2 的对象名是前缀匹配:ip mo / ip mon / ip moni 都等价于
            // ip monitor,精确相等拦不住。monitor 持续监听 netlink 事件,永不返回。
            if args.first().is_some_and(|a| !a.is_empty() && "monitor".starts_with(a)) {
                return Verdict::NeedsApproval("ip monitor 持续监听,不会自行结束");
            }
            match args.get(1) {
                None => Verdict::ReadOnly, // `ip` / `ip addr`:省略动词时隐含 show
                Some(&("show" | "list" | "lst" | "sh")) => Verdict::ReadOnly,
                _ => Verdict::NeedsApproval("ip 只放行 <对象> show/list 查询"),
            }
        }
        // ifconfig 无参数或只给接口名是查询,再多就可能是 up/down/addr 配置
        "ifconfig" => {
            if args.len() <= 1 {
                Verdict::ReadOnly
            } else {
                Verdict::NeedsApproval("带配置参数的 ifconfig 会改网络状态")
            }
        }
        // netstat -c(Linux)与 -w <秒>(BSD/macOS)都会持续输出,永不返回。
        // 注意 -w 会误伤 Linux net-tools 的 --raw 简写(显示 raw socket,只读),
        // 接受这个误拦 —— 多问一次比挂到 30 秒超时强。
        "netstat" => {
            if has_short(args, 'c') || has_long(args, "--continuous") {
                return Verdict::NeedsApproval("netstat -c 会持续输出,不会自行结束");
            }
            if has_short(args, 'w') {
                return Verdict::NeedsApproval("netstat -w 按间隔持续输出(BSD/macOS)");
            }
            Verdict::ReadOnly
        }
        // ss -K 会强杀连接;-E 持续显示 socket 事件,永不返回
        "ss" => {
            if has_short(args, 'K') || has_long(args, "--kill") {
                return Verdict::NeedsApproval("ss -K 会强杀连接");
            }
            if has_short(args, 'E') || has_long(args, "--events") {
                return Verdict::NeedsApproval("ss -E 会持续输出,不会自行结束");
            }
            Verdict::ReadOnly
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
            Some(&("status" | "is-active" | "is-enabled" | "list-units" | "show")) => {
                Verdict::ReadOnly
            }
            _ => Verdict::NeedsApproval("systemctl 子命令不是只读的"),
        },
        "git" => {
            // --output= 能把 diff 写进任意文件
            if has_long(args, "--output") {
                return Verdict::NeedsApproval("git --output 会写文件");
            }
            // branch 不只是列出,git branch <name> 会建分支,-d 会删
            match args.first() {
                Some(&("status" | "log" | "diff" | "show")) => Verdict::ReadOnly,
                _ => Verdict::NeedsApproval("git 子命令不是只读的"),
            }
        }
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
        ro("git status");
        ro("git diff");
        ro("git show HEAD");
        ro("systemctl is-active sshd");
        ro("systemctl is-enabled sshd");
        ro("systemctl list-units");
        ro("systemctl show sshd");
        ro("ip addr show");
        ro("ip route");
        ro("ip");
        ro("ip addr");
        ro("ip link show");
        ro("ifconfig");
        ro("ss -tunlp");
        ro("hostname");
        ro("hostname -f");   // -f 是 FQDN 查询,小写,只读
        ro("date");
        ro("date -u");
        ro("sort file.txt");
        ro("uniq file.txt");
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
        need("ls -la # 看看目录");          // 行内注释会把追加的哨兵吞掉
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
        need("env rm -rf /tmp/x");   // env 是命令执行器,不进白名单
        need("env");
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
        need("find . -fls out.txt");
        need("find . -fprint out.txt");
        need("find . -fprint0 out.txt");
        need("find . -fprintf out.txt %p");
        need("find . -ok rm {} ;");
        need("find . -okdir rm {} ;");
        need("git branch new-feature");      // 建分支是写操作
        need("git branch -d old");
        need("git diff --output=/tmp/x");    // 写任意文件
        need("date -s '2020-01-01'");        // 改系统时钟
        need("date --set=2020-01-01");
        need("sort -o /etc/passwd f");       // 写任意文件
        need("sort --output=/tmp/x f");
        need("uniq in.txt out.txt");         // 第二个位置参数是输出文件
        need("hostname newname");            // 改主机名
        need("ip link set eth0 down");       // 改网络状态
        need("ip addr add 1.2.3.4/24 dev eth0");
        need("ip -b");                        // 从 stdin 读 ip 命令执行 + 挂起
        need("ip -batch /tmp/cmds");
        need("ip addr add 1.2.3.4/24 dev eth0 label list"); // label 值骗过 show/list 扫描
        need("hostname -F /etc/hostname");    // 从文件读取并设置
        need("hostname -F/etc/hostname");     // 粘连写法
        need("hostname --file=/etc/hostname");
        need("ifconfig eth0 down");
        need("ss -K dst 1.2.3.4");           // 强杀连接
        need("top -b -n 0");                 // 0 轮行为依实现而定
        need("top -b");                      // 无 -n
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
        need("ls \u{2028}pwd");             // 行分隔符,is_control 不认
        need("ls \u{2029}pwd");             // 段分隔符
    }

    /// 粘连短选项不能绕过检测。
    ///
    /// `tail -vf` 与 `tail -v -f` 等价,但参数 token 是 "-vf"。用精确相等判断会
    /// 漏掉它,于是一条永不返回的命令被判为只读、自动执行 —— 循环就挂在那里了。
    #[test]
    fn clustered_short_options_are_not_bypassable() {
        need("tail -vf /var/log/syslog");
        need("tail -nf 5 /var/log/syslog");
        need("tail -F /var/log/syslog");      // follow with retry,同样不结束
        need("journalctl -xef");
        need("sort -rno /etc/passwd f");      // -o 粘在 -rn 后面

        // 不能误伤:这些短选项里没有被禁的那个字母
        ro("tail -n 20 /var/log/syslog");
        ro("tail -c 100 file");
        ro("head -n 5 file");
        ro("ping -i 1 -c 3 example.com");
    }

    /// 规则存在但原先无测试覆盖的几处 —— 改错或删掉时要能挂
    #[test]
    fn previously_uncovered_rules() {
        need("ls\rrm -rf /tmp/x");          // \r 也是 SHELL_META
        need("ls \u{FEFF}/etc");            // 零宽不换行空格
        need("ls \u{1B}[2J/etc");           // ANSI 转义,is_control 分支
    }

    /// 规则自身写宽导致的逃逸 —— 这两个洞都是为了少写几行代码放宽判定造成的。
    ///
    /// `ip -b` 栽在「参数少就放行」的短路上,`label list` 栽在「扫全表找关键词」上。
    /// 判定子命令要认位置,别认关键词出现过没有。
    #[test]
    fn loose_rule_shapes_do_not_leak() {
        // 参数少不等于安全:-batch 本身就是命令执行通道
        need("ip -b");
        need("ip -batch");
        // 关键词出现在任意位置不等于它是子命令
        need("ip addr add 1.2.3.4/24 dev eth0 label show");
        need("ip route del default label list");
        // 正常查询仍要放行
        ro("ip addr show");
        ro("ip link");
    }

    /// 不改状态、但永不返回的命令 —— 「会自行终止」是只读判定的第二个维度。
    ///
    /// 这类命令放行的后果不是数据被改,而是执行循环挂在那里等到超时。
    /// 前面 tail -f / top / ping 已各有规则,这几条是同一类里容易漏掉的。
    #[test]
    fn harmless_but_endless_commands_need_approval() {
        need("ip monitor");                 // 持续监听 netlink 事件
        need("ip mo");                      // iproute2 前缀匹配,等价于 ip monitor
        need("ip mon");
        need("ip monito");
        need("netstat -c");                 // 每秒重复输出
        need("netstat --continuous");
        need("netstat -w 1");               // BSD/macOS 的 interval 模式
        need("ss -E");                      // 持续显示 socket 事件
        need("ss --events");

        // 同名命令的正常形态仍要放行
        ro("netstat -tunlp");
        ro("ss -tunlp");
        ro("ip addr show");
    }
}
