import { env, exports } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import * as features from "../src/features";
import { settingSet } from "../src/settings";

const base = "https://kite.test";
const jsonHeaders = (ip?: string) => ({
  "content-type": "application/json",
  ...(ip ? { "cf-connecting-ip": ip } : {}),
});

function register(email: string, password: string, ip: string, extra: Record<string, unknown> = {}) {
  return exports.default.fetch(`${base}/api/register`, {
    method: "POST",
    headers: jsonHeaders(ip),
    body: JSON.stringify({ email, password, ...extra }),
  });
}

function login(email: string, password: string, extra: Record<string, unknown> = {}) {
  return exports.default.fetch(`${base}/api/login`, {
    method: "POST",
    headers: jsonHeaders(),
    body: JSON.stringify({ email, password, ...extra }),
  });
}

function me(token?: string) {
  return exports.default.fetch(`${base}/api/me`, {
    headers: token ? { authorization: `Bearer ${token}` } : {},
  });
}

describe("register", () => {
  it("成功注册 → 201,带 token 与 user 信息", async () => {
    const r = await register("alice@example.com", "Kite#2026", "10.0.0.1");
    expect(r.status).toBe(201);
    const j = (await r.json()) as Record<string, unknown>;
    expect(typeof j.userId).toBe("string");
    expect(typeof j.token).toBe("string");
    expect(typeof j.expiresAt).toBe("number");
    expect(j.totpSetupRequired).toBe(false);
    expect(j).toMatchObject({
      user: { email: "alice@example.com", syncEnabled: true, emailVerified: true, totpEnabled: false },
    });
  });

  it("重复邮箱 → 409 email_taken", async () => {
    const first = await register("bob@example.com", "Kite#2026", "10.0.0.2");
    expect(first.status).toBe(201);
    const second = await register("bob@example.com", "Kite#2026", "10.0.0.2");
    expect(second.status).toBe(409);
    const j = (await second.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "email_taken", message: "该邮箱已注册" } });
  });

  it("弱口令 → 400 weak_password", async () => {
    const r = await register("carol@example.com", "1234567", "10.0.0.3");
    expect(r.status).toBe(400);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "weak_password" } });
    expect((j.error as { message: string }).message).toBe("密码至少 8 位");
  });

  it("同一 IP 第 4 次注册 → 429 rate_limited;换 IP 仍可成功", async () => {
    const sameIp = "10.0.0.9";
    for (let i = 0; i < 3; i++) {
      const r = await register(`limit${i}@example.com`, "Kite#2026", sameIp);
      expect(r.status).toBe(201);
    }
    const fourth = await register("limit3@example.com", "Kite#2026", sameIp);
    expect(fourth.status).toBe(429);
    const j = (await fourth.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "rate_limited", message: "注册过于频繁,请稍后再试" } });

    const otherIp = await register("limit-other-ip@example.com", "Kite#2026", "10.0.0.10");
    expect(otherIp.status).toBe(201);
  });
});

describe("login", () => {
  it("成功登录 → 200,带 token", async () => {
    await register("dave@example.com", "Kite#2026", "10.0.1.1");
    const r = await login("dave@example.com", "Kite#2026");
    expect(r.status).toBe(200);
    const j = (await r.json()) as Record<string, unknown>;
    expect(typeof j.token).toBe("string");
    expect(j).toMatchObject({ user: { email: "dave@example.com" } });
  });

  it("错误口令 → 401 bad_credentials", async () => {
    await register("erin@example.com", "Kite#2026", "10.0.1.2");
    const r = await login("erin@example.com", "wrong-password");
    expect(r.status).toBe(401);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "bad_credentials", message: "邮箱或密码错误" } });
  });

  it("不存在的邮箱 → 401 bad_credentials", async () => {
    const r = await login("nobody@example.com", "Kite#2026");
    expect(r.status).toBe(401);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "bad_credentials", message: "邮箱或密码错误" } });
  });
});

describe("LOGIN_DISABLED", () => {
  it("单元测试:feature_login=0 时 effective(db, true, 'login') 为 false", async () => {
    await settingSet(env.DB, "feature_login", "0");
    expect(await features.effective(env.DB, true, "login")).toBe(false);
    // 总闸关闭(gating=false)时,login 是受门禁保护的功能,直接放行
    expect(await features.effective(env.DB, false, "login")).toBe(true);
  });

  it("HTTP:feature_login=0 但门禁总闸(KITE_FEATURE_GATING)关闭时,登录仍然成功", async () => {
    await register("frank@example.com", "Kite#2026", "10.0.1.3");
    await settingSet(env.DB, "feature_login", "0");
    const r = await login("frank@example.com", "Kite#2026");
    expect(r.status).toBe(200);
  });
});

