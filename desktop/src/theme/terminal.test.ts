/** 终端配色自检:两档主题都要能读、且 ANSI 色在各自底色上有足够对比度。
 *
 *  背景:终端配色曾经写死深色一档。补浅色时最容易犯的错是把深色档那些亮色
 *  (#7be8b3 之类)原样留下 —— 放到白底上几乎看不见,而这类问题跑起来才发现。
 *  这里直接解析 terminal.css,离线算对比度。 */
import { readFileSync } from "node:fs";

const css = readFileSync(new URL("./terminal.css", import.meta.url), "utf8");
const colors = readFileSync(new URL("./colors.css", import.meta.url), "utf8");

/** 把 --slate-300 这类引用解析成字面色值 */
function resolve(value: string, seen = 0): string {
  const m = value.match(/^var\(\s*(--[\w-]+)\s*\)$/);
  if (!m) return value.trim();
  if (seen > 5) throw new Error("变量引用过深: " + value);
  const re = new RegExp(`^\\s*${m[1]}:\\s*([^;]+);`, "m");
  const hit = colors.match(re) ?? css.match(re);
  if (!hit) throw new Error(`未定义的变量 ${m[1]}`);
  return resolve(hit[1], seen + 1);
}

/** 取某一档(:root 或 [data-theme="dark"])里的所有 --term-* */
function block(selector: string): Record<string, string> {
  const i = css.indexOf(selector + " {");
  if (i < 0) throw new Error(`找不到 ${selector} 段`);
  const body = css.slice(i, css.indexOf("\n}", i));
  const out: Record<string, string> = {};
  for (const m of body.matchAll(/(--term-[\w-]+):\s*([^;]+);/g)) {
    out[m[1]] = resolve(m[2]);
  }
  return out;
}

function luminance(hex: string): number {
  const h = hex.replace("#", "");
  const v = [0, 2, 4].map((i) => {
    const c = parseInt(h.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
}

function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)].sort((m, n) => n - m);
  return (x + 0.05) / (y + 0.05);
}

const LIGHT = block(":root");
const DARK = block('[data-theme="dark"]');

// 1) 两档必须覆盖同一组键 —— 少一个 xterm 就会退回它自己的默认色
const lightKeys = Object.keys(LIGHT).sort();
const darkKeys = Object.keys(DARK).sort();
if (lightKeys.join() !== darkKeys.join()) {
  const miss = [
    ...lightKeys.filter((k) => !darkKeys.includes(k)).map((k) => `深色缺 ${k}`),
    ...darkKeys.filter((k) => !lightKeys.includes(k)).map((k) => `浅色缺 ${k}`),
  ];
  console.error("两档主题的终端配色键不一致:\n  " + miss.join("\n  "));
  process.exit(1);
}

// 2) 底色方向要对:浅色档底亮、深色档底暗
if (luminance(LIGHT["--term-bg"]) < 0.5) {
  console.error(`浅色档底色不够亮: ${LIGHT["--term-bg"]}`);
  process.exit(1);
}
if (luminance(DARK["--term-bg"]) > 0.1) {
  console.error(`深色档底色不够暗: ${DARK["--term-bg"]}`);
  process.exit(1);
}

// 3) 每档里所有前景色对本档底色的对比度 —— ANSI 色按 3:1 卡(粗体/大字级别),
//    正文前景 --term-fg 按 4.5:1 卡。
//
//    例外:ANSI 的 black 与 white 是色轮两端,与画布同侧的那一端本来就贴着背景
//    (深色终端里的 black、浅色终端里的 white),程序拿它当「背景色」用而不是当
//    文本色。这是 ANSI 固有语义,不是配色错误,所以豁免;dim 灰(bright-black)
//    是注释/次要文本的常用色,仍然要卡。
const MIN_ANSI = 3;
const MIN_TEXT = 4.5;
const EXEMPT: Record<string, string[]> = {
  浅色: ["--term-white", "--term-bright-white"],
  深色: ["--term-black"],
};
const problems: string[] = [];
for (const [name, set] of [["浅色", LIGHT], ["深色", DARK]] as const) {
  const bg = set["--term-bg"];
  for (const [key, value] of Object.entries(set)) {
    if (key === "--term-bg" || key === "--term-selection") continue;
    if (EXEMPT[name].includes(key)) continue;
    if (!/^#[0-9a-f]{6}$/i.test(value)) continue; // rgba() 之类跳过
    const need = key === "--term-fg" ? MIN_TEXT : MIN_ANSI;
    const c = contrast(value, bg);
    if (c < need) problems.push(`${name} ${key} = ${value} 对底色 ${bg} 仅 ${c.toFixed(2)}:1(需 ≥ ${need})`);
  }
}
if (problems.length > 0) {
  console.error("终端配色对比度不足:\n  " + problems.join("\n  "));
  process.exit(1);
}

console.log(`TERMINAL_THEME_PASS(浅色 ${lightKeys.length} 项 / 深色 ${darkKeys.length} 项)`);
