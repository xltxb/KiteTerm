import { env, exports } from "cloudflare:workers";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { totpCodeAt } from "../src/crypto";
import { sendCode } from "../src/mail";

const base = "https://kite.test";
const jsonHeaders = { "content-type": "application/json" };

// 注册接口按 IP 限流(测试环境 KITE_REGISTER_LIMIT=3),每个用例必须用不重复的 IP,
// 否则同文件内多次注册会互相触发 429 rate_limited
let ipCounter = 0;
const nextIp = () => `10.9.${++ipCounter >> 8}.${ipCounter & 0xff}`;

function register(email: string, password: string) {
  return exports.default.fetch(`${base}/api/register`, {
    method: "POST",
    headers: { ...jsonHeaders, "cf-connecting-ip": nextIp() },
    body: JSON.stringify({ email, password }),
  });
}

function login(email: string, password: string, extra: Record<string, unknown> = {}) {
  return exports.default.fetch(`${base}/api/login`, {
    method: "POST",
    headers: jsonHeaders,
    body: JSON.stringify({ email, password, ...extra }),
  });
}

function me(token: string) {
  return exports.default.fetch(`${base}/api/me`, { headers: { authorization: `Bearer ${token}` } });
}

function verifyEmail(token: string, code: string) {
  return exports.default.fetch(`${base}/api/verify-email`, {
    method: "POST",
    headers: { ...jsonHeaders, authorization: `Bearer ${token}` },
    body: JSON.stringify({ code }),
  });
}

function resendVerification(token: string) {
  return exports.default.fetch(`${base}/api/verify-email/resend`, {
    method: "POST",
    headers: { authorization: `Bearer ${token}` },
  });
}

function totpSetup(token: string) {
  return exports.default.fetch(`${base}/api/2fa/setup`, {
    method: "POST",
    headers: { authorization: `Bearer ${token}` },
  });
}

function totpEnable(token: string, code: string) {
  return exports.default.fetch(`${base}/api/2fa/enable`, {
    method: "POST",
    headers: { ...jsonHeaders, authorization: `Bearer ${token}` },
    body: JSON.stringify({ code }),
  });
}

function totpDisable(token: string, code: string) {
  return exports.default.fetch(`${base}/api/2fa/disable`, {
    method: "POST",
    headers: { ...jsonHeaders, authorization: `Bearer ${token}` },
    body: JSON.stringify({ code }),
  });
}

function adminHeaders() {
  return { "x-admin-key": env.KITE_ADMIN_KEY, "content-type": "application/json" };
}

function adminReset2fa(email: string) {
  return exports.default.fetch(`${base}/api/admin/users/${encodeURIComponent(email)}/2fa/reset`, {
    method: "POST",
    headers: adminHeaders(),
  });
}

function adminRevoke(email: string) {
  return exports.default.fetch(`${base}/api/admin/users/${encodeURIComponent(email)}/revoke`, {
    method: "POST",
    headers: adminHeaders(),
  });
}

function adminVerify(email: string) {
  return exports.default.fetch(`${base}/api/admin/users/${encodeURIComponent(email)}/verify`, {
    method: "POST",
    headers: adminHeaders(),
  });
}

/** 直接从 D1 读出该用户当前待验证的验证码,模拟「查收邮件」 */
async function readEmailCode(userId: string): Promise<{ code: string; attempts: number } | null> {
  const row = await env.DB.prepare("SELECT code, attempts FROM email_codes WHERE user_id = ?1").bind(userId).first<{
    code: string;
    attempts: number;
  }>();
  return row ?? null;
}