describe("me / logout", () => {
  it("带令牌 200,无令牌 401", async () => {
    const reg = await register("grace@example.com", "Kite#2026", "10.0.2.1");
    const { token } = (await reg.json()) as { token: string };

    const withToken = await me(token);
    expect(withToken.status).toBe(200);
    const j = (await withToken.json()) as Record<string, unknown>;
    expect(j).toMatchObject({
      email: "grace@example.com",
      requirements: { emailVerify: false, totpRequired: false },
    });

    const withoutToken = await me();
    expect(withoutToken.status).toBe(401);
    const j2 = (await withoutToken.json()) as Record<string, unknown>;
    expect(j2).toMatchObject({ error: { code: "unauthorized", message: "缺少令牌" } });
  });

  it("logout 后旧令牌变为无效", async () => {
    const reg = await register("heidi@example.com", "Kite#2026", "10.0.2.2");
    const { token } = (await reg.json()) as { token: string };

    const logoutRes = await exports.default.fetch(`${base}/api/logout`, {
      method: "POST",
      headers: { authorization: `Bearer ${token}` },
    });
    expect(logoutRes.status).toBe(200);
    expect(await logoutRes.json()).toMatchObject({ ok: true });

    const after = await me(token);
    expect(after.status).toBe(401);
    const j = (await after.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "unauthorized", message: "令牌无效" } });
  });
});

describe("邀请码注册", () => {
  it("invite 模式下缺邀请码 → 400 invite_required", async () => {
    await settingSet(env.DB, "registration_mode", "invite");
    const r = await register("judy@example.com", "Kite#2026", "10.0.4.1");
    expect(r.status).toBe(400);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "invite_required", message: "该平台需要邀请码才能注册" } });
  });

  it("不存在的邀请码 → 400 invite_invalid(邀请码无效)", async () => {
    const r = await register("kevin@example.com", "Kite#2026", "10.0.4.2", { invite_code: "NOPE-NOPE-NOPE" });
    expect(r.status).toBe(400);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "invite_invalid", message: "邀请码无效" } });
  });

  it("已停用的邀请码 → 400 invite_invalid(邀请码已停用)", async () => {
    await env.DB.prepare(
      "INSERT INTO invites (code, note, max_uses, used_count, expires_at, disabled, created_at) VALUES ('DISA-BLED-CODE', '', 0, 0, 0, 1, 0)",
    ).run();
    const r = await register("laura@example.com", "Kite#2026", "10.0.4.3", { invite_code: "DISA-BLED-CODE" });
    expect(r.status).toBe(400);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "invite_invalid", message: "邀请码已停用" } });
  });

  it("已过期的邀请码 → 400 invite_expired", async () => {
    await env.DB.prepare(
      "INSERT INTO invites (code, note, max_uses, used_count, expires_at, disabled, created_at) VALUES ('EXPI-RED0-CODE', '', 0, 0, 1, 0, 0)",
    ).run();
    const r = await register("mallory@example.com", "Kite#2026", "10.0.4.4", { invite_code: "EXPI-RED0-CODE" });
    expect(r.status).toBe(400);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "invite_expired", message: "邀请码已过期" } });
  });

  it("已用尽的邀请码 → 400 invite_exhausted", async () => {
    await env.DB.prepare(
      "INSERT INTO invites (code, note, max_uses, used_count, expires_at, disabled, created_at) VALUES ('EXHA-USTE-D000', '', 1, 1, 0, 0, 0)",
    ).run();
    const r = await register("nathan@example.com", "Kite#2026", "10.0.4.5", { invite_code: "EXHA-USTE-D000" });
    expect(r.status).toBe(400);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "invite_exhausted", message: "邀请码使用次数已用尽" } });
  });

  it("有效邀请码 → 201,且 used_count 增加、写入 invite_uses", async () => {
    await env.DB.prepare(
      "INSERT INTO invites (code, note, max_uses, used_count, expires_at, disabled, created_at) VALUES ('GOOD-CODE-0001', '', 5, 0, 0, 0, 0)",
    ).run();
    const r = await register("olivia@example.com", "Kite#2026", "10.0.4.6", { invite_code: "GOOD-CODE-0001" });
    expect(r.status).toBe(201);
    const j = (await r.json()) as { userId: string };

    const invite = await env.DB.prepare("SELECT used_count FROM invites WHERE code = 'GOOD-CODE-0001'").first<{ used_count: number }>();
    expect(invite?.used_count).toBe(1);
    const use = await env.DB.prepare("SELECT user_id FROM invite_uses WHERE code = 'GOOD-CODE-0001'").first<{ user_id: string }>();
    expect(use?.user_id).toBe(j.userId);
  });

  it("恢复为 open 模式,不影响其它测试", async () => {
    await settingSet(env.DB, "registration_mode", "open");
    const r = await register("peter@example.com", "Kite#2026", "10.0.4.7");
    expect(r.status).toBe(201);
  });
});

describe("多设备策略", () => {
  it("feature_multi_device=0 时,二次登录挤掉旧令牌", async () => {
    await settingSet(env.DB, "feature_multi_device", "0");
    const reg = await register("ivan@example.com", "Kite#2026", "10.0.3.1");
    const { token: token1 } = (await reg.json()) as { token: string };

    // 注册签发的令牌在多设备关闭时仍然有效(挤兑只发生在登录时)
    expect((await me(token1)).status).toBe(200);

    const loginRes = await login("ivan@example.com", "Kite#2026");
    expect(loginRes.status).toBe(200);
    const { token: token2 } = (await loginRes.json()) as { token: string };
    expect(token2).not.toBe(token1);

    const oldTokenRes = await me(token1);
    expect(oldTokenRes.status).toBe(401);
    const j = (await oldTokenRes.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "unauthorized", message: "令牌无效" } });

    expect((await me(token2)).status).toBe(200);
  });
});
