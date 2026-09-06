/** 常见失败输出的行首/行内模式 —— 只看最近几行，命中即提示「AI 诊断」 */
const PATTERNS: RegExp[] = [
  /command not found/i,
  /permission denied/i,
  /no such file or directory/i,
  /^\s*(error|fatal|panic)(\[[A-Z0-9]+\])?\s*:/i,
  /^\s*[A-Za-z]+Error:/,           // NameError: / TypeError:
  /^\s*E:\s/,                      // apt
  /^\s*npm ERR!/,
  /Traceback \(most recent call last\)/,
  /segmentation fault/i,
  /connection refused/i,
  /exit(ed)? (with )?(code|status) [1-9]\d*/i,
];

export function looksLikeError(lines: string[]): boolean {
  const recent = lines.slice(-6).map((l) => l.trimEnd()).filter((l) => l.length > 0);
  return recent.some((l) => PATTERNS.some((p) => p.test(l)));
}
