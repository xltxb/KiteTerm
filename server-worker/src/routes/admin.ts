import { Hono } from "hono";
import type { Env } from "../env";
import { featureGating, registrationDefault } from "../env";
import { apiError, nowTs, requireAdmin } from "../http";
import type { C } from "../http";
import { settingGet, settingSet } from "../settings";
import * as features from "../features";
import { newInviteCode } from "../auth";
import type { StorageConfig } from "../storage";
import { isStorageConfigured, loadStorage, normalizeStorage, prefixValid, probe, publicStorageJson, saveStorage, usage } from "../storage";

export const adminRoutes = new Hono<{ Bindings: Env }>();

/** 管理接口与公开接口共用同一份取值,避免两边漂移 */
async function settingsPayload(db: D1Database, env: Env) {
  const admin = await features.adminPayload(db);
  const registrationMode = await settingGet(db, "registration_mode", registrationDefault(env));
  const defaultSyncEnabled = (await settingGet(db, "default_sync_enabled", "1")) === "1";
  return { ...admin, registrationMode, defaultSyncEnabled, featureGating: featureGating(env) };
}

adminRoutes.get("/api/admin/settings", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  return c.json(await settingsPayload(c.env.DB, c.env));
});

adminRoutes.post("/api/admin/settings", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const body = await c.req.json<Record<string, unknown>>().catch(() => ({}) as Record<string, unknown>);
  const db = c.env.DB;

  if (typeof body.registrationMode === "string") {
    const mode = body.registrationMode;
    if (!["open", "invite", "closed"].includes(mode)) {
      return apiError(c, 400, "invalid_mode", "注册策略只能是 open/invite/closed");
    }
    await settingSet(db, "registration_mode", mode);
  }
  if (typeof body.defaultSyncEnabled === "boolean") {
    await settingSet(db, "default_sync_enabled", body.defaultSyncEnabled ? "1" : "0");
  }
  for (const [key, value] of Object.entries(body)) {
    if (key === "registrationMode" || key === "defaultSyncEnabled") continue;
    if (!key.startsWith("feature")) continue;
    const rest = key.slice("feature".length);
    if (typeof value !== "boolean") continue;
    const snakeName = features.snake(rest);
    if (!features.NAMES.some(([n]) => n === snakeName)) {
      return apiError(c, 400, "UNKNOWN_FEATURE", `未知功能开关 ${key}`);
    }
    await features.set(db, snakeName, value);
  }
  return c.json(await settingsPayload(db, c.env));
});

adminRoutes.get("/api/admin/invites", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const { results } = await c.env.DB.prepare(
    "SELECT code, note, max_uses, used_count, expires_at, disabled, created_at FROM invites ORDER BY created_at DESC",
  ).all<{ code: string; note: string; max_uses: number; used_count: number; expires_at: number; disabled: number; created_at: number }>();
  const invites = results.map((r) => ({
    code: r.code,
    note: r.note,
    maxUses: r.max_uses,
    usedCount: r.used_count,
    expiresAt: r.expires_at,
    disabled: r.disabled !== 0,
    createdAt: r.created_at,
  }));
  return c.json({ invites });
});

adminRoutes.post("/api/admin/invites", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const body = await c.req.json<{ note?: string; maxUses?: number; expiresInDays?: number }>().catch(() => ({}) as Record<string, unknown>);
  const note = typeof body.note === "string" ? body.note : "";
  const maxUses = Math.max(0, Number(body.maxUses ?? 0) | 0);
  const expiresInDays = Number(body.expiresInDays ?? 0) | 0;
  const code = newInviteCode();
  const expiresAt = expiresInDays > 0 ? nowTs() + expiresInDays * 86400 : 0;
  await c.env.DB.prepare(
    "INSERT INTO invites (code, note, max_uses, used_count, expires_at, disabled, created_at) VALUES (?1, ?2, ?3, 0, ?4, 0, ?5)",
  ).bind(code, note, maxUses, expiresAt, nowTs()).run();
  return c.json({ code, expiresAt, maxUses });
});

adminRoutes.post("/api/admin/invites/:code/disable", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const code = c.req.param("code").trim();
  const res = await c.env.DB.prepare("UPDATE invites SET disabled = 1 WHERE code = ?1").bind(code).run();
  if ((res.meta.changes ?? 0) === 0) {
    return apiError(c, 404, "not_found", "邀请码不存在");
  }
  return c.json({ ok: true });
});

