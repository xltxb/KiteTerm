// 凭据下发:R2 临时凭据 / 通用 STS AssumeRole / 共享回退,以及 `/api/sync/credentials`
// 的检查顺序。插件 `@cloudflare/vitest-plugin`(当前版本)不导出 `cloudflare:test` 的
// `fetchMock`,改用 `vi.stubGlobal("fetch", ...)` 拦截出站请求 —— 已用一个临时用例验证过
// 这在本插件下对 Worker 的出站 fetch(包括经 `exports.default.fetch` 触发的路由内部调用)
// 同样生效,故路由级用例与单元级用例都用这一套。
// R2 场景需要 R2_API_TOKEN(R2_ACCOUNT_ID 可留空由 endpoint 解析)env 绑定,而 vitest 的全局 binding
// 没法按用例覆盖,所以 R2 / 通用 STS 的凭据换取直接单元调用 `issueCredentials`,
// 传入自定义 env 对象;检查顺序、共享回退等经 HTTP 路由验证。
import { env, exports } from "cloudflare:workers";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Env } from "../src/env";
import { issueCredentials, isR2Endpoint, r2AccountId, StsError } from "../src/creds";
import { probe, type StorageConfig } from "../src/storage";

const base = "https://kite.test";
const adminHeaders = { "x-admin-key": env.KITE_ADMIN_KEY, "content-type": "application/json" };
const jsonHeaders = { "content-type": "application/json" };

let ipCounter = 0;
const nextIp = () => `10.30.${++ipCounter >> 8}.${ipCounter & 0xff}`;

