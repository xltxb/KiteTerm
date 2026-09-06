/** KV 滑动窗口:值是窗口内时间戳数组;最终一致,允许少量超发 */
export async function rateLimitOk(kv: KVNamespace, bucket: string, key: string, max: number, windowSec = 3600): Promise<boolean> {
  const k = `rl:${bucket}:${key}`;
  const now = Math.floor(Date.now() / 1000);
  const hits = ((await kv.get<number[]>(k, "json")) ?? []).filter((t) => now - t < windowSec);
  if (hits.length >= max) return false;
  hits.push(now);
  await kv.put(k, JSON.stringify(hits), { expirationTtl: windowSec });
  return true;
}
