/** 执行循环的闸门。三道闸任一触发就把控制权交回用户。
 *
 *  连续失败 2 次:够覆盖「sed 报 Permission denied → 改用 sudo 重试」这一次
 *  自我修正,再失败说明方向错了,继续试只是烧 token。 */
export const MAX_CONSECUTIVE_FAILURES = 2;
export const MAX_STEPS = 12;

export function shouldStop(s: {
  steps: number;
  consecutiveFailures: number;
  stopped: boolean;
}): string | null {
  if (s.stopped) return "已按你的要求停止";
  if (s.consecutiveFailures >= MAX_CONSECUTIVE_FAILURES) {
    return `连续 ${MAX_CONSECUTIVE_FAILURES} 步失败,交回给你判断`;
  }
  if (s.steps >= MAX_STEPS) return `已达 ${MAX_STEPS} 步上限,交回给你判断`;
  return null;
}