function register(email: string, password: string) {
  return exports.default.fetch(`${base}/api/register`, {
    method: "POST",
    headers: { ...jsonHeaders, "cf-connecting-ip": nextIp() },
    body: JSON.stringify({ email, password }),
  });
}
function credentialsReq(token: string) {
  return exports.default.fetch(`${base}/api/sync/credentials`, { headers: { authorization: `Bearer ${token}` } });
}
function postStorage(body: Record<string, unknown>) {
  return exports.default.fetch(`${base}/api/admin/storage`, { method: "POST", headers: adminHeaders, body: JSON.stringify(body) });
}
function setSync(email: string, enabled: boolean) {
  return exports.default.fetch(`${base}/api/admin/users/${email}/sync`, {
    method: "POST",
    headers: adminHeaders,
    body: JSON.stringify({ enabled }),
  });
}
function setFeatures(body: Record<string, unknown>) {
  return exports.default.fetch(`${base}/api/admin/settings`, { method: "POST", headers: adminHeaders, body: JSON.stringify(body) });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

const baseCfg: StorageConfig = {
  endpoint: "https://s3.test.local",
  region: "us-east-1",
  bucket: "test-bucket",
  prefix: "",
  accessKey: "AK",
  secretKey: "SK",
  pathStyle: true,
  stsMode: "auto",
};

function fakeRequestUrl(input: RequestInfo | URL): string {
  return input instanceof Request ? input.url : input.toString();
}
function fakeRequestMethod(input: RequestInfo | URL, init?: RequestInit): string {
  return input instanceof Request ? input.method : (init?.method ?? "GET");
}

describe("isR2Endpoint", () => {
  it("只认 .r2.cloudflarestorage.com 域名", () => {
    expect(isR2Endpoint("https://acct123.r2.cloudflarestorage.com")).toBe(true);
    expect(isR2Endpoint("https://s3.amazonaws.com")).toBe(false);
    expect(isR2Endpoint("not a url")).toBe(false);
  });
});

describe("r2AccountId", () => {
  it("从 R2 endpoint 的首段子域解析账号 ID,含 jurisdiction 端点;非 R2 返回 undefined", () => {
    expect(r2AccountId("https://acct123.r2.cloudflarestorage.com")).toBe("acct123");
    expect(r2AccountId("https://acct123.eu.r2.cloudflarestorage.com")).toBe("acct123");
    expect(r2AccountId("https://s3.amazonaws.com")).toBeUndefined();
    expect(r2AccountId("https://r2.cloudflarestorage.com")).toBeUndefined();
    expect(r2AccountId("not a url")).toBeUndefined();
  });
});

describe("issueCredentials —— R2 临时凭据(单元调用,注入 R2_ACCOUNT_ID/R2_API_TOKEN)", () => {
  it.each([
    { label: "prefix 为空", prefix: "", expectedUserPrefix: "users/user-1" },
    { label: "prefix 为 prod/", prefix: "prod/", expectedUserPrefix: "prod/users/user-1" },
  ])("成功($label):请求方法/请求头/请求体精确匹配,返回 sessionToken 与按用户收敛的 prefix", async ({ prefix, expectedUserPrefix }) => {
    let captured: { url: string; method: string; headers: Headers; body: string } | undefined;
    vi.stubGlobal("fetch", async (input: RequestInfo | URL, init?: RequestInit) => {
      captured = {
        url: fakeRequestUrl(input),
        method: fakeRequestMethod(input, init),
        headers: new Headers(init?.headers),
        body: String(init?.body ?? ""),
      };
      return new Response(
        JSON.stringify({ success: true, result: { accessKeyId: "ak", secretAccessKey: "sk", sessionToken: "st" } }),
        { status: 200 },
      );
    });
    const cfg: StorageConfig = { ...baseCfg, endpoint: "https://acct123.r2.cloudflarestorage.com", prefix };
    const fakeEnv = { R2_ACCOUNT_ID: "acct123", R2_API_TOKEN: "tok" } as unknown as Env;
    const creds = await issueCredentials(cfg, fakeEnv, "user-1");

    expect(captured?.url).toBe("https://api.cloudflare.com/client/v4/accounts/acct123/r2/temp-access-credentials");
    expect(captured?.method).toBe("POST");
    expect(captured?.headers.get("authorization")).toBe("Bearer tok");
    expect(captured?.headers.get("content-type")).toBe("application/json");
    expect(JSON.parse(captured?.body ?? "{}")).toEqual({
      bucket: cfg.bucket,
      parentAccessKeyId: cfg.accessKey,
      permission: "object-read-write",
      ttlSeconds: 3600,
      prefixes: [`${expectedUserPrefix}/`],
    });

    expect(creds).toMatchObject({ accessKey: "ak", secretKey: "sk", sessionToken: "st", prefix: expectedUserPrefix });
    expect(typeof creds.expiration).toBe("string");
  });

  it("success:false;stsMode=require → 抛 StsError", async () => {
    vi.stubGlobal(
      "fetch",
      async () => new Response(JSON.stringify({ success: false, errors: [{ message: "invalid token" }] }), { status: 200 }),
    );
    const cfg: StorageConfig = { ...baseCfg, endpoint: "https://acct123.r2.cloudflarestorage.com", stsMode: "require" };
    const fakeEnv = { R2_ACCOUNT_ID: "acct123", R2_API_TOKEN: "tok" } as unknown as Env;
    await expect(issueCredentials(cfg, fakeEnv, "user-1")).rejects.toBeInstanceOf(StsError);
  });

  it("success:false;stsMode=auto → 回退共享凭据(无 sessionToken)", async () => {
    vi.stubGlobal(
      "fetch",
      async () => new Response(JSON.stringify({ success: false, errors: [{ message: "invalid token" }] }), { status: 200 }),
    );
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const cfg: StorageConfig = { ...baseCfg, endpoint: "https://acct123.r2.cloudflarestorage.com", stsMode: "auto" };
    const fakeEnv = { R2_ACCOUNT_ID: "acct123", R2_API_TOKEN: "tok" } as unknown as Env;
    const creds = await issueCredentials(cfg, fakeEnv, "user-1");
    expect(creds.sessionToken).toBeUndefined();
    expect(creds.accessKey).toBe(cfg.accessKey);
    expect(warn).toHaveBeenCalledOnce();
    warn.mockRestore();
  });

  it("R2_ACCOUNT_ID 为空时从 endpoint 解析账号 ID;显式设置时以显式值为准", async () => {
    const urls: string[] = [];
    vi.stubGlobal("fetch", async (input: RequestInfo | URL) => {
      urls.push(fakeRequestUrl(input));
      return new Response(
        JSON.stringify({ success: true, result: { accessKeyId: "ak", secretAccessKey: "sk", sessionToken: "st" } }),
        { status: 200 },
      );
    });
    const cfg: StorageConfig = { ...baseCfg, endpoint: "https://fromhost.r2.cloudflarestorage.com" };
    await issueCredentials(cfg, { R2_ACCOUNT_ID: "", R2_API_TOKEN: "tok" } as unknown as Env, "user-1");
    await issueCredentials(cfg, { R2_API_TOKEN: "tok" } as unknown as Env, "user-1");
    await issueCredentials(cfg, { R2_ACCOUNT_ID: "explicit", R2_API_TOKEN: "tok" } as unknown as Env, "user-1");
    expect(urls).toEqual([
      "https://api.cloudflare.com/client/v4/accounts/fromhost/r2/temp-access-credentials",
      "https://api.cloudflare.com/client/v4/accounts/fromhost/r2/temp-access-credentials",
      "https://api.cloudflare.com/client/v4/accounts/explicit/r2/temp-access-credentials",
    ]);
  });

  it("endpoint 是 R2 但缺 R2_API_TOKEN → 落到通用 STS(不会去请求 api.cloudflare.com)", async () => {
    vi.stubGlobal("fetch", async (input: RequestInfo | URL) => {
      expect(fakeRequestUrl(input)).not.toContain("api.cloudflare.com");
      const xml =
        "<AssumeRoleResponse><AssumeRoleResult><Credentials>" +
        "<AccessKeyId>tak</AccessKeyId><SecretAccessKey>tsk</SecretAccessKey>" +
        "<SessionToken>tst</SessionToken><Expiration>2030-01-01T00:00:00Z</Expiration>" +
        "</Credentials></AssumeRoleResult></AssumeRoleResponse>";
      return new Response(xml, { status: 200 });
    });
    const cfg: StorageConfig = { ...baseCfg, endpoint: "https://acct123.r2.cloudflarestorage.com" };
    const creds = await issueCredentials(cfg, {} as unknown as Env, "user-2");
    expect(creds.sessionToken).toBe("tst");
  });
});

describe("issueCredentials —— 通用 STS AssumeRole(非 R2 endpoint)", () => {
  it("成功:表单体正确、解析 XML 标签,且 session policy 精确收敛到该用户前缀", async () => {
    const cfg: StorageConfig = { ...baseCfg, endpoint: "https://sts.minio.local", prefix: "prod/" };
    vi.stubGlobal("fetch", async (input: RequestInfo | URL) => {
      const req = input instanceof Request ? input : new Request(input);
      expect(req.method).toBe("POST");
      const body = await req.text();
      expect(body).toContain("Action=AssumeRole");
      expect(body).toContain("DurationSeconds=3600");
      expect(body).toContain("RoleSessionName=kite-user-9");

      const policyRaw = new URLSearchParams(body).get("Policy");
      expect(policyRaw).toBeTruthy();
      const policy = JSON.parse(decodeURIComponent(policyRaw ?? ""));
      expect(policy.Statement[0].Resource[0]).toBe(`arn:aws:s3:::${cfg.bucket}/${cfg.prefix}users/user-9/*`);
      expect(policy.Statement[1].Condition.StringLike["s3:prefix"][0]).toBe(`${cfg.prefix}users/user-9/*`);

      const xml =
        "<AssumeRoleResponse><AssumeRoleResult><Credentials>" +
        "<AccessKeyId>tak</AccessKeyId><SecretAccessKey>tsk</SecretAccessKey>" +
        "<SessionToken>tst</SessionToken><Expiration>2030-01-01T00:00:00Z</Expiration>" +
        "</Credentials></AssumeRoleResult></AssumeRoleResponse>";
      return new Response(xml, { status: 200 });
    });
    const creds = await issueCredentials(cfg, {} as unknown as Env, "user-9");
    expect(creds).toMatchObject({
      accessKey: "tak",
      secretKey: "tsk",
      sessionToken: "tst",
      expiration: "2030-01-01T00:00:00Z",
      prefix: "prod/users/user-9",
    });
  });

  it("失败:stsMode=require → 抛 StsError;stsMode=auto → 回退共享凭据", async () => {
    vi.stubGlobal("fetch", async () => new Response("access denied", { status: 403 }));
    const requireCfg: StorageConfig = { ...baseCfg, endpoint: "https://sts.minio.local", stsMode: "require" };
    await expect(issueCredentials(requireCfg, {} as unknown as Env, "u")).rejects.toBeInstanceOf(StsError);

    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const autoCfg: StorageConfig = { ...baseCfg, endpoint: "https://sts.minio.local", stsMode: "auto" };
    const creds = await issueCredentials(autoCfg, {} as unknown as Env, "u");
    expect(creds.sessionToken).toBeUndefined();
    expect(creds.accessKey).toBe(autoCfg.accessKey);
    warn.mockRestore();
  });
});

describe("issueCredentials —— stsMode=off", () => {
  it("直接返回共享凭据,不发任何出站请求", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    const cfg: StorageConfig = { ...baseCfg, stsMode: "off", prefix: "prod/" };
    const creds = await issueCredentials(cfg, {} as unknown as Env, "u1");
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(creds).toMatchObject({ accessKey: "AK", secretKey: "SK", prefix: "prod/users/u1" });
    expect(creds.sessionToken).toBeUndefined();
    expect(creds.expiration).toBeUndefined();
  });
});

