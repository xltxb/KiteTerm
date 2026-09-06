/**
 * 终端错误识别用例：node --experimental-strip-types src/lib/errorDetect.test.ts
 */
import { looksLikeError } from "./errorDetect.ts";

let failed = 0;
function expect(lines: string[], want: boolean, why: string) {
  const got = looksLikeError(lines);
  if (got !== want) { failed++; console.error(`  ✗ ${JSON.stringify(lines)} → ${got}，应为 ${want}（${why}）`); }
}

expect(["zsh: command not found: kubeclt", "$ "], true, "command not found");
expect(["bash: /etc/shadow: Permission denied"], true, "permission denied");
expect(["ls: cannot access 'nope': No such file or directory"], true, "no such file");
expect(["error[E0425]: cannot find value `x` in this scope"], true, "编译器 error");
expect(["fatal: not a git repository (or any of the parent directories): .git"], true, "git fatal");
expect(["Traceback (most recent call last):", "  File \"a.py\", line 1", "NameError: name 'x' is not defined"], true, "python traceback");
expect(["E: Unable to locate package foo"], true, "apt E:");
expect(["total 8", "drwxr-xr-x 2 u u 4096 Sep 6 10:00 ."], false, "普通 ls 输出");
expect(["Building... done", "$ "], false, "正常完成");
expect(["Error handling is described in chapter 3"], false, "句中提到 Error 不算");
expect([], false, "空");

if (failed) { console.error(`${failed} 个用例失败`); process.exit(1); }
console.log("errorDetect: 全部通过");
