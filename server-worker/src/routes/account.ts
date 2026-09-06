import { Hono } from "hono";
import type { Env } from "../env";
import { featureGating, registrationDefault } from "../env";
import { apiError, bearerToken, clientIp, nowTs } from "../http";
import { settingGet } from "../settings";
import * as features from "../features";
import { hashPassword, verifyPassword } from "../crypto";
import { rateLimitOk } from "../ratelimit";
import { authenticate, issueToken, newTokenValue, passwordIssue, tokenInsertStatement, userPublic, validEmail } from "../auth";

export const accountRoutes = new Hono<{ Bindings: Env }>();

// 与 Rust 版 `Credentials` 一致:该结构体没有 `#[serde(rename_all = "camelCase")]`,
// 字段按 serde 默认规则走蛇形命名,所以这里用 `invite_code` 而非 `inviteCode`。
interface Credentials {
  email?: unknown;
  password?: unknown;
  invite_code?: unknown;
  totp?: unknown;
}

const emptyCredentials: Credentials = {};

// ---- 注册 ----

accountRoutes.post("/api/register", async (c) => {
  const db = c.env.DB;
  // 限流放在最前:注册接口会通过 email_taken 泄露邮箱是否已注册,
  // 必须先压制暴力枚举,再做其余校验
  const ip = clientIp(c);
  const limit = Number(c.env.KITE_REGISTER_LIMIT ?? 5);
  if (!(await rateLimitOk(c.env.RATE, "register", ip, limit))) {
    return apiError(c, 429, "rate_limited", "注册过于频繁,请稍后再试");
  }

  const body = await c.req.json<Credentials>().catch(() => emptyCredentials);
  const email = String(body.email ?? "").trim().toLowerCase();
  if (!validEmail(email)) return apiError(c, 400, "invalid_email", "邮箱格式不正确");
  const password = String(body.password ?? "");
  const issue = passwordIssue(password);
  if (issue) return apiError(c, 400, "weak_password", issue);

  const mode = await settingGet(db, "registration_mode", registrationDefault(c.env));
  if (mode === "closed") return apiError(c, 403, "registration_closed", "平台当前未开放注册");

  // 邀请码:invite 模式必填并校验有效性(未停用 / 未过期 / 未用尽)
  const inviteCodeRaw = typeof body.invite_code === "string" ? body.invite_code.trim() : "";
  const invite = inviteCodeRaw.length > 0 ? inviteCodeRaw : null;
  if (mode === "invite") {
    if (!invite) return apiError(c, 400, "invite_required", "该平台需要邀请码才能注册");
    const row = await db
      .prepare("SELECT max_uses, used_count, expires_at, disabled FROM invites WHERE code = ?1")
      .bind(invite)
      .first<{ max_uses: number; used_count: number; expires_at: number; disabled: number }>();
    if (!row) return apiError(c, 400, "invite_invalid", "邀请码无效");
    if (row.disabled !== 0) return apiError(c, 400, "invite_invalid", "邀请码已停用");
    if (row.expires_at !== 0 && row.expires_at < nowTs()) return apiError(c, 400, "invite_expired", "邀请码已过期");
    if (row.max_uses !== 0 && row.used_count >= row.max_uses) return apiError(c, 400, "invite_exhausted", "邀请码使用次数已用尽");
  }

  const hash = await hashPassword(password);
  const id = crypto.randomUUID();
  const defaultSync = await settingGet(db, "default_sync_enabled", "1");
  const syncEnabled = defaultSync === "1";
  // 邮箱验证与 TOTP 的实际判定在 Task 3 补上,本任务先固定为已验证 / 未绑定
  const inserted = await db
    .prepare(
      "INSERT OR IGNORE INTO users (id, email, password_hash, sync_enabled, created_at, email_verified) VALUES (?1, ?2, ?3, ?4, ?5, 1)",
    )
    .bind(id, email, hash, syncEnabled ? 1 : 0, nowTs())
    .run();
  if ((inserted.meta.changes ?? 0) === 0) {
    return apiError(c, 409, "email_taken", "该邮箱已注册");
  }

  // 用户已创建之后的副作用(签发令牌、消费邀请码)放在同一批次里原子提交,
  // 避免出现「令牌发了但邀请码没扣」或反之的中间状态
  const { token, expiresAt } = newTokenValue();
  const statements = [tokenInsertStatement(db, id, token, expiresAt)];
  if (mode === "invite" && invite) {
    statements.push(db.prepare("UPDATE invites SET used_count = used_count + 1 WHERE code = ?1").bind(invite));
    statements.push(db.prepare("INSERT INTO invite_uses (code, user_id, used_at) VALUES (?1, ?2, ?3)").bind(invite, id, nowTs()));
  }
  await db.batch(statements);

  const totpSetupRequired = await features.raw(db, "totp_required");
  return c.json(
    {
      userId: id,
      token,
      expiresAt,
      totpSetupRequired,
      user: { id, email, syncEnabled, emailVerified: true, totpEnabled: false },
    },
    201,
  );
});

