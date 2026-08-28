/** xterm 的 theme 只吃字面色值,不认 CSS 变量。这里在运行时把
 *  theme/terminal.css 里的 --term-* 读出来组装成 xterm theme,
 *  让配色仍然只有 CSS 一处定义。 */

const KEYS = {
  background: "--term-bg",
  foreground: "--term-fg",
  cursor: "--term-cursor",
  selectionBackground: "--term-selection",
  black: "--term-black",
  red: "--term-red",
  green: "--term-green",
  yellow: "--term-yellow",
  blue: "--term-blue",
  magenta: "--term-magenta",
  cyan: "--term-cyan",
  white: "--term-white",
  brightBlack: "--term-bright-black",
  brightRed: "--term-bright-red",
  brightGreen: "--term-bright-green",
  brightYellow: "--term-bright-yellow",
  brightBlue: "--term-bright-blue",
  brightMagenta: "--term-bright-magenta",
  brightCyan: "--term-bright-cyan",
  brightWhite: "--term-bright-white",
} as const;

/** 样式表未就绪时的兜底,与 terminal.css 的画布色保持一致 */
const FALLBACK_BG = "#0a0a0b";

function readVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

/** 终端画布底色 —— 界面里包住 xterm 的容器也用它,避免出现色差边缘 */
export function termBg(): string {
  return readVar("--term-bg") || FALLBACK_BG;
}

/** 组装 xterm 的完整调色板 */
export function terminalTheme(): Record<string, string> {
  const style = getComputedStyle(document.documentElement);
  const theme: Record<string, string> = {};
  for (const [key, cssVar] of Object.entries(KEYS)) {
    const value = style.getPropertyValue(cssVar).trim();
    if (value) theme[key] = value;
  }
  theme.background ||= FALLBACK_BG;
  return theme;
}
