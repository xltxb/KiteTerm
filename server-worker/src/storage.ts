// S3 存储配置:运行期可改,DB 优先、环境变量兜底(仅作初值)。
// 密钥明文落 D1(与此前明文放环境变量的强度一致),与 `server/src/storage.rs` 逐一对应。
import { AwsClient } from "aws4fetch";
import type { Env } from "./env";
import { settingGet, settingSet } from "./settings";
import { tryProbeCredentials } from "./creds";

export const STORAGE_KEY = "storage_config";

export interface StorageConfig {
  endpoint: string;
  region: string;
  bucket: string;
  /** 空或形如 `prod/`(规范化后必以 `/` 结尾) */
  prefix: string;
  accessKey: string;
  secretKey: string;
  /** MinIO / 自建网关需开启;AWS S3 / R2 关闭 */
  pathStyle: boolean;
  /** auto | require | off */
  stsMode: string;
}

const VALID_STS_MODES = new Set(["auto", "require", "off"]);

/** `prod` / `/prod//` / `prod//` → `prod/`;空 → 空 */
export function normalizePrefix(raw: string): string {
  const trimmed = raw.trim().replace(/^\/+/, "").replace(/\/+$/, "");
  return trimmed === "" ? "" : `${trimmed}/`;
}

/** 归一化后的前缀是否只含安全字符:字母、数字、`.` `_` `-` `/`,且不含 `..` 或 `\`。空前缀视为合法。 */
export function prefixValid(p: string): boolean {
  if (p === "") return true;
  if (p.includes("..") || p.includes("\\")) return false;
  return /^[A-Za-z0-9._/-]*$/.test(p);
}

function fromEnv(env: Env): StorageConfig {
  const pathStyleRaw = (env.S3_PATH_STYLE ?? "").toLowerCase();
  return {
    endpoint: env.S3_ENDPOINT ?? "",
    region: env.S3_REGION || "us-east-1",
    bucket: env.S3_BUCKET || "vela-sync",
    prefix: normalizePrefix(env.S3_PREFIX ?? ""),
    accessKey: env.S3_ACCESS_KEY ?? "",
    secretKey: env.S3_SECRET_KEY ?? "",
    pathStyle: !["off", "0", "false"].includes(pathStyleRaw),
    stsMode: env.KITE_STS || "auto",
  };
}

/** 与 Rust `StorageConfig::normalized` 一致:trim + 前缀规范化 + stsMode 非法值回退 auto */
export function normalizeStorage(cfg: StorageConfig): StorageConfig {
  const stsMode = VALID_STS_MODES.has(cfg.stsMode) ? cfg.stsMode : "auto";
  return {
    ...cfg,
    endpoint: cfg.endpoint.trim().replace(/\/+$/, ""),
    region: cfg.region.trim(),
    bucket: cfg.bucket.trim(),
    prefix: normalizePrefix(cfg.prefix),
    accessKey: cfg.accessKey.trim(),
    stsMode,
  };
}

export function isStorageConfigured(cfg: StorageConfig): boolean {
  return cfg.endpoint !== "" && cfg.bucket !== "" && cfg.accessKey !== "";
}

export function userPrefix(cfg: StorageConfig, userId: string): string {
  return `${cfg.prefix}users/${userId}`;
}

/** 对外形态:secret 只回传"是否已设置" */
export function publicStorageJson(cfg: StorageConfig, source: "db" | "env") {
  return {
    endpoint: cfg.endpoint,
    region: cfg.region,
    bucket: cfg.bucket,
    prefix: cfg.prefix,
    accessKey: cfg.accessKey,
    hasSecret: cfg.secretKey !== "",
    pathStyle: cfg.pathStyle,
    stsMode: cfg.stsMode,
    configured: isStorageConfigured(cfg),
    source,
  };
}

export interface LoadedStorage {
  cfg: StorageConfig;
  source: "db" | "env";
}

/** DB 有就用 DB(source=db),否则环境变量(source=env) */
export async function loadStorage(db: D1Database, env: Env): Promise<LoadedStorage> {
  const fromDb = await settingGet(db, STORAGE_KEY, "");
  if (fromDb !== "") {
    try {
      const parsed = JSON.parse(fromDb) as Partial<Record<keyof StorageConfig, unknown>>;
      const cfg = normalizeStorage({
        endpoint: String(parsed.endpoint ?? ""),
        region: String(parsed.region ?? ""),
        bucket: String(parsed.bucket ?? ""),
        prefix: String(parsed.prefix ?? ""),
        accessKey: String(parsed.accessKey ?? ""),
        secretKey: String(parsed.secretKey ?? ""),
        pathStyle: parsed.pathStyle !== false,
        stsMode: String(parsed.stsMode ?? "auto"),
      });
      return { cfg, source: "db" };
    } catch {
      // 落库内容损坏时忽略,回退环境变量,与 Rust `serde_json::from_str` 失败分支一致
    }
  }
  return { cfg: normalizeStorage(fromEnv(env)), source: "env" };
}

