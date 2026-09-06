// 云同步凭据下发:优先 R2 临时凭据 / 通用 STS AssumeRole 换取按用户前缀收敛的限权凭据,
// 换取失败时按 stsMode 决定报错(require)或回退共享凭据(auto)。
// 与 `server/src/sts.rs`、`server/src/main.rs::sync_credentials` 逐一对应。
import { AwsClient } from "aws4fetch";
import type { Env } from "./env";
import type { StorageConfig } from "./storage";
import { userPrefix } from "./storage";

export interface Creds {
  endpoint: string;
  bucket: string;
  region: string;
  accessKey: string;
  secretKey: string;
  sessionToken?: string;
  expiration?: string;
  prefix: string;
  pathStyle: boolean;
}

/** stsMode=require 时换取失败,路由据此映射为 502 sts_error */
export class StsError extends Error {}

/** endpoint host 是否为 Cloudflare R2 网关 */
export function isR2Endpoint(endpoint: string): boolean {
  try {
    return new URL(endpoint).host.endsWith(".r2.cloudflarestorage.com");
  } catch {
    return false;
  }
}

/**
 * 从 R2 endpoint 解析账号 ID:`<accountId>.r2.cloudflarestorage.com` 或带 jurisdiction 的
 * `<accountId>.eu.r2.cloudflarestorage.com`,取 host 的首段子域。非 R2 端点返回 undefined。
 */
export function r2AccountId(endpoint: string): string | undefined {
  if (!isR2Endpoint(endpoint)) return undefined;
  const host = new URL(endpoint).host;
  const first = host.split(".")[0];
  return first && first !== "r2" ? first : undefined;
}

function sharedCreds(cfg: StorageConfig, userId: string): Creds {
  return {
    endpoint: cfg.endpoint,
    bucket: cfg.bucket,
    region: cfg.region,
    accessKey: cfg.accessKey,
    secretKey: cfg.secretKey,
    prefix: userPrefix(cfg, userId),
    pathStyle: cfg.pathStyle,
  };
}

/**
 * R2 临时凭据 API:parentAccessKeyId 收敛到指定用户前缀,固定 1 小时(`ttlSeconds: 3600`)有效。
 * 与简报要求的请求体字段一致,不接收 `durationSecs`(`rawTempCredentials` 里探测用的 900s
 * 只对通用 STS 分支生效,对 R2 分支没有意义 —— R2 侧统一走 Cloudflare 这份临时凭据接口
 * 固定的 1 小时,不做 900s/3600s 的区分)。
 */
