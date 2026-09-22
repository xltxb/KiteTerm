import { env, exports } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
const base = "https://kite.test";
describe("features", () => {
  it("默认值与门禁", async () => {
    const r = await exports.default.fetch(`${base}/api/features`);
    const j = await r.json() as Record<string, unknown>;
    expect(j).toMatchObject({ login: true, keySync: true, aiSummary: true, aiAgent: true, emailVerify: false, totpRequired: false, multiDevice: true, enforced: false, registration: "open" });
    expect(j.passwordMinLength).toBe(8);
    // 关掉 login:总闸 off 时生效值仍为 true
    await env.DB.prepare("INSERT INTO platform_settings (key,value) VALUES ('feature_login','0')").run();
    const j2 = await (await exports.default.fetch(`${base}/api/features`)).json() as Record<string, unknown>;
    expect(j2.login).toBe(true);
    // 安全策略不看总闸
    await env.DB.prepare("INSERT INTO platform_settings (key,value) VALUES ('feature_totp_required','1')").run();
    const j3 = await (await exports.default.fetch(`${base}/api/features`)).json() as Record<string, unknown>;
    expect(j3.totpRequired).toBe(true);
  });
});
