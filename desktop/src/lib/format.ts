/** 界面里反复出现的数值格式化。各屏原本各写一份,统一到这里。 */

/** 字节数 → 人类可读。zero 决定 0 的写法:传输列表用「—」,流量统计用「0 B」。 */
export function fmtBytes(n: number, zero = "0 B"): string {
  if (n <= 0) return zero;
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(1)} GB`;
}

/** 秒 → mm:ss(录制计时、回放进度) */
export function fmtClock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

/** 毫秒 → mm:ss */
export function fmtDuration(ms: number): string {
  return fmtClock(ms / 1000);
}