export async function saveStorage(db: D1Database, cfg: StorageConfig): Promise<void> {
  await settingSet(db, STORAGE_KEY, JSON.stringify(cfg));
}

// ---- S3 请求辅助 ----

export function s3Client(cfg: StorageConfig): AwsClient {
  return new AwsClient({ accessKeyId: cfg.accessKey, secretAccessKey: cfg.secretKey, service: "s3", region: cfg.region });
}

/** bucket 的根地址:pathStyle → 拼在路径里;否则把 bucket 作为子域插到 endpoint 主机前 */
function bucketOrigin(cfg: StorageConfig): string {
  if (cfg.pathStyle) return `${cfg.endpoint}/${cfg.bucket}`;
  const u = new URL(cfg.endpoint);
  u.host = `${cfg.bucket}.${u.host}`;
  return u.origin;
}

export function objectUrl(cfg: StorageConfig, key: string): string {
  const encoded = key.split("/").map(encodeURIComponent).join("/");
  return `${bucketOrigin(cfg)}/${encoded}`;
}

export function listUrl(cfg: StorageConfig, prefix: string, token?: string, opts?: { maxKeys?: number; delimiter?: string }): string {
  const params = new URLSearchParams({ "list-type": "2", prefix, "max-keys": String(opts?.maxKeys ?? 1000) });
  if (opts?.delimiter) params.set("delimiter", opts.delimiter);
  if (token) params.set("continuation-token", token);
  return `${bucketOrigin(cfg)}?${params.toString()}`;
}

function shortMessage(status: number, body: string): string {
  return `HTTP ${status}: ${body.slice(0, 300)}`;
}

function errMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export interface ProbeReport {
  ok: boolean;
  latencyMs: number;
  list?: string;
  put?: string;
  delete?: string;
  sts?: string;
}

/**
 * 探测:ListBucket → Put 探针 → Delete 探针;stsMode != "off" 时再试一次临时凭据换取。
 * 各项省略表示通过,取值为失败原因。语义同 Rust `storage::probe`(TS 里没有单独的
 * `bucket()` 构造步骤,故略去其失败分支)。
 */
export async function probe(cfg: StorageConfig, env: Env): Promise<ProbeReport> {
  const t0 = Date.now();
  const report: ProbeReport = { ok: false, latencyMs: 0 };
  const client = s3Client(cfg);

  try {
    const url = listUrl(cfg, cfg.prefix, undefined, { maxKeys: 1, delimiter: "/" });
    const res = await client.fetch(url, { method: "GET" });
    if (!res.ok) report.list = shortMessage(res.status, await res.text().catch(() => ""));
  } catch (e) {
    report.list = errMessage(e);
  }

  const probeKey = `${cfg.prefix}.kite-probe`;
  try {
    const putRes = await client.fetch(objectUrl(cfg, probeKey), { method: "PUT", body: "probe" });
    if (putRes.ok) {
      try {
        const delRes = await client.fetch(objectUrl(cfg, probeKey), { method: "DELETE" });
        if (!delRes.ok) report.delete = shortMessage(delRes.status, await delRes.text().catch(() => ""));
      } catch (e) {
        report.delete = errMessage(e);
      }
    } else {
      report.put = shortMessage(putRes.status, await putRes.text().catch(() => ""));
    }
  } catch (e) {
    report.put = errMessage(e);
  }

  if (cfg.stsMode !== "off") {
    // 这里只是"试一次能否换到临时凭据",不套用 issueCredentials 的 require/auto 兜底语义
    try {
      await tryProbeCredentials(cfg, env);
    } catch (e) {
      report.sts = errMessage(e);
    }
  }

  report.latencyMs = Date.now() - t0;
  report.ok =
    report.list === undefined &&
    report.put === undefined &&
    report.delete === undefined &&
    (cfg.stsMode !== "require" || report.sts === undefined);
  return report;
}

export interface UsageResult {
  objects: number;
  bytes: number;
}

/** 该用户前缀下的对象数与字节数;分页累加 `<Contents>` 里的 `<Size>` */
export async function usage(cfg: StorageConfig, userId: string): Promise<UsageResult> {
  const client = s3Client(cfg);
  const prefix = `${userPrefix(cfg, userId)}/`;
  let token: string | undefined;
  let objects = 0;
  let bytes = 0;
  do {
    const res = await client.fetch(listUrl(cfg, prefix, token), { method: "GET" });
    const text = await res.text();
    if (!res.ok) throw new Error(shortMessage(res.status, text));
    for (const m of text.matchAll(/<Size>(\d+)<\/Size>/g)) {
      objects += 1;
      bytes += Number(m[1]);
    }
    const truncated = /<IsTruncated>true<\/IsTruncated>/.test(text);
    const next = text.match(/<NextContinuationToken>(.*?)<\//);
    token = truncated ? next?.[1] : undefined;
  } while (token);
  return { objects, bytes };
}