adminRoutes.get("/api/admin/users", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const { results } = await c.env.DB.prepare(
    "SELECT id, email, sync_enabled, created_at, email_verified, totp_enabled FROM users ORDER BY created_at",
  ).all<{ id: string; email: string; sync_enabled: number; created_at: number; email_verified: number; totp_enabled: number }>();
  const users = results.map((r) => ({
    id: r.id,
    email: r.email,
    syncEnabled: r.sync_enabled !== 0,
    emailVerified: r.email_verified !== 0,
    totpEnabled: r.totp_enabled !== 0,
    createdAt: r.created_at,
  }));
  return c.json({ users });
});

adminRoutes.post("/api/admin/users/:email/sync", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  // 与 Rust 版一致:WHERE 用归一化(trim + 小写)后的邮箱匹配,
  // 但响应体的 "email" 回显原始路径参数,不做归一化
  const rawEmail = c.req.param("email");
  const normalizedEmail = rawEmail.trim().toLowerCase();
  const body = await c.req.json<{ enabled?: boolean }>().catch(() => ({}) as Record<string, unknown>);
  if (typeof body.enabled !== "boolean") {
    return apiError(c, 400, "INVALID_BODY", "enabled 必须是布尔值");
  }
  const enabled = body.enabled;
  const res = await c.env.DB.prepare("UPDATE users SET sync_enabled = ?2 WHERE email = ?1").bind(normalizedEmail, enabled ? 1 : 0).run();
  if ((res.meta.changes ?? 0) === 0) {
    return apiError(c, 404, "not_found", "用户不存在");
  }
  return c.json({ ok: true, email: rawEmail, syncEnabled: enabled });
});

/** 管理员手工标记已验证(没有邮件服务的部署) */
adminRoutes.post("/api/admin/users/:email/verify", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const db = c.env.DB;
  const email = c.req.param("email").trim().toLowerCase();
  const res = await db.prepare("UPDATE users SET email_verified = 1 WHERE email = ?1").bind(email).run();
  if ((res.meta.changes ?? 0) === 0) return apiError(c, 404, "not_found", "用户不存在");
  // 顺带清掉待验证的验证码 —— 已被管理员手工标记,残留的码/尝试次数没有意义
  await db
    .prepare("DELETE FROM email_codes WHERE user_id IN (SELECT id FROM users WHERE email = ?1)")
    .bind(email)
    .run();
  return c.json({ ok: true });
});

/** 管理员重置(用户丢了验证器):清掉密钥并吊销令牌,下次登录重新绑定 */
adminRoutes.post("/api/admin/users/:email/2fa/reset", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const db = c.env.DB;
  const email = c.req.param("email").trim().toLowerCase();
  const res = await db.prepare("UPDATE users SET totp_enabled = 0, totp_secret = NULL WHERE email = ?1").bind(email).run();
  if ((res.meta.changes ?? 0) === 0) return apiError(c, 404, "not_found", "用户不存在");
  await db.prepare("DELETE FROM tokens WHERE user_id IN (SELECT id FROM users WHERE email = ?1)").bind(email).run();
  return c.json({ ok: true });
});

/** 强制下线:吊销该用户全部令牌 */
adminRoutes.post("/api/admin/users/:email/revoke", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const email = c.req.param("email").trim().toLowerCase();
  const res = await c.env.DB
    .prepare("DELETE FROM tokens WHERE user_id IN (SELECT id FROM users WHERE email = ?1)")
    .bind(email)
    .run();
  return c.json({ ok: true, revoked: res.meta.changes ?? 0 });
});

