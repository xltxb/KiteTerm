import { env, exports } from "cloudflare:workers";
import { afterEach, describe, expect, it, vi } from "vitest";
import { normalizePrefix, normalizeStorage, prefixValid, publicStorageJson, usage, type StorageConfig } from "../src/storage";

const base = "https://kite.test";
const adminHeaders = { "x-admin-key": env.KITE_ADMIN_KEY, "content-type": "application/json" };

afterEach(() => {
  vi.unstubAllGlobals();
});

function getStorage() {
  return exports.default.fetch(`${base}/api/admin/storage`, { headers: adminHeaders });
}

function postStorage(body: Record<string, unknown>) {
  return exports.default.fetch(`${base}/api/admin/storage`, {
    method: "POST",
    headers: adminHeaders,
    body: JSON.stringify(body),
  });
}

function stats() {
  return exports.default.fetch(`${base}/api/admin/stats`, { headers: adminHeaders });
}

describe("normalizePrefix", () => {
  it("空 → 空;去掉首尾多余的 /;单个 prod → prod/", () => {
    expect(normalizePrefix("")).toBe("");
    expect(normalizePrefix("prod")).toBe("prod/");
    expect(normalizePrefix("/prod//")).toBe("prod/");
    expect(normalizePrefix("prod//")).toBe("prod/");
  });
});

describe("prefixValid", () => {
  it("../evil、a\\b 非法;空、prod/v2 合法", () => {
    expect(prefixValid("")).toBe(true);
    expect(prefixValid("prod/v2")).toBe(true);
    expect(prefixValid("../evil")).toBe(false);
    expect(prefixValid("a\\b")).toBe(false);
  });
});

describe("normalizeStorage", () => {
  it("endpoint 去尾斜杠;stsMode 非法值回退 auto", () => {
    const raw: StorageConfig = {
      endpoint: "http://x/",
      region: "r",
      bucket: "b",
      prefix: "p",
      accessKey: "a",
      secretKey: "s",
      pathStyle: true,
      stsMode: "weird",
    };
    const cfg = normalizeStorage(raw);
    expect(cfg.endpoint).toBe("http://x");
    expect(cfg.prefix).toBe("p/");
    expect(cfg.stsMode).toBe("auto");
  });
});

describe("publicStorageJson", () => {
  it("不包含 secretKey,只回传 hasSecret", () => {
    const cfg: StorageConfig = {
      endpoint: "http://x",
      region: "r",
      bucket: "b",
      prefix: "p/",
      accessKey: "a",
      secretKey: "s",
      pathStyle: true,
      stsMode: "auto",
    };
    const json = publicStorageJson(cfg, "db") as Record<string, unknown>;
    expect(json.hasSecret).toBe(true);
    expect(json.secretKey).toBeUndefined();
    expect(json.configured).toBe(true);
    expect(json.source).toBe("db");
  });
});

describe("管理接口:GET/POST /api/admin/storage", () => {
  it("未落库时 GET 返回 source:env", async () => {
    const r = await getStorage();
    expect(r.status).toBe(200);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j.source).toBe("env");
    expect(j.hasSecret).toBe(false);
  });

  it("非 http(s) 开头的 endpoint → 400 INVALID_ENDPOINT", async () => {
    const r = await postStorage({
      endpoint: "ftp://bad.example.com",
      bucket: "b",
      accessKey: "ak",
      secretKey: "sk",
    });
    expect(r.status).toBe(400);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "INVALID_ENDPOINT", message: "Endpoint 须以 http:// 或 https:// 开头" } });
  });

  it("非法前缀 ../evil → 400 INVALID_STORAGE", async () => {
    const r = await postStorage({
      endpoint: "https://s3.example.com",
      bucket: "b",
      accessKey: "ak",
      secretKey: "sk",
      prefix: "../evil",
    });
    expect(r.status).toBe(400);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "INVALID_STORAGE" } });
  });

  it("bucket/accessKey/secret 有空值 → 400 INVALID_STORAGE", async () => {
    const r = await postStorage({ endpoint: "https://s3.example.com", bucket: "", accessKey: "ak", secretKey: "sk" });
    expect(r.status).toBe(400);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "INVALID_STORAGE", message: "Bucket、Access Key、Secret 不能为空" } });
  });

  it("保存成功后 GET 回显 source:db,再次保存时空 secret 沿用旧值,且 /api/admin/stats 的 bucket 立刻变化", async () => {
    const saved = await postStorage({
      endpoint: "https://s3.example.com/",
      bucket: "kite-bucket-1",
      accessKey: "AK1",
      secretKey: "SECRET1",
      prefix: "prod",
    });
    expect(saved.status).toBe(200);
    const savedJson = (await saved.json()) as Record<string, unknown>;
    expect(savedJson).toMatchObject({
      endpoint: "https://s3.example.com",
      bucket: "kite-bucket-1",
      prefix: "prod/",
      hasSecret: true,
      configured: true,
      source: "db",
    });
    expect(savedJson.secretKey).toBeUndefined();

    const afterStats = await stats();
    const statsJson = (await afterStats.json()) as Record<string, unknown>;
    expect(statsJson.bucket).toBe("kite-bucket-1");
    expect(statsJson.storageConfigured).toBe(true);

    // 空 secret 沿用已保存的值:再次保存改 bucket,不传 secretKey
    const again = await postStorage({
      endpoint: "https://s3.example.com",
      bucket: "kite-bucket-2",
      accessKey: "AK1",
      secretKey: "",
      prefix: "prod",
    });
    expect(again.status).toBe(200);
    const againJson = (await again.json()) as Record<string, unknown>;
    expect(againJson).toMatchObject({ bucket: "kite-bucket-2", hasSecret: true });

    const afterStats2 = await stats();
    const statsJson2 = (await afterStats2.json()) as Record<string, unknown>;
    expect(statsJson2.bucket).toBe("kite-bucket-2");
  });
});

