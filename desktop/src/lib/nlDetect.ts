/**
 * 判断终端里输入的一行是"白话"还是 shell 命令。
 *
 * 取舍:宁可漏判(当成命令照常执行)也不要误判 —— 误判会打断正常操作,
 * 而漏判只是少一次便利。因此除显式 `?` 前缀外,规则都偏保守。
 *
 * 注意 find / show / start / restart 这类词既是常见命令又是英文意图词,
 * 一律按"命令优先"处理;要对这类开头的句子走 AI,加 `?` 前缀即可。
 */

/** 常见命令 —— 首词命中则一定按命令执行 */
export const KNOWN_COMMANDS = new Set([
  "ls", "cd", "pwd", "cat", "less", "more", "head", "tail", "grep", "rg", "find", "fd",
  "cp", "mv", "rm", "mkdir", "rmdir", "touch", "ln", "chmod", "chown", "du", "df", "stat",
  "ps", "top", "htop", "kill", "killall", "pkill", "lsof", "netstat", "ss", "ping", "curl",
  "wget", "ssh", "scp", "rsync", "tar", "zip", "unzip", "gzip", "git", "docker", "kubectl",
  "systemctl", "service", "journalctl", "apt", "apt-get", "yum", "dnf", "brew", "pacman",
  "npm", "pnpm", "yarn", "node", "python", "python3", "pip", "pip3", "cargo", "go", "java",
  "make", "cmake", "gcc", "vim", "vi", "nano", "emacs", "echo", "printf", "export", "source",
  "alias", "which", "whereis", "man", "history", "clear", "exit", "sudo", "su", "env", "set",
  "awk", "sed", "sort", "uniq", "wc", "cut", "tr", "xargs", "tee", "diff", "patch", "date",
  "uptime", "uname", "whoami", "id", "groups", "mount", "umount", "fdisk", "crontab", "nc",
  "dig", "nslookup", "traceroute", "ip", "ifconfig", "iptables", "nginx", "pm2", "psql",
  "mysql", "redis-cli", "mongo", "sqlite3", "tmux", "screen", "watch", "sleep", "time",
  "bash", "sh", "zsh", "fish", "code", "open", "say", "pbcopy", "jq", "yq", "helm", "terraform",
]);

/** 英文自然语言的意图词开头 */
const INTENT_WORDS =
  /^(how|what|why|where|which|who|find|show|list|check|create|delete|remove|install|start|stop|restart|kill|search|count|copy|move|rename|tell|give|help|can|please)\b/i;

/** 命中即视为 shell 语法,不再考虑白话 */
const SHELL_SYNTAX = /[|;&><$`]|\.\/|\s--|\s-[a-zA-Z]/;

export function looksLikeNaturalLanguage(line: string): boolean {
  const s = line.trim();
  if (s.length < 4) return false;
  // 显式前缀:? 开头一定是问 AI
  if (s.startsWith("?")) return true;
  if (SHELL_SYNTAX.test(s)) return false;

  const words = s.split(/\s+/);
  const first = words[0].toLowerCase();
  if (KNOWN_COMMANDS.has(first)) return false;
  // 路径、含扩展名、含等号(环境变量赋值)更像命令
  if (first.includes("/") || /\.[a-z0-9]{1,4}$/i.test(first) || first.includes("=")) return false;

  // 中文基本不会是命令
  if (/[一-龥]/.test(s)) return true;
  // 英文:需要多个词且以意图词开头
  return words.length >= 3 && INTENT_WORDS.test(s);
}

/**
 * 从终端整行文本里剥掉 shell 提示符,取用户输入的部分。
 *
 * 字符集含 ❯ ➜ » —— starship、oh-my-zsh 这些主题的提示符符号不是 ASCII 的 >,
 * 漏了它们整行提示符都会被当成用户输入。
 *
 * 提示符字符后面的空白是可选的:xterm 的 translateToString(true) 会 trim 掉尾部空白,
 * 所以空行的光标行读出来就是「❯」而不是「❯ 」。
 *
 * **认不出提示符时返回空串,而不是返回整行** —— 不知道哪里是用户输入就别猜。
 * 与本文件的既定取向一致:宁可漏判(少一次便利)也不要误判(把提示符当成用户的话
 * 送给 AI,或者让「交给 AI」按钮在空行上也亮起来)。
 */
export function stripPrompt(rawLine: string): string {
  const m = rawLine.match(/^.*?[$#%>❯➜»](?:\s(.*))?$/);
  if (!m) return "";
  let s = m[1] ?? "";
  // 右提示符(zsh 的 RPROMPT)渲染在行的最右端,与用户输入之间是一大片填充空格。
  // 人不会连着敲这么多空格,以此为界切掉 —— 否则「❯␣␣␣…␣␣~」这种空行会被读成输入「~」。
  // 代价是命令里若真有 4 个以上连续空格(如 echo "a    b")会被截断,换取的是空行不误触发。
  const rprompt = s.search(/ {4,}/);
  if (rprompt >= 0) s = s.slice(0, rprompt);
  return s.trim();
}