describe("probe —— 模拟 S3 端点", () => {
  const cfg: StorageConfig = { ...baseCfg, stsMode: "off" };
  const LIST_XML = "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>";

  it("list/put/delete 均成功 → ok:true 且三项都不带原因", async () => {
    vi.stubGlobal("fetch", async (input: RequestInfo | URL, init?: RequestInit) => {
      const method = fakeRequestMethod(input, init);
      if (method === "GET") return new Response(LIST_XML, { status: 200 });
      if (method === "PUT") return new Response("", { status: 200 });
      if (method === "DELETE") return new Response(null, { status: 204 });
      return new Response("unexpected", { status: 500 });
    });
    const report = await probe(cfg, {} as unknown as Env);
    expect(report.ok).toBe(true);
    expect(report.list).toBeUndefined();
    expect(report.put).toBeUndefined();
    expect(report.delete).toBeUndefined();
    expect(typeof report.latencyMs).toBe("number");
  });

  it("put 返回 403 → ok:false 且 put 带原因,delete 不会被调用", async () => {
    const calledMethods: string[] = [];
    vi.stubGlobal("fetch", async (input: RequestInfo | URL, init?: RequestInit) => {
      const method = fakeRequestMethod(input, init);
      calledMethods.push(method);
      if (method === "GET") return new Response(LIST_XML, { status: 200 });
      if (method === "PUT") return new Response("Forbidden", { status: 403 });
      if (method === "DELETE") return new Response(null, { status: 204 });
      return new Response("unexpected", { status: 500 });
    });
    const report = await probe(cfg, {} as unknown as Env);
    expect(report.ok).toBe(false);
    expect(report.put).toContain("403");
    expect(report.delete).toBeUndefined();
    expect(calledMethods).not.toContain("DELETE");
  });
});

