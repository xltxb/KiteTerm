// PBKDF2 口令哈希 + 随机令牌/常量时间比较。
// 存储格式:`pbkdf2$<iterations>$<salt_b64>$<hash_b64>`。
// Workers 运行时(workerd)对 PBKDF2 有硬上限:iterations 超过 100000 直接抛
// NotSupportedError,线上注册曾因 600_000 全部 500。上限值本身已是 OWASP 对
// PBKDF2-HMAC-SHA256 的推荐下限;verifyPassword 按哈希串里记录的次数校验,改动不影响存量。
const ITER = 100_000;
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

// ---- TOTP(RFC 6238,HMAC-SHA1、6 位、30 s 步长),与 `server/src/totp.rs` 逐位一致 ----

const B32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/** RFC 4648 base32 编码,无填充 */
export function base32Encode(u: Uint8Array): string {
  let bits = 0;
  let val = 0;
  let out = "";
  for (const b of u) {
    val = (val << 8) | b;
    bits += 8;
    while (bits >= 5) {
      out += B32[(val >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) out += B32[(val << (5 - bits)) & 31];
  return out;
}

/** RFC 4648 base32 解码,忽略大小写与末尾的 `=` 填充 */
export function base32Decode(s: string): Uint8Array {
  let bits = 0;
  let val = 0;
  const out: number[] = [];
  for (const ch of s.toUpperCase().replace(/=+$/, "")) {
    const i = B32.indexOf(ch);
    if (i < 0) throw new Error("bad base32");
    val = (val << 5) | i;
    bits += 5;
    if (bits >= 8) {
      out.push((val >>> (bits - 8)) & 255);
      bits -= 8;
    }
  }
  return new Uint8Array(out);
}

/** 生成 20 字节随机密钥,base32 编码后为 32 个字符 */
export const newTotpSecret = () => base32Encode(crypto.getRandomValues(new Uint8Array(20)));

async function hotp(secret: Uint8Array, counter: number): Promise<number> {
  const key = await crypto.subtle.importKey("raw", secret, { name: "HMAC", hash: "SHA-1" }, false, ["sign"]);
  const msg = new Uint8Array(8);
  new DataView(msg.buffer).setBigUint64(0, BigInt(counter));
  const h = new Uint8Array(await crypto.subtle.sign("HMAC", key, msg));
  const off = h[19] & 0x0f;
  return (((h[off] & 0x7f) << 24) | (h[off + 1] << 16) | (h[off + 2] << 8) | h[off + 3]) % 1_000_000;
}

/** 给定 base32 密钥与时间戳(秒),计算当前 30 s 步长内的 6 位动态验证码 */
export async function totpCodeAt(secretB32: string, ts: number): Promise<string> {
  return String(await hotp(base32Decode(secretB32), Math.floor(ts / 30))).padStart(6, "0");
}

/** 允许前后各一个时间步(±30 s)的时钟漂移;非 6 位数字直接判否;非法密钥(空/非 base32)一律判否而非抛错 */
export async function totpVerify(secretB32: string, code: string, now: number): Promise<boolean> {
  code = code.trim();
  if (!/^\d{6}$/.test(code)) return false;
  try {
    for (const d of [-1, 0, 1]) if ((await totpCodeAt(secretB32, now + d * 30)) === code) return true;
    return false;
  } catch {
    return false;
  }
}

export const otpauthUrl = (secret: string, email: string) =>
  `otpauth://totp/KiteTerm:${email}?secret=${secret}&issuer=KiteTerm&algorithm=SHA1&digits=6&period=30`;