async function r2TempCredentials(cfg: StorageConfig, env: Env, accountId: string, userId: string): Promise<Creds> {
  const prefix = userPrefix(cfg, userId);
  const res = await fetch(`https://api.cloudflare.com/client/v4/accounts/${accountId}/r2/temp-access-credentials`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${env.R2_API_TOKEN}`,
      "content-type": "application/json",
    },
    body: JSON.stringify({
      bucket: cfg.bucket,
      parentAccessKeyId: cfg.accessKey,
      permission: "object-read-write",
      ttlSeconds: 3600,
      prefixes: [`${prefix}/`],
    }),
  });
  const body = (await res.json().catch(() => null)) as {
    success?: boolean;
    result?: { accessKeyId: string; secretAccessKey: string; sessionToken: string };
    errors?: Array<{ message: string }>;
  } | null;
  if (!body || body.success !== true || !body.result) {
    const msg = body?.errors?.[0]?.message ?? `R2 临时凭据接口 HTTP ${res.status}`;
    throw new Error(msg);
  }
  return {
    endpoint: cfg.endpoint,
    bucket: cfg.bucket,
    region: cfg.region,
    accessKey: body.result.accessKeyId,
    secretKey: body.result.secretAccessKey,
    sessionToken: body.result.sessionToken,
    expiration: new Date(Date.now() + 3600_000).toISOString(),
    prefix,
    pathStyle: cfg.pathStyle,
  };
}

/** RFC3986 百分号编码(表单值用),与 `sts.rs::urlencode` 一致:只留 A-Za-z0-9-_.~ */
function urlEncode(s: string): string {
  const bytes = new TextEncoder().encode(s);
  let out = "";
  for (const b of bytes) {
    const ch = String.fromCharCode(b);
    if (/[A-Za-z0-9\-_.~]/.test(ch)) {
      out += ch;
    } else {
      out += `%${b.toString(16).toUpperCase().padStart(2, "0")}`;
    }
  }
  return out;
}

function sessionPolicy(bucket: string, prefix: string, userId: string): string {
  return JSON.stringify({
    Version: "2012-10-17",
    Statement: [
      {
        Effect: "Allow",
        Action: ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"],
        Resource: [`arn:aws:s3:::${bucket}/${prefix}users/${userId}/*`],
      },
      {
        Effect: "Allow",
        Action: ["s3:ListBucket"],
        Resource: [`arn:aws:s3:::${bucket}`],
        Condition: { StringLike: { "s3:prefix": [`${prefix}users/${userId}/*`] } },
      },
    ],
  });
}

function extractTag(xml: string, tag: string): string | undefined {
  const m = xml.match(new RegExp(`<${tag}>([\\s\\S]*?)</${tag}>`));
  return m?.[1];
}

/** 通用 STS AssumeRole(SigV4,aws4fetch 代签):移植自 `server/src/sts.rs` */
async function stsAssumeRole(cfg: StorageConfig, userId: string, durationSecs: number): Promise<Creds> {
  const policy = sessionPolicy(cfg.bucket, cfg.prefix, userId);
  const body =
    `Action=AssumeRole&Version=2011-06-15&DurationSeconds=${durationSecs}` +
    `&RoleArn=${urlEncode("arn:aws:iam::000000000000:role/vela-sync")}` +
    `&RoleSessionName=${urlEncode(`kite-${userId}`)}` +
    `&Policy=${urlEncode(policy)}`;
  const client = new AwsClient({ accessKeyId: cfg.accessKey, secretAccessKey: cfg.secretKey, service: "sts", region: cfg.region });
  const res = await client.fetch(`${cfg.endpoint}/`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body,
  });
  const text = await res.text();
  if (!res.ok) {
    throw new Error(`STS HTTP ${res.status}: ${text.slice(0, 300)}`);
  }
  const accessKey = extractTag(text, "AccessKeyId");
  const secretKey = extractTag(text, "SecretAccessKey");
  const sessionToken = extractTag(text, "SessionToken");
  if (!accessKey) throw new Error("STS 响应缺少 AccessKeyId");
  if (!secretKey) throw new Error("STS 响应缺少 SecretAccessKey");
  if (!sessionToken) throw new Error("STS 响应缺少 SessionToken");
  return {
    endpoint: cfg.endpoint,
    bucket: cfg.bucket,
    region: cfg.region,
    accessKey,
    secretKey,
    sessionToken,
    expiration: extractTag(text, "Expiration") ?? "",
    prefix: userPrefix(cfg, userId),
    pathStyle: cfg.pathStyle,
  };
}

/**
 * 只负责"用哪种机制换临时凭据"(R2 或通用 STS),不做 stsMode 的 require/auto 兜底语义。
 * 供 `issueCredentials` 与 `probe()` 复用。`durationSecs` 只传给通用 STS 分支
 * (`AssumeRole` 的 `DurationSeconds`);R2 分支固定 1 小时,见 `r2TempCredentials` 的注释。
 */
async function rawTempCredentials(cfg: StorageConfig, env: Env, userId: string, durationSecs: number): Promise<Creds> {
  // R2_ACCOUNT_ID 未显式设置时,从 endpoint 的子域解析(账号 ID 本就写在 R2 域名里)
  const accountId = env.R2_ACCOUNT_ID || r2AccountId(cfg.endpoint);
  if (accountId && isR2Endpoint(cfg.endpoint) && env.R2_API_TOKEN) {
    return r2TempCredentials(cfg, env, accountId, userId);
  }
  return stsAssumeRole(cfg, userId, durationSecs);
}

/**
 * 下发同步凭据:stsMode=off 直接共享;否则尝试限权临时凭据 —— require 失败即抛 `StsError`,
 * auto 失败则打印告警并回退共享凭据。
 */
export async function issueCredentials(cfg: StorageConfig, env: Env, userId: string): Promise<Creds> {
  if (cfg.stsMode === "off") {
    return sharedCreds(cfg, userId);
  }
  try {
    return await rawTempCredentials(cfg, env, userId, 3600);
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    if (cfg.stsMode === "require") {
      throw new StsError(message);
    }
    console.warn(`[kite] STS 失败,回退共享凭据: ${message}`);
    return sharedCreds(cfg, userId);
  }
}

/** 供 `probe()` 复用:直接试一次临时凭据换取,不套用 require/auto 的兜底/报错包装 */
export async function tryProbeCredentials(cfg: StorageConfig, env: Env, userId = "probe", durationSecs = 900): Promise<Creds> {
  return rawTempCredentials(cfg, env, userId, durationSecs);
}