describe("邮箱验证(日志路径,未配置 RESEND_API_KEY)", () => {
  beforeAll(async () => {
    await env.DB.prepare("INSERT OR REPLACE INTO platform_settings (key, value) VALUES ('feature_email_verify', '1')").run();
  });
  afterAll(async () => {
    await env.DB.prepare("INSERT OR REPLACE INTO platform_settings (key, value) VALUES ('feature_email_verify', '0')").run();
  });

  it("开启邮箱验证后注册 → emailVerified:false,并写入验证码", async () => {
    const r = await register("verify-a@example.com", "Kite#2026");
    expect(r.status).toBe(201);
    const j = (await r.json()) as { userId: string; user: { emailVerified: boolean } };
    expect(j.user.emailVerified).toBe(false);
    const row = await readEmailCode(j.userId);
    expect(row).not.toBeNull();
    expect(row?.code).toMatch(/^\d{6}$/);
  });

  it("错码 ×5 后验证码行被删除,第 5 次提示错误次数过多", async () => {
    const r = await register("verify-b@example.com", "Kite#2026");
    const j = (await r.json()) as { userId: string; token: string };
    for (let i = 0; i < 4; i++) {
      const bad = await verifyEmail(j.token, "000000");
      expect(bad.status).toBe(400);
      const bj = (await bad.json()) as Record<string, unknown>;
      expect(bj).toMatchObject({ error: { code: "CODE_INVALID", message: "验证码错误" } });
    }
    const fifth = await verifyEmail(j.token, "000000");
    expect(fifth.status).toBe(400);
    const fj = (await fifth.json()) as Record<string, unknown>;
    expect(fj).toMatchObject({ error: { code: "CODE_INVALID", message: "验证码错误次数过多,请重新发送" } });
    expect(await readEmailCode(j.userId)).toBeNull();
  });

  it("重发:60 s 内重发 → 429 RESEND_TOO_SOON", async () => {
    const r = await register("verify-c@example.com", "Kite#2026");
    const j = (await r.json()) as { token: string };
    // 注册时已发过一次(sent_at = 刚刚),此时立即重发应被节流
    const resend = await resendVerification(j.token);
    expect(resend.status).toBe(429);
    const rj = (await resend.json()) as Record<string, unknown>;
    expect(rj).toMatchObject({ error: { code: "RESEND_TOO_SOON" } });
  });

  it("从 D1 读码提交验证 → 成功,/api/me 显示 emailVerified:true", async () => {
    const r = await register("verify-d@example.com", "Kite#2026");
    const j = (await r.json()) as { userId: string; token: string };
    const row = await readEmailCode(j.userId);
    expect(row).not.toBeNull();
    const ok = await verifyEmail(j.token, row!.code);
    expect(ok.status).toBe(200);
    const oj = (await ok.json()) as Record<string, unknown>;
    expect(oj).toMatchObject({ ok: true, emailVerified: true });
    expect(await readEmailCode(j.userId)).toBeNull();

    const meRes = await me(j.token);
    const meJson = (await meRes.json()) as Record<string, unknown>;
    expect(meJson.emailVerified).toBe(true);
  });

  it("已验证用户重发 → 直接 ok,不再发码", async () => {
    const r = await register("verify-e@example.com", "Kite#2026");
    const j = (await r.json()) as { userId: string; token: string };
    const row = await readEmailCode(j.userId);
    await verifyEmail(j.token, row!.code);
    const resend = await resendVerification(j.token);
    expect(resend.status).toBe(200);
    const rj = (await resend.json()) as Record<string, unknown>;
    expect(rj).toMatchObject({ ok: true, emailVerified: true });
  });

  it("管理员标记已验证:清空待验证码,用户不存在 → 404", async () => {
    const r = await register("verify-f@example.com", "Kite#2026");
    const j = (await r.json()) as { userId: string };
    expect(await readEmailCode(j.userId)).not.toBeNull();
    const ok = await adminVerify("verify-f@example.com");
    expect(ok.status).toBe(200);
    expect(await readEmailCode(j.userId)).toBeNull();
    const meRow = await env.DB.prepare("SELECT email_verified FROM users WHERE id = ?1").bind(j.userId).first<{ email_verified: number }>();
    expect(meRow?.email_verified).toBe(1);

    const missing = await adminVerify("nobody@example.com");
    expect(missing.status).toBe(404);
    const mj = (await missing.json()) as Record<string, unknown>;
    expect(mj).toMatchObject({ error: { code: "not_found", message: "用户不存在" } });
  });
});

