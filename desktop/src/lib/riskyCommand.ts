/** 命中这些模式的命令给出显著风险提示。
 *
 *  与 cmdguard 的只读判定是两回事:那个决定「能不能自动执行」,这个决定
 *  「批准时要不要多拦一道」。一条命令可以既非只读、又不破坏性(touch),
 *  也可以两者都是(rm -rf)。 */
const RISKY = [
  /\brm\s+-[a-zA-Z]*[rf]/, /\bmkfs\b/, /\bdd\s+if=/, /:\s*\(\)\s*\{.*\};\s*:/,
  /\bshutdown\b/, /\breboot\b/, /\bhalt\b/, /\bkill(all)?\s+-9\b/,
  /\bchmod\s+-R\s+777\b/, /\bchown\s+-R\b/, /\bdrop\s+(table|database)\b/i,
  /\btruncate\b/, />\s*\/dev\/[sh]d[a-z]/, /\bgit\s+push\s+.*--force/,
];

export function isRisky(command: string): boolean {
  return RISKY.some((re) => re.test(command));
}