describe("usage —— 分页累加", () => {
  it("多页 NextContinuationToken 按 continuation-token 请求下一页,累加 objects/bytes", async () => {
    const cfg: StorageConfig = {
      endpoint: "https://s3.usage-unit.local",
      region: "us-east-1",
      bucket: "b",
      prefix: "",
      accessKey: "AK",
      secretKey: "SK",
      pathStyle: true,
      stsMode: "off",
    };
    const page1 =
      "<ListBucketResult><IsTruncated>true</IsTruncated><NextContinuationToken>tok-2</NextContinuationToken>" +
      "<Contents><Key>a</Key><Size>10</Size></Contents><Contents><Key>b</Key><Size>20</Size></Contents></ListBucketResult>";
    const page2 = "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>c</Key><Size>5</Size></Contents></ListBucketResult>";
    const urls: string[] = [];
    vi.stubGlobal("fetch", async (input: RequestInfo | URL) => {
      const url = input instanceof Request ? input.url : input.toString();
      urls.push(url);
      return new Response(url.includes("continuation-token=tok-2") ? page2 : page1, { status: 200 });
    });
    const result = await usage(cfg, "user-x");
    expect(result).toEqual({ objects: 3, bytes: 35 });
    expect(urls).toHaveLength(2);
    expect(urls[1]).toContain("continuation-token=tok-2");
  });
});

describe("GET /api/admin/users/:email/usage", () => {
  it("未知邮箱 → 404 not_found", async () => {
    const r = await exports.default.fetch(`${base}/api/admin/users/nobody-usage@example.com/usage`, { headers: adminHeaders });
    expect(r.status).toBe(404);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "not_found", message: "用户不存在" } });
  });

  it("S3 请求失败 → 502 S3_ERROR", async () => {
    const reg = await exports.default.fetch(`${base}/api/register`, {
      method: "POST",
      headers: { "content-type": "application/json", "cf-connecting-ip": "10.50.0.1" },
      body: JSON.stringify({ email: "usage-fail@example.com", password: "Kite#2026" }),
    });
    expect(reg.status).toBe(201);
    const saved = await postStorage({
      endpoint: "https://s3.usage-fail.local",
      bucket: "b-usage-fail",
      accessKey: "ak",
      secretKey: "sk",
      stsMode: "off",
    });
    expect(saved.status).toBe(200);

    vi.stubGlobal("fetch", async () => new Response("forbidden", { status: 403 }));
    const r = await exports.default.fetch(`${base}/api/admin/users/usage-fail@example.com/usage`, { headers: adminHeaders });
    expect(r.status).toBe(502);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ error: { code: "S3_ERROR" } });
  });
});

describe("POST /api/admin/storage/test", () => {
  it("用请求体里的值探测,不落库", async () => {
    const beforeGet = await getStorage();
    const beforeJson = (await beforeGet.json()) as Record<string, unknown>;

    vi.stubGlobal("fetch", async (input: RequestInfo | URL, init?: RequestInit) => {
      const method = input instanceof Request ? input.method : (init?.method ?? "GET");
      if (method === "GET") return new Response("<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>", { status: 200 });
      if (method === "PUT") return new Response("", { status: 200 });
      if (method === "DELETE") return new Response(null, { status: 204 });
      return new Response("unexpected", { status: 500 });
    });

    const r = await exports.default.fetch(`${base}/api/admin/storage/test`, {
      method: "POST",
      headers: adminHeaders,
      body: JSON.stringify({
        endpoint: "https://s3.probe-test.local",
        bucket: "probe-bucket",
        accessKey: "pk",
        secretKey: "ps",
        stsMode: "off",
      }),
    });
    expect(r.status).toBe(200);
    const j = (await r.json()) as Record<string, unknown>;
    expect(j).toMatchObject({ ok: true });
    expect(j.list).toBeUndefined();
    expect(j.put).toBeUndefined();
    expect(j.delete).toBeUndefined();

    // 未落库:GET /api/admin/storage 的值不受影响
    const afterGet = await getStorage();
    const afterJson = (await afterGet.json()) as Record<string, unknown>;
    expect(afterJson).toEqual(beforeJson);
  });
});
