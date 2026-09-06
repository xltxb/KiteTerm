import { env, exports } from "cloudflare:workers";
import { describe, expect, it } from "vitest";

const base = "https://kite.test";
const adminHeaders = { "x-admin-key": env.KITE_ADMIN_KEY, "content-type": "application/json" };

describe("admin settings", () => {
  it("无密钥 401", async () => {
    const r = await exports.default.fetch(`${base}/api/admin/settings`);
    expect(r.status).toBe(401);
    const j = await r.json() as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "unauthorized", message: "管理密钥错误" } });
  });

  it("POST 修改功能开关,返回值包含两键", async () => {
    const r = await exports.default.fetch(`${base}/api/admin/settings`, {
      method: "POST",
      headers: adminHeaders,
      body: JSON.stringify({ featureAiSummary: false, featureTotpRequired: true }),
    });
    expect(r.status).toBe(200);
    const j = await r.json() as Record<string, unknown>;
    expect(j).toMatchObject({ featureAiSummary: false, featureTotpRequired: true });
  });

  it("未知开关 → 400 UNKNOWN_FEATURE", async () => {
    const r = await exports.default.fetch(`${base}/api/admin/settings`, {
      method: "POST",
      headers: adminHeaders,
      body: JSON.stringify({ featureNope: true }),
    });
    expect(r.status).toBe(400);
    const j = await r.json() as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "UNKNOWN_FEATURE", message: "未知功能开关 featureNope" } });
  });

  it("非法注册策略 → 400 invalid_mode", async () => {
    const r = await exports.default.fetch(`${base}/api/admin/settings`, {
      method: "POST",
      headers: adminHeaders,
      body: JSON.stringify({ registrationMode: "x" }),
    });
    expect(r.status).toBe(400);
    const j = await r.json() as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "invalid_mode", message: "注册策略只能是 open/invite/closed" } });
  });

  it("邀请码:创建 → 列表 → 停用 → 停用不存在的邀请码 404", async () => {
    const created = await exports.default.fetch(`${base}/api/admin/invites`, {
      method: "POST",
      headers: adminHeaders,
      body: JSON.stringify({ note: "测试邀请", maxUses: 3, expiresInDays: 7 }),
    });
    expect(created.status).toBe(200);
    const createdJson = await created.json() as Record<string, unknown>;
    expect(createdJson.maxUses).toBe(3);
    expect(typeof createdJson.code).toBe("string");
    const code = createdJson.code as string;
    expect(code).toMatch(/^[A-Z0-9]{4}-[A-Z0-9]{4}-[A-Z0-9]{4}$/);

    const listed = await exports.default.fetch(`${base}/api/admin/invites`, { headers: adminHeaders });
    const listedJson = await listed.json() as { invites: Array<Record<string, unknown>> };
    const found = listedJson.invites.find((i) => i.code === code);
    expect(found).toMatchObject({ code, note: "测试邀请", maxUses: 3, usedCount: 0, disabled: false });

    const disabled = await exports.default.fetch(`${base}/api/admin/invites/${code}/disable`, {
      method: "POST",
      headers: adminHeaders,
    });
    expect(disabled.status).toBe(200);
    expect(await disabled.json()).toMatchObject({ ok: true });

    // 注意:与 Rust 版一致,`UPDATE ... WHERE code = ?1` 不检查 disabled 现状 ——
    // SQLite 的 changes() 按「WHERE 是否匹配到行」计数,而非「值是否真的变化」,
    // 所以对同一个已停用的邀请码再次停用仍返回 200/ok:true,不是 404。
    // 404 only 发生在邀请码本不存在时,这里用一个从未创建过的邀请码验证 not_found。
    const disabledNonexistent = await exports.default.fetch(`${base}/api/admin/invites/NOPE-NOPE-NOPE/disable`, {
      method: "POST",
      headers: adminHeaders,
    });
    expect(disabledNonexistent.status).toBe(404);
    const disabledNonexistentJson = await disabledNonexistent.json() as Record<string, unknown>;
    expect(disabledNonexistentJson).toMatchObject({ error: { code: "not_found", message: "邀请码不存在" } });
  });

  it("用户列表初始为空数组", async () => {
    const r = await exports.default.fetch(`${base}/api/admin/users`, { headers: adminHeaders });
    expect(r.status).toBe(200);
    const j = await r.json() as { users: unknown[] };
    expect(j.users).toEqual([]);
  });
});