// ---- 登录 ----

accountRoutes.post("/api/login", async (c) => {
  const db = c.env.DB;
  const body = await c.req.json<Credentials>().catch(() => emptyCredentials);
  const email = String(body.email ?? "").trim().toLowerCase();
  const row = await db
    .prepare("SELECT id, password_hash, sync_enabled, email_verified, totp_secret, totp_enabled FROM users WHERE email = ?1")
    .bind(email)
    .first<{
      id: string;
      password_hash: string;
      sync_enabled: number;
      email_verified: number;
      totp_secret: string | null;
      totp_enabled: number;
    }>();
  if (!row) return apiError(c, 401, "bad_credentials", "邮箱或密码错误");
  const password = String(body.password ?? "");
  if (!(await verifyPassword(password, row.password_hash))) {
    return apiError(c, 401, "bad_credentials", "邮箱或密码错误");
  }
  // 开关校验放在口令校验之后:先把凭据不对的挡掉,避免通过错误信息区分
  // 「密码错」和「登录已关」而泄露邮箱是否存在
  if (!(await features.effective(db, featureGating(c.env), "login"))) {
    return apiError(c, 403, "LOGIN_DISABLED", "平台当前已关闭登录");
  }

  // ---- Task 3: TOTP 分支插入点 ----
  // row.totp_enabled !== 0 时:先按「用户+IP」限流(每小时 10 次,TOTP_RATE_LIMITED),
  // 再校验 body.totp 是否存在(缺失 → TOTP_REQUIRED)、是否正确(错误 → TOTP_INVALID)。

  const { token, expiresAt } = await issueToken(db, row.id);
  // 多设备登录关闭:新登录挤掉其它设备的令牌
  if (!(await features.raw(db, "multi_device"))) {
    await db.prepare("DELETE FROM tokens WHERE user_id = ?1 AND token != ?2").bind(row.id, token).run();
  }
  const totpSetupRequired = (await features.raw(db, "totp_required")) && row.totp_enabled === 0;
  return c.json({
    token,
    expiresAt,
    totpSetupRequired,
    user: userPublic({
      id: row.id,
      email,
      sync_enabled: row.sync_enabled,
      email_verified: row.email_verified,
      totp_enabled: row.totp_enabled,
    }),
  });
});

// ---- 登出 ----

accountRoutes.post("/api/logout", async (c) => {
  const token = bearerToken(c);
  if (token) {
    await c.env.DB.prepare("DELETE FROM tokens WHERE token = ?1").bind(token).run();
  }
  return c.json({ ok: true });
});

// ---- 当前用户 ----

accountRoutes.get("/api/me", async (c) => {
  const user = await authenticate(c);
  if (user instanceof Response) return user;
  const db = c.env.DB;
  return c.json({
    ...user,
    requirements: {
      emailVerify: await features.raw(db, "email_verify"),
      totpRequired: await features.raw(db, "totp_required"),
    },
  });
});