describe("probe —— STS 分支", () => {
  const LIST_XML = "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>";
  const ASSUME_ROLE_XML =
    "<AssumeRoleResponse><AssumeRoleResult><Credentials>" +
    "<AccessKeyId>tak</AccessKeyId><SecretAccessKey>tsk</SecretAccessKey>" +
    "<SessionToken>tst</SessionToken><Expiration>2030-01-01T00:00:00Z</Expiration>" +
    "</Credentials></AssumeRoleResult></AssumeRoleResponse>";

  /** list/put/delete 都模拟成功;STS(唯一走 POST 的请求)按传入的响应工厂返回 */
  function stubS3AndSts(stsResponse: () => Response) {
    vi.stubGlobal("fetch", async (input: RequestInfo | URL, init?: RequestInit) => {
      const method = fakeRequestMethod(input, init);
      if (method === "GET") return new Response(LIST_XML, { status: 200 });
      if (method === "PUT") return new Response("", { status: 200 });
      if (method === "DELETE") return new Response(null, { status: 204 });
      if (method === "POST") return stsResponse();
      return new Response("unexpected", { status: 500 });
    });
  }

  it("stsMode=auto,STS 返回 403 → report.sts 非空字符串,但 list/put/delete 都成功时 ok 仍为 true", async () => {
    stubS3AndSts(() => new Response("access denied", { status: 403 }));
    const cfg: StorageConfig = { ...baseCfg, stsMode: "auto" };
    const report = await probe(cfg, {} as unknown as Env);
    expect(report.ok).toBe(true);
    expect(typeof report.sts).toBe("string");
    expect((report.sts ?? "").length).toBeGreaterThan(0);
  });

  it("stsMode=require,STS 失败 → ok:false", async () => {
    stubS3AndSts(() => new Response("access denied", { status: 403 }));
    const cfg: StorageConfig = { ...baseCfg, stsMode: "require" };
    const report = await probe(cfg, {} as unknown as Env);
    expect(report.ok).toBe(false);
    expect(typeof report.sts).toBe("string");
  });

  it("stsMode=auto,STS 成功 → sts 为 undefined", async () => {
    stubS3AndSts(() => new Response(ASSUME_ROLE_XML, { status: 200 }));
    const cfg: StorageConfig = { ...baseCfg, stsMode: "auto" };
    const report = await probe(cfg, {} as unknown as Env);
    expect(report.sts).toBeUndefined();
    expect(report.ok).toBe(true);
  });
});

