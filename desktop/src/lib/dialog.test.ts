/** 防回归:禁止再用 window.confirm / alert / prompt。
 *
 *  背景见 lib/dialog.ts —— 本运行时的 WKWebView 没实现 JS 对话框面板,这些调用
 *  不弹窗、直接返回 false/undefined,且**不报任何错**。一旦有人写回去,表现是
 *  「按钮点了没反应」,极难从代码上一眼看出。所以用测试钉死。 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;
const BANNED = /(?<![.\w])(?:window\s*\.\s*)?(confirm|alert|prompt)\s*\(/;

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return walk(full);
    return /\.tsx?$/.test(name) && !name.endsWith(".test.ts") ? [full] : [];
  });
}

const offenders: string[] = [];
for (const file of walk(ROOT)) {
  // dialog.ts 自己的注释里会提到这些名字,豁免
  if (file.endsWith("lib/dialog.ts")) continue;
  readFileSync(file, "utf8").split("\n").forEach((line, i) => {
    const code = line.replace(/\/\/.*$/, "").replace(/\/\*.*?\*\//g, "");
    if (BANNED.test(code)) offenders.push(`${file.replace(ROOT, "")}:${i + 1}  ${line.trim()}`);
  });
}

if (offenders.length > 0) {
  console.error("禁止使用 window.confirm / alert / prompt(在本运行时是哑的),请改用 @/lib/dialog:");
  offenders.forEach((o) => console.error("  " + o));
  process.exit(1);
}
console.log("DIALOG_GUARD_PASS");
