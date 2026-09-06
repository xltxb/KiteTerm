import { Hono } from "hono";
import type { Env } from "../env";
import { featureGating, registrationDefault } from "../env";
import { apiError, bearerToken, clientIp, nowTs } from "../http";
import { settingGet } from "../settings";
import * as features from "../features";
import { hashPassword, newTotpSecret, otpauthUrl, totpVerify, verifyPassword } from "../crypto";
import { rateLimitOk } from "../ratelimit";
import { authenticate, issueToken, newTokenValue, passwordIssue, tokenInsertStatement, userPublic, validEmail } from "../auth";
import { newCode, sendCode } from "../mail";

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
  // 邮箱验证开关开着时,新用户先落地为未验证,注册成功后即发验证码
  const needVerify = await features.raw(db, "email_verify");
  const inserted = await db
    .prepare(
      "INSERT OR IGNORE INTO users (id, email, password_hash, sync_enabled, created_at, email_verified) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(id, email, hash, syncEnabled ? 1 : 0, nowTs(), needVerify ? 0 : 1)
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

  // ---- 邮箱验证 ----
  if (needVerify) {
    const code = newCode();
    await db
      .prepare(
        "INSERT INTO email_codes (user_id, code, expires_at, sent_at, attempts) VALUES (?1, ?2, ?3, ?4, 0) " +
          "ON CONFLICT(user_id) DO UPDATE SET code=?2, expires_at=?3, sent_at=?4, attempts=0",
      )
      .bind(id, code, nowTs() + 600, nowTs())
      .run();
    // 发信不阻塞响应,交给 waitUntil 在响应返回后继续执行
    c.executionCtx.waitUntil(sendCode(c.env, email, code));
  }

  const totpSetupRequired = await features.raw(db, "totp_required");
  return c.json(
    {
      userId: id,
      token,
      expiresAt,
      totpSetupRequired,
      user: { id, email, syncEnabled, emailVerified: !needVerify, totpEnabled: false },
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

  if (row.totp_enabled !== 0) {
    // 猜测节流:同一账户 + 同一 IP 每小时最多尝试 10 次,防暴力破解 6 位动态验证码
    const ip = clientIp(c);
    if (!(await rateLimitOk(c.env.RATE, "totp", `${row.id}:${ip}`, 10))) {
      return apiError(c, 429, "TOTP_RATE_LIMITED", "动态验证码尝试过于频繁,请稍后再试");
    }
    const totp = typeof body.totp === "string" ? body.totp.trim() : "";
    if (!totp) return apiError(c, 401, "TOTP_REQUIRED", "请输入动态验证码");
    if (!(await totpVerify(row.totp_secret ?? "", totp, nowTs()))) {
      return apiError(c, 401, "TOTP_INVALID", "动态验证码错误");
    }
  }

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

// ---- 邮箱验证 ----

interface CodeBody {
  code?: unknown;
}

accountRoutes.post("/api/verify-email", async (c) => {
  const user = await authenticate(c);
  if (user instanceof Response) return user;
  // 猜测节流:同一账户每小时最多尝试 10 次,与下面按次数删码互为补充
  if (!(await rateLimitOk(c.env.RATE, "verify_email", user.id, 10))) {
    return apiError(c, 429, "CODE_RATE_LIMITED", "验证码尝试过于频繁,请稍后再试");
  }
  const db = c.env.DB;
  const body = await c.req.json<CodeBody>().catch(() => ({}) as CodeBody);
  const submitted = String(body.code ?? "").trim();
  const row = await db
    .prepare("SELECT code, expires_at, attempts FROM email_codes WHERE user_id = ?1")
    .bind(user.id)
    .first<{ code: string; expires_at: number; attempts: number }>();
  if (!row) return apiError(c, 400, "CODE_INVALID", "验证码不存在,请重新发送");
  if (row.expires_at < nowTs()) return apiError(c, 400, "CODE_EXPIRED", "验证码已过期,请重新发送");
  if (row.code !== submitted) {
    const attempts = row.attempts + 1;
    if (attempts >= 5) {
      // 错误次数用尽:直接把码删掉,逼用户重新发送,而不是无限期让人猜
      await db.prepare("DELETE FROM email_codes WHERE user_id = ?1").bind(user.id).run();
      return apiError(c, 400, "CODE_INVALID", "验证码错误次数过多,请重新发送");
    }
    await db.prepare("UPDATE email_codes SET attempts = ?1 WHERE user_id = ?2").bind(attempts, user.id).run();
    return apiError(c, 400, "CODE_INVALID", "验证码错误");
  }
  await db.prepare("UPDATE users SET email_verified = 1 WHERE id = ?1").bind(user.id).run();
  await db.prepare("DELETE FROM email_codes WHERE user_id = ?1").bind(user.id).run();
  return c.json({ ok: true, emailVerified: true });
});

accountRoutes.post("/api/verify-email/resend", async (c) => {
  const user = await authenticate(c);
  if (user instanceof Response) return user;
  if (user.emailVerified) return c.json({ ok: true, emailVerified: true });
  const db = c.env.DB;
  const code = newCode();
  const last = await db.prepare("SELECT sent_at FROM email_codes WHERE user_id = ?1").bind(user.id).first<{ sent_at: number }>();
  if (last && nowTs() - last.sent_at < 60) {
    return apiError(c, 429, "RESEND_TOO_SOON", "发送过于频繁,请一分钟后再试");
  }
  await db
    .prepare(
      "INSERT INTO email_codes (user_id, code, expires_at, sent_at, attempts) VALUES (?1, ?2, ?3, ?4, 0) " +
        "ON CONFLICT(user_id) DO UPDATE SET code=?2, expires_at=?3, sent_at=?4, attempts=0",
    )
    .bind(user.id, code, nowTs() + 600, nowTs())
    .run();
  try {
    await sendCode(c.env, user.email, code);
  } catch (e) {
    return apiError(c, 502, "MAIL_ERROR", e instanceof Error ? e.message : String(e));
  }
  return c.json({ ok: true, emailVerified: false });
});

// ---- 双因素认证 ----

accountRoutes.post("/api/2fa/setup", async (c) => {
  const user = await authenticate(c);
  if (user instanceof Response) return user;
  if (user.totpEnabled) {
    return apiError(c, 409, "TOTP_ALREADY_ENABLED", "已绑定双因素认证,如需更换请先解绑");
  }
  const secret = newTotpSecret();
  await c.env.DB.prepare("UPDATE users SET totp_secret = ?2 WHERE id = ?1").bind(user.id, secret).run();
  return c.json({ secret, otpauthUrl: otpauthUrl(secret, user.email) });
});

accountRoutes.post("/api/2fa/enable", async (c) => {
  const user = await authenticate(c);
  if (user instanceof Response) return user;
  const db = c.env.DB;
  const body = await c.req.json<CodeBody>().catch(() => ({}) as CodeBody);
  const code = String(body.code ?? "");
  const row = await db.prepare("SELECT totp_secret FROM users WHERE id = ?1").bind(user.id).first<{ totp_secret: string | null }>();
  const secret = row?.totp_secret ?? null;
  if (!secret) return apiError(c, 400, "TOTP_NOT_PENDING", "请先生成密钥");
  if (!(await totpVerify(secret, code, nowTs()))) {
    return apiError(c, 400, "TOTP_INVALID", "动态验证码错误");
  }
  await db.prepare("UPDATE users SET totp_enabled = 1 WHERE id = ?1").bind(user.id).run();
  return c.json({ ok: true, totpEnabled: true });
});

accountRoutes.post("/api/2fa/disable", async (c) => {
  const user = await authenticate(c);
  if (user instanceof Response) return user;
  const db = c.env.DB;
  if (await features.raw(db, "totp_required")) {
    return apiError(c, 403, "TOTP_ENFORCED", "平台强制双因素认证,不能解绑");
  }
  const body = await c.req.json<CodeBody>().catch(() => ({}) as CodeBody);
  const code = String(body.code ?? "");
  const row = await db.prepare("SELECT totp_secret FROM users WHERE id = ?1").bind(user.id).first<{ totp_secret: string | null }>();
  const secret = row?.totp_secret ?? "";
  if (!user.totpEnabled || !(await totpVerify(secret, code, nowTs()))) {
    return apiError(c, 400, "TOTP_INVALID", "动态验证码错误");
  }
  await db.prepare("UPDATE users SET totp_enabled = 0, totp_secret = NULL WHERE id = ?1").bind(user.id).run();
  return c.json({ ok: true, totpEnabled: false });
});