describe("邮箱验证:限流", () => {
  it("同账户第 11 次验证 → 429 CODE_RATE_LIMITED", async () => {
    await env.DB.prepare("INSERT OR REPLACE INTO platform_settings (key, value) VALUES ('feature_email_verify', '1')").run();
    const r = await register("verify-limit@example.com", "Kite#2026");
    const j = (await r.json()) as { token: string };
    for (let i = 0; i < 10; i++) {
      const res = await verifyEmail(j.token, "000000");
      expect(res.status).toBe(400);
    }
    const eleventh = await verifyEmail(j.token, "000000");
    expect(eleventh.status).toBe(429);
    const ej = (await eleventh.json()) as Record<string, unknown>;
    expect(ej).toMatchObject({ error: { code: "CODE_RATE_LIMITED" } });
    await env.DB.prepare("INSERT OR REPLACE INTO platform_settings (key, value) VALUES ('feature_email_verify', '0')").run();
  });
});

describe("双因素认证(TOTP)", () => {
  it("setup → enable → 登录需要验证码 → 强制开关下禁止解绑 → 管理员重置后旧令牌失效", async () => {
    const email = "totp-a@example.com";
    const password = "Kite#2026";
    const reg = await register(email, password);
    const regJson = (await reg.json()) as { token: string };
    const firstToken = regJson.token;

    // setup:生成待确认密钥
    const setup = await totpSetup(firstToken);
    expect(setup.status).toBe(200);
    const setupJson = (await setup.json()) as { secret: string; otpauthUrl: string };
    expect(setupJson.secret).toMatch(/^[A-Z2-7]{32}$/);
    expect(setupJson.otpauthUrl).toContain(`secret=${setupJson.secret}`);

    // enable:用当前验证码确认绑定
    const now = Math.floor(Date.now() / 1000);
    const code = await totpCodeAt(setupJson.secret, now);
    const enable = await totpEnable(firstToken, code);
    expect(enable.status).toBe(200);
    expect(await enable.json()).toMatchObject({ ok: true, totpEnabled: true });

    // 无码登录 → 401 TOTP_REQUIRED
    const noCode = await login(email, password);
    expect(noCode.status).toBe(401);
    expect(await noCode.json()).toMatchObject({ error: { code: "TOTP_REQUIRED", message: "请输入动态验证码" } });

    // 错码登录 → 401 TOTP_INVALID
    const badCode = await login(email, password, { totp: "000000" });
    expect(badCode.status).toBe(401);
    expect(await badCode.json()).toMatchObject({ error: { code: "TOTP_INVALID" } });

    // 带正确码登录 → 200
    const loginNow = Math.floor(Date.now() / 1000);
    const goodCode = await totpCodeAt(setupJson.secret, loginNow);
    const okLogin = await login(email, password, { totp: goodCode });
    expect(okLogin.status).toBe(200);
    const okJson = (await okLogin.json()) as { token: string; user: { totpEnabled: boolean } };
    expect(okJson.user.totpEnabled).toBe(true);
    const secondToken = okJson.token;

    // 强制 2FA 打开时禁止解绑
    await env.DB.prepare("INSERT OR REPLACE INTO platform_settings (key, value) VALUES ('feature_totp_required', '1')").run();
    const disableNow = Math.floor(Date.now() / 1000);
    const disableCode = await totpCodeAt(setupJson.secret, disableNow);
    const denied = await totpDisable(secondToken, disableCode);
    expect(denied.status).toBe(403);
    expect(await denied.json()).toMatchObject({ error: { code: "TOTP_ENFORCED" } });

    // 管理员重置:清空密钥、置 0、删除该用户全部令牌
    const reset = await adminReset2fa(email);
    expect(reset.status).toBe(200);
    expect(await reset.json()).toMatchObject({ ok: true });

    const staleAfterReset = await me(secondToken);
    expect(staleAfterReset.status).toBe(401);

    await env.DB.prepare("INSERT OR REPLACE INTO platform_settings (key, value) VALUES ('feature_totp_required', '0')").run();
  });

  it("setup 重复调用:已启用时 409 TOTP_ALREADY_ENABLED;enable 无密钥 400 TOTP_NOT_PENDING", async () => {
    const email = "totp-b@example.com";
    const password = "Kite#2026";
    const reg = await register(email, password);
    const { token } = (await reg.json()) as { token: string };

    const notPending = await totpEnable(token, "123456");
    expect(notPending.status).toBe(400);
    expect(await notPending.json()).toMatchObject({ error: { code: "TOTP_NOT_PENDING" } });

    const setup = await totpSetup(token);
    const { secret } = (await setup.json()) as { secret: string };
    const code = await totpCodeAt(secret, Math.floor(Date.now() / 1000));
    const enable = await totpEnable(token, code);
    expect(enable.status).toBe(200);

    const again = await totpSetup(token);
    expect(again.status).toBe(409);
    expect(await again.json()).toMatchObject({ error: { code: "TOTP_ALREADY_ENABLED" } });
  });

  it("管理员强制下线(revoke):吊销后旧令牌 401,返回吊销数量", async () => {
    const email = "totp-c@example.com";
    const reg = await register(email, "Kite#2026");
    const { token } = (await reg.json()) as { token: string };
    expect((await me(token)).status).toBe(200);

    const revoke = await adminRevoke(email);
    expect(revoke.status).toBe(200);
    const rj = (await revoke.json()) as { ok: boolean; revoked: number };
    expect(rj.ok).toBe(true);
    expect(rj.revoked).toBeGreaterThanOrEqual(1);

    expect((await me(token)).status).toBe(401);
  });

  it("2fa/reset 与 revoke 对不存在的用户:reset 404,revoke 返回 revoked:0", async () => {
    const missingReset = await adminReset2fa("nobody-totp@example.com");
    expect(missingReset.status).toBe(404);
    expect(await missingReset.json()).toMatchObject({ error: { code: "not_found", message: "用户不存在" } });

    const missingRevoke = await adminRevoke("nobody-totp@example.com");
    expect(missingRevoke.status).toBe(200);
    expect(await missingRevoke.json()).toMatchObject({ ok: true, revoked: 0 });
  });
});

