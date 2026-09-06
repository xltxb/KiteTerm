// PBKDF2 口令哈希 + 随机令牌/常量时间比较。
// 存储格式:`pbkdf2$<iterations>$<salt_b64>$<hash_b64>`。
const ITER = 600_000;
const enc = new TextEncoder();

export const b64 = (u: Uint8Array) => btoa(String.fromCharCode(...u));
export const unb64 = (s: string) => Uint8Array.from(atob(s), (ch) => ch.charCodeAt(0));

/** 生成 n 字节的随机十六进制字符串(令牌用) */
export const randomHex = (n: number) =>
  [...crypto.getRandomValues(new Uint8Array(n))].map((b) => b.toString(16).padStart(2, "0")).join("");

/** 常量时间比较,避免通过响应耗时侧信道泄露口令/令牌信息 */
export function timingSafeEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  let d = 0;
  for (let i = 0; i < a.length; i++) d |= a[i] ^ b[i];
  return d === 0;
}

async function derive(pw: string, salt: Uint8Array, iter: number): Promise<Uint8Array> {
  const key = await crypto.subtle.importKey("raw", enc.encode(pw), "PBKDF2", false, ["deriveBits"]);
  return new Uint8Array(await crypto.subtle.deriveBits({ name: "PBKDF2", hash: "SHA-256", salt, iterations: iter }, key, 256));
}

export async function hashPassword(pw: string): Promise<string> {
  const salt = crypto.getRandomValues(new Uint8Array(16));
  return `pbkdf2$${ITER}$${b64(salt)}$${b64(await derive(pw, salt, ITER))}`;
}

export async function verifyPassword(pw: string, stored: string): Promise<boolean> {
  const [alg, iterS, saltB, hashB] = stored.split("$");
  if (alg !== "pbkdf2") return false;
  const got = await derive(pw, unb64(saltB), Number(iterS));
  return timingSafeEqual(got, unb64(hashB));
}
