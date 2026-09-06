import { Hono } from "hono";
import type { Env } from "../env";
import { featureGating, registrationDefault } from "../env";
import { apiError, nowTs, requireAdmin } from "../http";
import { settingGet, settingSet } from "../settings";
import * as features from "../features";

export const adminRoutes = new Hono<{ Bindings: Env }>();

/** 管理接口与公开接口共用同一份取值,避免两边漂移 */
async function settingsPayload(db: D1Database, env: Env) {
  const admin = await features.adminPayload(db);
  const registrationMode = await settingGet(db, "registration_mode", registrationDefault(env));
  const defaultSyncEnabled = (await settingGet(db, "default_sync_enabled", "1")) === "1";
  return { ...admin, registrationMode, defaultSyncEnabled, featureGating: featureGating(env) };
}

/** 去掉易混字符(0/O/1/I/L),便于口头/手抄传达 */
const INVITE_ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";
export function newInviteCode(): string {
  const raw = new Uint8Array(12);
  crypto.getRandomValues(raw);
  const body = Array.from(raw, (b) => INVITE_ALPHABET[b % INVITE_ALPHABET.length]).join("");
  return `${body.slice(0, 4)}-${body.slice(4, 8)}-${body.slice(8, 12)}`;
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
  const email = c.req.param("email").trim().toLowerCase();
  const body = await c.req.json<{ enabled?: boolean }>().catch(() => ({}) as Record<string, unknown>);
  const enabled = body.enabled === true;
  const res = await c.env.DB.prepare("UPDATE users SET sync_enabled = ?2 WHERE email = ?1").bind(email, enabled ? 1 : 0).run();
  if ((res.meta.changes ?? 0) === 0) {
    return apiError(c, 404, "not_found", "用户不存在");
  }
  return c.json({ ok: true, email, syncEnabled: enabled });
});

adminRoutes.get("/api/admin/stats", async (c) => {
  const deny = requireAdmin(c);
  if (deny) return deny;
  const db = c.env.DB;
  const total = (await db.prepare("SELECT COUNT(*) AS n FROM users").first<{ n: number }>())?.n ?? 0;
  const enabled = (await db.prepare("SELECT COUNT(*) AS n FROM users WHERE sync_enabled = 1").first<{ n: number }>())?.n ?? 0;
  const tokens = (await db.prepare("SELECT COUNT(*) AS n FROM tokens WHERE expires_at > ?1").bind(nowTs()).first<{ n: number }>())?.n ?? 0;
  const env = c.env;
  const endpoint = env.S3_ENDPOINT ?? "";
  const bucket = env.S3_BUCKET ?? "";
  const stsMode = env.KITE_STS ?? "auto";
  const storageConfigured = Boolean(endpoint && bucket && (env.S3_ACCESS_KEY ?? "") !== "");
  return c.json({
    users: total,
    syncEnabled: enabled,
    activeTokens: tokens,
    bucket,
    endpoint,
    stsMode,
    storageConfigured,
  });
});
