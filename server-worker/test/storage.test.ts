import { env, exports } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { normalizePrefix, normalizeStorage, prefixValid, publicStorageJson, type StorageConfig } from "../src/storage";

const base = "https://kite.test";
const adminHeaders = { "x-admin-key": env.KITE_ADMIN_KEY, "content-type": "application/json" };

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