describe("/api/sync/credentials 检查顺序", () => {
  it("未配置存储 → 503 STORAGE_UNCONFIGURED", async () => {
    const reg = await register("sync-nostorage@example.com", "Kite#2026");
    const { token } = (await reg.json()) as { token: string };
    const r = await credentialsReq(token);
    expect(r.status).toBe(503);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "STORAGE_UNCONFIGURED", message: "平台尚未配置存储,请联系管理员" } });
  });

  it("配置存储后:停用用户同步 + 平台强制 2FA(用户未绑定)→ 命中用户级 SYNC_DISABLED,而非 TOTP_SETUP_REQUIRED", async () => {
    // stsMode off,后续用例走共享凭据即可,规避对真实网络的依赖
    const saved = await postStorage({ endpoint: "https://s3.example.com", bucket: "b1", accessKey: "ak", secretKey: "sk", stsMode: "off" });
    expect(saved.status).toBe(200);
    await setFeatures({ featureTotpRequired: true });

    const email = "sync-order@example.com";
    const reg = await register(email, "Kite#2026");
    const { token } = (await reg.json()) as { token: string };
    await setSync(email, false);

    const r = await credentialsReq(token);
    expect(r.status).toBe(403);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "SYNC_DISABLED", message: "该账户的云同步已被平台停用,请联系管理员" } });

    await setFeatures({ featureTotpRequired: false });
  });

  it("同步已放开、平台要求邮箱验证且用户未验证 → 403 EMAIL_UNVERIFIED(先于强制 2FA 检查)", async () => {
    await setFeatures({ featureEmailVerify: true, featureTotpRequired: true });
    const email = "sync-emailverify@example.com";
    const reg = await register(email, "Kite#2026");
    const { token } = (await reg.json()) as { token: string };
    const r = await credentialsReq(token);
    expect(r.status).toBe(403);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "EMAIL_UNVERIFIED" } });
    await setFeatures({ featureEmailVerify: false, featureTotpRequired: false });
  });

  it("一切放开(stsMode off)→ 200,返回共享凭据与用户前缀", async () => {
    const email = "sync-ok@example.com";
    const reg = await register(email, "Kite#2026");
    const { token, userId } = (await reg.json()) as { token: string; userId: string };
    const r = await credentialsReq(token);
    expect(r.status).toBe(200);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j.sessionToken).toBeUndefined();
    expect(j.prefix).toBe(`users/${userId}`);
    expect(j.bucket).toBe("b1");
  });
});