adminRoutes.get("/api/admin/stats", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const db = c.env.DB;
  const total = (await db.prepare("SELECT COUNT(*) AS n FROM users").first<{ n: number }>())?.n ?? 0;
  const enabled = (await db.prepare("SELECT COUNT(*) AS n FROM users WHERE sync_enabled = 1").first<{ n: number }>())?.n ?? 0;
  const tokens = (await db.prepare("SELECT COUNT(*) AS n FROM tokens WHERE expires_at > ?1").bind(nowTs()).first<{ n: number }>())?.n ?? 0;
  const { cfg } = await loadStorage(db, c.env);
  return c.json({
    users: total,
    syncEnabled: enabled,
    activeTokens: tokens,
    bucket: cfg.bucket,
    endpoint: cfg.endpoint,
    // 控制台据此显示凭据模式徽章(STS 限权 / 共享回退)
    stsMode: cfg.stsMode,
    storageConfigured: isStorageConfigured(cfg),
  });
});

/** 该用户前缀下的对象数与字节数 */
adminRoutes.get("/api/admin/users/:email/usage", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const email = c.req.param("email").trim().toLowerCase();
  const row = await c.env.DB.prepare("SELECT id FROM users WHERE email = ?1").bind(email).first<{ id: string }>();
  if (!row) return apiError(c, 404, "not_found", "用户不存在");
  const { cfg } = await loadStorage(c.env.DB, c.env);
  try {
    const result = await usage(cfg, row.id);
    return c.json(result);
  } catch (e) {
    return apiError(c, 502, "S3_ERROR", e instanceof Error ? e.message : String(e));
  }
});

// ---- S3 存储配置(管理接口)----

adminRoutes.get("/api/admin/storage", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const { cfg, source } = await loadStorage(c.env.DB, c.env);
  return c.json(publicStorageJson(cfg, source));
});

/** body 里字符串/布尔字段的宽松取值:类型不对就当缺省处理,交给下面的校验兜底 */
function bodyString(body: Record<string, unknown>, key: string): string {
  return typeof body[key] === "string" ? (body[key] as string) : "";
}

/** 与 Rust `storage_from_body` 一致:endpoint 须 http(s) 开头;bucket/accessKey/secret 非空;prefix 合法 */
function storageFromBody(c: C, current: StorageConfig, body: Record<string, unknown>): StorageConfig | Response {
  const secretRaw = bodyString(body, "secretKey");
  // 空 secret 沿用已保存的值
  const secretKey = secretRaw === "" ? current.secretKey : secretRaw;
  const regionRaw = bodyString(body, "region");
  const cfg = normalizeStorage({
    endpoint: bodyString(body, "endpoint"),
    region: regionRaw === "" ? "us-east-1" : regionRaw,
    bucket: bodyString(body, "bucket"),
    prefix: bodyString(body, "prefix"),
    accessKey: bodyString(body, "accessKey"),
    secretKey,
    pathStyle: typeof body.pathStyle === "boolean" ? body.pathStyle : true,
    stsMode: typeof body.stsMode === "string" ? body.stsMode : "auto",
  });
  if (!cfg.endpoint.startsWith("http://") && !cfg.endpoint.startsWith("https://")) {
    return apiError(c, 400, "INVALID_ENDPOINT", "Endpoint 须以 http:// 或 https:// 开头");
  }
  if (cfg.bucket === "" || cfg.accessKey === "" || cfg.secretKey === "") {
    return apiError(c, 400, "INVALID_STORAGE", "Bucket、Access Key、Secret 不能为空");
  }
  if (!prefixValid(cfg.prefix)) {
    return apiError(c, 400, "INVALID_STORAGE", "路径前缀只能包含字母、数字、. _ - /,且不能含 ..");
  }
  return cfg;
}

/** 保存并热生效:之后的凭据下发、用量统计、STS 都用新配置 */
adminRoutes.post("/api/admin/storage", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const body = await c.req.json<Record<string, unknown>>().catch(() => ({}) as Record<string, unknown>);
  const { cfg: current } = await loadStorage(c.env.DB, c.env);
  const result = storageFromBody(c, current, body);
  if (result instanceof Response) return result;
  await saveStorage(c.env.DB, result);
  return c.json(publicStorageJson(result, "db"));
});

/** 用表单里的值(未保存)探测;secret 留空则用已保存的 */
adminRoutes.post("/api/admin/storage/test", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const body = await c.req.json<Record<string, unknown>>().catch(() => ({}) as Record<string, unknown>);
  const { cfg: current } = await loadStorage(c.env.DB, c.env);
  const result = storageFromBody(c, current, body);
  if (result instanceof Response) return result;
  const report = await probe(result, c.env);
  return c.json(report);
});
