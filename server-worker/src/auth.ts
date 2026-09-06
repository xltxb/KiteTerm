// 账户相关的纯逻辑与鉴权:邮箱/口令校验、邀请码生成、令牌签发与校验。
// 与 `server/src/main.rs` 的同名函数逐一对应,保持规则、错误码与文案一致。
import type { C } from "./http";
import { apiError, bearerToken, nowTs } from "./http";
import { randomHex } from "./crypto";

const PASSWORD_MIN_LEN = 8;
const PASSWORD_MAX_LEN = 128;
const TOKEN_TTL_SECS = 30 * 24 * 3600;

/** 邮箱基本结构校验:单个 @、本地段与域名段非空、域名含点且各段合法、无空白 */
export function validEmail(email: string): boolean {
  if (email.length > 254 || /\s/.test(email)) return false;
  const parts = email.split("@");
  if (parts.length !== 2) return false;
  const [local, domain] = parts;
  if (!local || local.length > 64 || !domain || domain.length > 255) return false;
  if (!domain.includes(".")) return false;
  return domain
    .split(".")
    .every((seg) => seg.length > 0 && !seg.startsWith("-") && !seg.endsWith("-") && /^[a-zA-Z0-9-]+$/.test(seg));
}

/** 常见弱口令表,照抄 Rust 版 WEAK 常量 */
const WEAK = new Set([
  "password", "password1", "password123", "12345678", "123456789", "1234567890",
  "qwerty123", "admin123", "abc12345", "iloveyou", "letmein1", "welcome1",
]);

/** 口令强度:长度 + 至少两类字符 + 排除常见弱口令。返回 null 表示合格,否则返回不合格原因。 */
export function passwordIssue(pw: string): string | null {
  const chars = [...pw];
  const len = chars.length;
  if (len < PASSWORD_MIN_LEN) return `密码至少 ${PASSWORD_MIN_LEN} 位`;
  if (len > PASSWORD_MAX_LEN) return `密码最多 ${PASSWORD_MAX_LEN} 位`;
  const hasLetter = chars.some((c) => /\p{L}/u.test(c));
  const hasDigit = chars.some((c) => /\p{N}/u.test(c));
  const hasSymbol = chars.some((c) => !/[\p{L}\p{N}]/u.test(c) && !/\s/.test(c));
  const kinds = [hasLetter, hasDigit, hasSymbol].filter(Boolean).length;
  if (kinds < 2) return "密码需包含字母、数字、符号中的至少两类";
  if (new Set(chars).size < 4) return "密码字符过于单一";
  if (WEAK.has(pw.toLowerCase())) return "密码过于常见,请更换";
  return null;
}

/** 去掉易混字符(0/O/1/I/L),便于口头/手抄传达 */
const INVITE_ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";
export function newInviteCode(): string {
  const raw = new Uint8Array(12);
  crypto.getRandomValues(raw);
  const body = Array.from(raw, (b) => INVITE_ALPHABET[b % INVITE_ALPHABET.length]).join("");
  return `${body.slice(0, 4)}-${body.slice(4, 8)}-${body.slice(8, 12)}`;
}

/** 生成一枚新令牌的值(不落库),供需要与其它写入同批次提交的场景使用 */
export function newTokenValue(): { token: string; expiresAt: number } {
  return { token: randomHex(32), expiresAt: nowTs() + TOKEN_TTL_SECS };
}

export function tokenInsertStatement(db: D1Database, userId: string, token: string, expiresAt: number): D1PreparedStatement {
  return db.prepare("INSERT INTO tokens (token, user_id, expires_at) VALUES (?1, ?2, ?3)").bind(token, userId, expiresAt);
}

/** 签发登录令牌(注册与登录共用)。注册场景若需要与邀请码计数原子提交,改用 newTokenValue + tokenInsertStatement + db.batch。 */
export async function issueToken(db: D1Database, userId: string): Promise<{ token: string; expiresAt: number }> {
  const { token, expiresAt } = newTokenValue();
  await tokenInsertStatement(db, userId, token, expiresAt).run();
  return { token, expiresAt };
}

// ---- 鉴权 ----

export interface AuthedUser {
  id: string;
  email: string;
  syncEnabled: boolean;
  emailVerified: boolean;
  totpEnabled: boolean;
}

interface UserRow {
  id: string;
  email: string;
  sync_enabled: number;
  email_verified: number;
  totp_enabled: number;
}

export function userPublic(row: UserRow): AuthedUser {
  return {
    id: row.id,
    email: row.email,
    syncEnabled: row.sync_enabled !== 0,
    emailVerified: row.email_verified !== 0,
    totpEnabled: row.totp_enabled !== 0,
  };
}

interface AuthRow extends UserRow {
  expires_at: number;
}

/** 从 Authorization: Bearer 头解析并校验令牌;缺失/无效/过期分别对应三种 401 */
export async function authenticate(c: C): Promise<AuthedUser | Response> {
  const token = bearerToken(c);
  if (!token) return apiError(c, 401, "unauthorized", "缺少令牌");
  const row = await c.env.DB.prepare(
    `SELECT u.id, u.email, u.sync_enabled, u.email_verified, u.totp_enabled, t.expires_at
     FROM tokens t JOIN users u ON u.id = t.user_id WHERE t.token = ?1`,
  ).bind(token).first<AuthRow>();
  if (!row) return apiError(c, 401, "unauthorized", "令牌无效");
  if (row.expires_at <= nowTs()) return apiError(c, 401, "token_expired", "令牌已过期,请重新登录");
  return userPublic(row);
}