describe("sendCode:Resend 路径", () => {
  it("配置 RESEND_API_KEY 时 POST 到 Resend API", async () => {
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    const fakeFetch = vi.fn(async (url: string, init?: RequestInit) => {
      calls.push({ url, init });
      return new Response(JSON.stringify({ id: "x" }), { status: 200 });
    });
    vi.stubGlobal("fetch", fakeFetch);
    try {
      const fakeEnv = { RESEND_API_KEY: "re_test_key", KITE_SMTP_FROM: "no-reply@kiteterm.dev" } as unknown as Parameters<typeof sendCode>[0];
      await sendCode(fakeEnv, "user@example.com", "123456");
      expect(fakeFetch).toHaveBeenCalledTimes(1);
      expect(calls[0].url).toBe("https://api.resend.com/emails");
      const body = JSON.parse(String(calls[0].init?.body));
      expect(body).toMatchObject({
        from: "no-reply@kiteterm.dev",
        to: "user@example.com",
        subject: "KiteTerm 邮箱验证码",
      });
      expect(body.text).toContain("123456");
      const headers = calls[0].init?.headers as Record<string, string>;
      expect(headers.authorization).toBe("Bearer re_test_key");
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("Resend 返回非 2xx 时抛错", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("bad request", { status: 400 })),
    );
    try {
      const fakeEnv = { RESEND_API_KEY: "re_test_key", KITE_SMTP_FROM: "no-reply@kiteterm.dev" } as unknown as Parameters<typeof sendCode>[0];
      await expect(sendCode(fakeEnv, "user@example.com", "123456")).rejects.toThrow();
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
