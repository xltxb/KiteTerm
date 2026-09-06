# 管理平台 Cloudflare Workers 版(server-worker)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `server-worker/` 用 TypeScript(Hono)+ D1 + KV + R2 实现与 `server/`(Rust)**完全相同的 HTTP 契约**,可 `wrangler deploy` 到 Cloudflare Workers;桌面端与管理控制台零改动。

**Architecture:** Hono 应用导出 `fetch` 与 `scheduled`;所有状态在 D1(六张表,与 Rust 版同名同列)与 KV(限流计数);口令 PBKDF2-SHA256、TOTP HMAC-SHA1、令牌随机 hex 均用 WebCrypto;S3 操作用 `aws4fetch` 签名;凭据下发在 R2 上走「临时访问凭据 API」,其它存储走 SigV4 `AssumeRole`,失败按 `stsMode` 回退共享凭据;控制台 `admin.html` 作为静态资源由 Workers Assets 直接返回。

**Tech Stack:** TypeScript 5、hono 4.13、aws4fetch 1.0、wrangler 4.129、vitest 4.1 + @cloudflare/vitest-plugin 1.1、@cloudflare/workers-types 5;pnpm。

**Spec:** `docs/deploy/cloudflare-workers.md`(方案)+ `server/src/main.rs` 等 Rust 实现(**契约权威**:路径、请求体、响应体、错误码、检查顺序以 Rust 版为准,实现者遇到歧义直接读 Rust 源)。

## Global Constraints

- 契约逐字对齐 Rust 版:错误形态 `{ "error": { "code", "message" } }`,状态码与错误码字面值相同(含既有小写码如 `not_found`、`bad_credentials`、`unauthorized`、`invalid_email`、`weak_password`、`email_taken`、`registration_closed`、`invite_required`、`invite_invalid`、`invite_expired`、`invite_exhausted`、`rate_limited`、`invalid_mode`、`internal`,以及新码 `UNKNOWN_FEATURE`、`LOGIN_DISABLED`、`SYNC_DISABLED`、`STORAGE_UNCONFIGURED`、`EMAIL_UNVERIFIED`、`CODE_INVALID`、`CODE_EXPIRED`、`CODE_RATE_LIMITED`、`RESEND_TOO_SOON`、`MAIL_ERROR`、`TOTP_REQUIRED`、`TOTP_INVALID`、`TOTP_RATE_LIMITED`、`TOTP_SETUP_REQUIRED`、`TOTP_ENFORCED`、`TOTP_NOT_PENDING`、`TOTP_ALREADY_ENABLED`、`INVALID_ENDPOINT`、`INVALID_STORAGE`、`S3_ERROR`、`sts_error`)。
- JSON 键 camelCase 与 Rust 版一致;时间戳为 Unix 秒整数。
- 不改 `server/`、`desktop/`;`server-worker/public/admin.html` 由脚本从 `server/src/admin.html` 复制,不手改。
- 命名一律 KiteTerm;报告与注释用中文。
- 每个任务:`pnpm test` 与 `pnpm exec tsc --noEmit` 全绿再提交;提交信息末尾带 `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` 与 `Claude-Session: https://claude.ai/code/session_01Eh46U9sXNjd7Lvrhys385C`。
- 本机可用:node 22、pnpm、网络;无 Cloudflare 账号登录,所以 **不做 `wrangler deploy`**,只做 `wrangler dev --local` 冒烟(Task 5)。

---

## 文件结构

```
server-worker/
  package.json  tsconfig.json  wrangler.toml  vitest.config.ts  .gitignore
  migrations/0001_init.sql
  scripts/sync-admin.sh          把 ../server/src/admin.html 复制到 public/admin.html + public/index.html
  public/                        (生成物,gitignore;CI 与 dev 前先跑 scripts/sync-admin.sh)
  src/env.ts        Env 绑定类型
  src/http.ts       apiError / requireAdmin / bearerToken / clientIp / nowTs
  src/settings.ts   settingGet / settingSet(platform_settings)
  src/features.ts   NAMES 注册表、raw / effective / set / payload / adminPayload
  src/crypto.ts     hashPassword / verifyPassword(PBKDF2)、randomHex、base32、totp*
  src/ratelimit.ts  rateLimitOk(kv, bucket, key, max)
  src/auth.ts       issueToken / authenticate / passwordIssue / validEmail / newInviteCode
  src/storage.ts    StorageConfig 读写、normalizePrefix / prefixValid、S3 客户端(aws4fetch)、probe、usage
  src/creds.ts      r2TempCredentials / stsAssumeRole / issueCredentials
  src/mail.ts       sendCode(Resend 或日志)
  src/routes/public.ts   /api/health /api/features /api/register/policy
  src/routes/account.ts  register/login/logout/me/verify-email/2fa/sync-credentials
  src/routes/admin.ts    /api/admin/*
  src/index.ts      组装 Hono app,导出 { fetch, scheduled }
  test/apply-migrations.ts  test/env.d.ts  test/*.test.ts
```

---

### Task 1: 骨架、D1 迁移、功能开关注册表、管理设置/邀请码/用户列表、静态控制台、测试基建

**Files:** 上表中除 `crypto.ts / ratelimit.ts / auth.ts / storage.ts / creds.ts / mail.ts / routes/account.ts` 外的全部;`test/features.test.ts`、`test/admin-settings.test.ts`。

**Interfaces:**
- Produces:`Env`;`apiError(c, status, code, message)`;`requireAdmin(c)`(返回 `Response | null`);`settingGet(db, key, def)` / `settingSet(db, key, value)`;`features.NAMES / raw / effective / set / payload / adminPayload`;`app`(Hono<{ Bindings: Env }>)。

- [ ] **Step 1: 项目文件**

`server-worker/package.json`:
```json
{
  "name": "kite-server-worker",
  "private": true,
  "type": "module",
  "scripts": {
    "sync-admin": "sh scripts/sync-admin.sh",
    "dev": "pnpm sync-admin && wrangler dev --local",
    "deploy": "pnpm sync-admin && wrangler deploy",
    "test": "pnpm sync-admin && vitest run",
    "typecheck": "tsc --noEmit",
    "migrate:remote": "wrangler d1 migrations apply kite-server --remote"
  },
  "dependencies": { "aws4fetch": "^1.0.20", "hono": "^4.13.7" },
  "devDependencies": {
    "@cloudflare/vitest-plugin": "^1.1.4",
    "@cloudflare/workers-types": "^5.20260906.1",
    "typescript": "^5.6.3",
    "vitest": "^4.1.0",
    "wrangler": "^4.129.0"
  }
}
```

`server-worker/tsconfig.json`:
```json
{
  "compilerOptions": {
    "target": "ES2022", "module": "ESNext", "moduleResolution": "Bundler", "lib": ["ES2022"],
    "types": ["@cloudflare/workers-types"], "strict": true, "noEmit": true, "skipLibCheck": true,
    "resolveJsonModule": true, "isolatedModules": true
  },
  "include": ["src", "test", "vitest.config.ts"]
}
```

`server-worker/wrangler.toml`:
```toml
name = "kite-server"
main = "src/index.ts"
compatibility_date = "2026-09-01"
compatibility_flags = ["nodejs_compat"]

[assets]
directory = "./public"

[[d1_databases]]
binding = "DB"
database_name = "kite-server"
database_id = "00000000-0000-0000-0000-000000000000"   # wrangler d1 create kite-server 后替换

[[kv_namespaces]]
binding = "RATE"
id = "00000000000000000000000000000000"                 # wrangler kv namespace create RATE 后替换

[triggers]
crons = ["0 4 * * *"]

[vars]
KITE_REGISTRATION = "open"
KITE_FEATURE_GATING = "off"
KITE_REGISTER_LIMIT = "5"
S3_ENDPOINT = ""
S3_BUCKET = "kite-sync"
S3_REGION = "auto"
S3_PREFIX = ""
S3_PATH_STYLE = "on"
KITE_STS = "auto"
R2_ACCOUNT_ID = ""
KITE_SMTP_FROM = ""
# secrets(wrangler secret put):KITE_ADMIN_KEY S3_ACCESS_KEY S3_SECRET_KEY R2_API_TOKEN RESEND_API_KEY
```

`server-worker/.gitignore`:`node_modules/`、`public/`、`.wrangler/`、`dist/`。

`server-worker/scripts/sync-admin.sh`:
```sh
#!/bin/sh
# 控制台单一来源在 server/src/admin.html;这里只复制,不手改
set -e
cd "$(dirname "$0")/.."
mkdir -p public
cp ../server/src/admin.html public/admin.html
cp ../server/src/admin.html public/index.html
```

`server-worker/migrations/0001_init.sql`:与 `docs/deploy/cloudflare-workers.md` §6 完全相同(users/tokens+索引/platform_settings/invites/invite_uses/email_codes)。

`server-worker/vitest.config.ts`:
```ts
import path from "node:path";
import { cloudflareTest, readD1Migrations } from "@cloudflare/vitest-plugin";
import { defineConfig } from "vitest/config";

export default defineConfig(async () => {
  const migrations = await readD1Migrations(path.join(__dirname, "migrations"));
  return {
    plugins: [
      cloudflareTest({
        wrangler: { configPath: "./wrangler.toml" },
        miniflare: {
          bindings: {
            TEST_MIGRATIONS: migrations,
            KITE_ADMIN_KEY: "test-admin-key",
            KITE_REGISTER_LIMIT: "3",
          },
        },
      }),
    ],
    test: { setupFiles: ["./test/apply-migrations.ts"] },
  };
});
```

`server-worker/test/apply-migrations.ts`:
```ts
import { applyD1Migrations } from "cloudflare:test";
import { env } from "cloudflare:workers";
await applyD1Migrations(env.DB, env.TEST_MIGRATIONS);
```

`server-worker/test/env.d.ts`:
```ts
import type { Env } from "../src/env";
declare module "cloudflare:workers" {
  interface Env extends import("../src/env").Env { TEST_MIGRATIONS: D1Migration[] }
}
```
(若 `@cloudflare/vitest-plugin` 的类型要求用 `cloudflare:test` 的 `ProvidedEnv` 声明,按其 README 调整,报告中说明。)

- [ ] **Step 2: 基础模块**

`src/env.ts`:
```ts
export interface Env {
  DB: D1Database;
  RATE: KVNamespace;
  KITE_ADMIN_KEY: string;
  KITE_REGISTRATION?: string;
  KITE_FEATURE_GATING?: string;
  KITE_REGISTER_LIMIT?: string;
  S3_ENDPOINT?: string; S3_BUCKET?: string; S3_REGION?: string; S3_PREFIX?: string; S3_PATH_STYLE?: string;
  S3_ACCESS_KEY?: string; S3_SECRET_KEY?: string;
  KITE_STS?: string;
  R2_ACCOUNT_ID?: string; R2_API_TOKEN?: string;
  RESEND_API_KEY?: string; KITE_SMTP_FROM?: string;
}
export const featureGating = (env: Env) => ["on", "1", "true"].includes((env.KITE_FEATURE_GATING ?? "off").toLowerCase());
export const registrationDefault = (env: Env) => (["open", "invite", "closed"].includes(env.KITE_REGISTRATION ?? "") ? env.KITE_REGISTRATION! : "open");
```

`src/http.ts`:
```ts
import type { Context } from "hono";
import type { Env } from "./env";
export type C = Context<{ Bindings: Env }>;
export const nowTs = () => Math.floor(Date.now() / 1000);
export function apiError(c: C, status: number, code: string, message: string) {
  return c.json({ error: { code, message } }, status as 400);
}
export function requireAdmin(c: C): Response | null {
  const key = c.req.header("x-admin-key") ?? "";
  if (!key || key !== c.env.KITE_ADMIN_KEY) return apiError(c, 401, "unauthorized", "管理密钥错误");
  return null;
}
export const bearerToken = (c: C) => c.req.header("authorization")?.replace(/^Bearer\s+/i, "") ?? null;
export const clientIp = (c: C) => c.req.header("cf-connecting-ip") ?? c.req.header("x-forwarded-for")?.split(",")[0]?.trim() ?? "unknown";
```

`src/settings.ts`:
```ts
export async function settingGet(db: D1Database, key: string, def: string): Promise<string> {
  const row = await db.prepare("SELECT value FROM platform_settings WHERE key = ?1").bind(key).first<{ value: string }>();
  return row?.value ?? def;
}
export async function settingSet(db: D1Database, key: string, value: string): Promise<void> {
  await db.prepare("INSERT INTO platform_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2").bind(key, value).run();
}
```

`src/features.ts`(与 `server/src/features.rs` 同表同语义):
```ts
import { settingGet, settingSet } from "./settings";
/** [名字, 默认值, 是否受门禁总闸] */
export const NAMES: ReadonlyArray<readonly [string, boolean, boolean]> = [
  ["login", true, true], ["sync", true, true], ["ai", true, true],
  ["key_sync", true, true], ["recording_sync", true, true],
  ["ai_command", true, true], ["ai_diagnose", true, true], ["ai_summary", true, true],
  ["email_verify", false, false], ["totp_required", false, false], ["multi_device", true, false],
];
const meta = (name: string) => NAMES.find(([n]) => n === name) ?? [name, true, true] as const;
export const camel = (s: string) => s.replace(/_([a-z])/g, (_, ch: string) => ch.toUpperCase());
const pascal = (s: string) => { const c = camel(s); return c[0].toUpperCase() + c.slice(1); };
export const snake = (pascalName: string) => pascalName.replace(/([A-Z])/g, (m, _c, i) => (i ? "_" : "") + m.toLowerCase());
export async function raw(db: D1Database, name: string): Promise<boolean> {
  const [, def] = meta(name);
  return (await settingGet(db, `feature_${name}`, def ? "1" : "0")) === "1";
}
export async function effective(db: D1Database, gating: boolean, name: string): Promise<boolean> {
  const [, , gated] = meta(name);
  if (gated && !gating) return true;
  return raw(db, name);
}
export const set = (db: D1Database, name: string, on: boolean) => settingSet(db, `feature_${name}`, on ? "1" : "0");
export async function payload(db: D1Database, gating: boolean): Promise<Record<string, boolean>> {
  const out: Record<string, boolean> = {};
  for (const [n] of NAMES) out[camel(n)] = await effective(db, gating, n);
  return out;
}
export async function adminPayload(db: D1Database): Promise<Record<string, boolean>> {
  const out: Record<string, boolean> = {};
  for (const [n] of NAMES) out[`feature${pascal(n)}`] = await raw(db, n);
  return out;
}
```

- [ ] **Step 3: 路由(公开 + 管理,本任务范围)**

`src/routes/public.ts`:`GET /api/health` → `{ ok: true, service: "kite-server" }`;`GET /api/features` → `payload` + `registration`(`settingGet(db,"registration_mode", registrationDefault(env))`)+ `enforced` + `passwordMinLength: 8` + `passwordRule: "至少 8 位,且包含字母、数字、符号中的至少两类"`;`GET /api/register/policy` → `{ mode, passwordMinLength, passwordRule }`。

`src/routes/admin.ts`(每个 handler 开头 `const deny = requireAdmin(c); if (deny) return deny;`):
- `GET /api/admin/settings` → `settingsPayload(db, env)` = `adminPayload` + `registrationMode` + `defaultSyncEnabled`(`default_sync_enabled` 默认 "1")+ `featureGating`。
- `POST /api/admin/settings`:body 任意子集;`registrationMode` 校验三档否则 400 `invalid_mode`「注册策略只能是 open/invite/closed」;`defaultSyncEnabled` 布尔;其余键 `feature*` → `snake(rest)` 必须在 `NAMES`,否则 400 `UNKNOWN_FEATURE`「未知功能开关 {key}」;非布尔值跳过;返回 `settingsPayload`。
- `GET /api/admin/invites`、`POST /api/admin/invites`(`note`、`maxUses`、`expiresInDays` → 生成 `newInviteCode()`:字母表 `ABCDEFGHJKMNPQRSTUVWXYZ23456789`,12 位分三段 `XXXX-XXXX-XXXX`,用 `crypto.getRandomValues`)、`POST /api/admin/invites/:code/disable`(0 行 → 404 `not_found`「邀请码不存在」)。响应字段与 Rust 版相同(`code, note, maxUses, usedCount, expiresAt, disabled, createdAt`)。
- `GET /api/admin/users` → `{ users: [{ id, email, syncEnabled, emailVerified, totpEnabled, createdAt }] }` 按 created_at 升序。
- `POST /api/admin/users/:email/sync` `{ enabled }` → 更新;0 行 404 `not_found`「用户不存在」;返回 `{ ok, email, syncEnabled }`。
- `GET /api/admin/stats` → `{ users, syncEnabled, activeTokens, bucket, endpoint, stsMode, storageConfigured }`(bucket/endpoint/stsMode/storageConfigured 本任务先从 `env` 读 —— Task 4 改为读存储配置)。

`src/index.ts`:
```ts
import { Hono } from "hono";
import type { Env } from "./env";
import { publicRoutes } from "./routes/public";
import { adminRoutes } from "./routes/admin";
const app = new Hono<{ Bindings: Env }>();
app.route("/", publicRoutes);
app.route("/", adminRoutes);
app.notFound((c) => c.json({ error: { code: "not_found", message: "接口不存在" } }, 404));
export default {
  fetch: app.fetch,
  async scheduled(_e: ScheduledEvent, env: Env) {
    await env.DB.prepare("DELETE FROM tokens WHERE expires_at < ?1").bind(Math.floor(Date.now() / 1000)).run();
  },
};
```
`/admin` 与 `/` 由 Assets 处理,Worker 不写路由。

- [ ] **Step 4: 测试**

`test/features.test.ts`:
```ts
import { env, exports } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
const base = "https://kite.test";
describe("features", () => {
  it("默认值与门禁", async () => {
    const r = await exports.default.fetch(`${base}/api/features`);
    const j = await r.json() as Record<string, unknown>;
    expect(j).toMatchObject({ login: true, keySync: true, aiSummary: true, emailVerify: false, totpRequired: false, multiDevice: true, enforced: false, registration: "open" });
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
```
`test/admin-settings.test.ts`:无密钥 401;`POST {featureAiSummary:false, featureTotpRequired:true}` 回包含两键;`{featureNope:true}` → 400 `UNKNOWN_FEATURE`;`{registrationMode:"x"}` → 400 `invalid_mode`;邀请码创建→列表→停用→再停用 404;用户列表初始为空数组。

- [ ] **Step 5: 跑通**

Run:`cd server-worker && pnpm install && pnpm test && pnpm typecheck`。Expected:全部通过。`pnpm dev` 能启动并 `curl http://127.0.0.1:8787/api/health` 返回 ok、`curl -I http://127.0.0.1:8787/admin` 200(静态资源)。

- [ ] **Step 6: Commit**

`git add server-worker && git commit -m "feat(worker): Workers 版管理平台骨架 —— D1 迁移、功能开关注册表、管理设置/邀请码/用户列表、静态控制台、vitest"`

---

### Task 2: 账户 —— PBKDF2、令牌、KV 限流、注册/登录/登出/me、多设备

**Files:** `src/crypto.ts`(pbkdf2 部分 + `randomHex` + 常量时间比较)、`src/ratelimit.ts`、`src/auth.ts`、`src/routes/account.ts`(本任务的接口)、`src/index.ts`(挂载)、`test/crypto.test.ts`、`test/account.test.ts`。

**Interfaces:**
- Produces:`hashPassword(pw) -> Promise<string>`(`pbkdf2$600000$<salt_b64>$<hash_b64>`)、`verifyPassword(pw, stored) -> Promise<boolean>`、`randomHex(bytes)`、`timingSafeEqual(a: Uint8Array, b: Uint8Array)`;`rateLimitOk(kv, bucket, key, max, windowSec = 3600) -> Promise<boolean>`;`issueToken(db, userId) -> {token, expiresAt}`(30 天);`authenticate(c) -> Promise<AuthedUser | Response>`(`AuthedUser { id, email, syncEnabled, emailVerified, totpEnabled }`;缺令牌 401 `unauthorized`「缺少令牌」,过期 401 `token_expired`「令牌已过期,请重新登录」,无效 401 `unauthorized`「令牌无效」);`validEmail`、`passwordIssue(pw) -> string | null`(规则与 Rust 相同:8–128 位、至少两类、≥4 种不同字符、弱口令表);`userPublic(row)` → `{ id, email, syncEnabled, emailVerified, totpEnabled }`。
- 登录响应 `{ token, expiresAt, totpSetupRequired, user }`;注册响应 201 `{ userId, token, expiresAt, totpSetupRequired, user }`(邮箱验证与 TOTP 的实际判定在 Task 3 补上,本任务先写 `emailVerified: true`、`totpEnabled: false`、`totpSetupRequired: raw(db,"totp_required")`)。

- [ ] **Step 1: crypto.ts(PBKDF2 部分)**
```ts
const ITER = 600_000;
const enc = new TextEncoder();
export const b64 = (u: Uint8Array) => btoa(String.fromCharCode(...u));
export const unb64 = (s: string) => Uint8Array.from(atob(s), (ch) => ch.charCodeAt(0));
export const randomHex = (n: number) => [...crypto.getRandomValues(new Uint8Array(n))].map((b) => b.toString(16).padStart(2, "0")).join("");
export function timingSafeEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  let d = 0; for (let i = 0; i < a.length; i++) d |= a[i] ^ b[i];
  return d === 0;
}
async function derive(pw: string, salt: Uint8Array, iter: number): Promise<Uint8Array> {
  const key = await crypto.subtle.importKey("raw", enc.encode(pw), "PBKDF2", false, ["deriveBits"]);
  return new Uint8Array(await crypto.subtle.deriveBits({ name: "PBKDF2", hash: "SHA-256", salt, iterations: iter }, key, 256));
}
export async function hashPassword(pw: string): Promise<string> {
  const salt = crypto.getRandomValues(new Uint8Array(16));
  return `pbkdf2$${ITER}$${b64(salt)}$${b64(await derive(pw, salt, ITER))}`;
}
export async function verifyPassword(pw: string, stored: string): Promise<boolean> {
  const [alg, iterS, saltB, hashB] = stored.split("$");
  if (alg !== "pbkdf2") return false;
  const got = await derive(pw, unb64(saltB), Number(iterS));
  return timingSafeEqual(got, unb64(hashB));
}
```

- [ ] **Step 2: ratelimit.ts**
```ts
/** KV 滑动窗口:值是窗口内时间戳数组;最终一致,允许少量超发 */
export async function rateLimitOk(kv: KVNamespace, bucket: string, key: string, max: number, windowSec = 3600): Promise<boolean> {
  const k = `rl:${bucket}:${key}`;
  const now = Math.floor(Date.now() / 1000);
  const hits = ((await kv.get<number[]>(k, "json")) ?? []).filter((t) => now - t < windowSec);
  if (hits.length >= max) return false;
  hits.push(now);
  await kv.put(k, JSON.stringify(hits), { expirationTtl: windowSec });
  return true;
}
```

- [ ] **Step 3: auth.ts + routes/account.ts**

移植 `server/src/main.rs` 的 `valid_email`、`password_issue`、`new_invite_code`、`issue_token`、`authenticate`、`register`、`login`、`logout`、`me`。要点:
- `register`:限流 `rateLimitOk(env.RATE, "register", ip, Number(env.KITE_REGISTER_LIMIT ?? 5))` 不通过 → 429 `rate_limited`「注册过于频繁,请稍后再试」;邮箱 `trim().toLowerCase()`;策略 closed → 403 `registration_closed`;invite 模式校验邀请码(四个错误码与文案照抄);`INSERT` 用 `INSERT OR IGNORE`,`meta.changes === 0` → 409 `email_taken`;`default_sync_enabled`;invite 计数与 `invite_uses` 用 `db.batch([...])`;签发令牌;201。
- `login`:查 `id, password_hash, sync_enabled, email_verified, totp_secret, totp_enabled`;不存在或口令错 → 401 `bad_credentials`「邮箱或密码错误」;再查 `effective("login")` → 403 `LOGIN_DISABLED`「平台当前已关闭登录」;(TOTP 分支在 Task 3 插入此处);签发令牌;`multi_device` 为 false 时 `DELETE FROM tokens WHERE user_id = ?1 AND token != ?2`。
- `logout`:有 Bearer 就删;`{ ok: true }`。
- `me`:`{ id, email, syncEnabled, emailVerified, totpEnabled, requirements: { emailVerify: raw("email_verify"), totpRequired: raw("totp_required") } }`。

- [ ] **Step 4: 测试**

`test/crypto.test.ts`:`hashPassword` 往返、错口令 false、格式前缀 `pbkdf2$600000$`;`timingSafeEqual` 长度不等 false。
`test/account.test.ts`:注册 201 且带 token;重复邮箱 409;弱口令 400 `weak_password`;第 4 次注册(同 IP,limit=3)429;登录成功/错口令 401;`feature_login=0` + 总闸 on(测试里用 `KITE_FEATURE_GATING` 绑定覆盖,可在 vitest.config 的 bindings 里设 "on" 并让 Task 1 测试相应调整,或单独 `describe` 里直接断言 `effective` 函数)→ 403 `LOGIN_DISABLED`;`/api/me` 带令牌 200、无令牌 401;`feature_multi_device=0` 时二次登录后旧令牌 401;logout 后 401。

- [ ] **Step 5: 跑通并提交**

`pnpm test && pnpm typecheck`;`git commit -m "feat(worker): 账户 —— PBKDF2 口令、令牌、KV 限流、注册/登录/登出/me、单设备策略"`。

---

### Task 3: TOTP、邮箱验证、Resend 发信、管理员动作

**Files:** `src/crypto.ts`(base32 + TOTP)、`src/mail.ts`、`src/routes/account.ts`(TOTP 分支与新接口)、`src/routes/admin.ts`(verify / 2fa reset / revoke)、`test/totp.test.ts`、`test/verify-2fa.test.ts`。

**Interfaces:**
- Produces:`base32Encode(u8) / base32Decode(s)`(RFC 4648 无填充)、`newTotpSecret()`(20 字节)、`totpCodeAt(secretB32, ts)`、`totpVerify(secretB32, code, now)`(±1 步)、`otpauthUrl(secret, email)`;`sendCode(env, to, code) -> Promise<void>`(有 `RESEND_API_KEY` 时 `POST https://api.resend.com/emails` `{ from: env.KITE_SMTP_FROM, to, subject: "KiteTerm 邮箱验证码", text }`,否则 `console.log("[mail] 未配置邮件服务 —— {to} 的邮箱验证码: {code}(10 分钟内有效)")`);接口 `POST /api/verify-email`、`POST /api/verify-email/resend`、`POST /api/2fa/setup|enable|disable`、`POST /api/admin/users/:email/verify`、`POST /api/admin/users/:email/2fa/reset`、`POST /api/admin/users/:email/revoke`。

- [ ] **Step 1: TOTP(WebCrypto HMAC-SHA1)**
```ts
const B32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
export function base32Encode(u: Uint8Array): string { let bits = 0, val = 0, out = ""; for (const b of u) { val = (val << 8) | b; bits += 8; while (bits >= 5) { out += B32[(val >>> (bits - 5)) & 31]; bits -= 5; } } if (bits > 0) out += B32[(val << (5 - bits)) & 31]; return out; }
export function base32Decode(s: string): Uint8Array { let bits = 0, val = 0; const out: number[] = []; for (const ch of s.toUpperCase().replace(/=+$/, "")) { const i = B32.indexOf(ch); if (i < 0) throw new Error("bad base32"); val = (val << 5) | i; bits += 5; if (bits >= 8) { out.push((val >>> (bits - 8)) & 255); bits -= 8; } } return new Uint8Array(out); }
export const newTotpSecret = () => base32Encode(crypto.getRandomValues(new Uint8Array(20)));
async function hotp(secret: Uint8Array, counter: number): Promise<number> {
  const key = await crypto.subtle.importKey("raw", secret, { name: "HMAC", hash: "SHA-1" }, false, ["sign"]);
  const msg = new Uint8Array(8); new DataView(msg.buffer).setBigUint64(0, BigInt(counter));
  const h = new Uint8Array(await crypto.subtle.sign("HMAC", key, msg));
  const off = h[19] & 0x0f;
  return (((h[off] & 0x7f) << 24) | (h[off + 1] << 16) | (h[off + 2] << 8) | h[off + 3]) % 1_000_000;
}
export async function totpCodeAt(secretB32: string, ts: number): Promise<string> { return String(await hotp(base32Decode(secretB32), Math.floor(ts / 30))).padStart(6, "0"); }
export async function totpVerify(secretB32: string, code: string, now: number): Promise<boolean> {
  code = code.trim(); if (!/^\d{6}$/.test(code)) return false;
  for (const d of [-1, 0, 1]) if ((await totpCodeAt(secretB32, now + d * 30)) === code) return true;
  return false;
}
export const otpauthUrl = (secret: string, email: string) => `otpauth://totp/KiteTerm:${email}?secret=${secret}&issuer=KiteTerm&algorithm=SHA1&digits=6&period=30`;
```
测试向量(RFC 6238 附录 B,密钥 `12345678901234567890` 的 base32 为 `GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ`):t=59 → `287082`,t=1111111109 → `081804`,t=1234567890 → `005924`;窗口 ±30 s 通过、+90 s 失败;非 6 位数字失败。

- [ ] **Step 2: 接口(逐条移植 Rust 语义)**
- `login` TOTP 分支(在 `LOGIN_DISABLED` 之后、签发之前):`totp_enabled` 时先 `rateLimitOk(RATE, "totp", `${userId}:${ip}`, 10)` 否则 429 `TOTP_RATE_LIMITED`;缺码 401 `TOTP_REQUIRED`「请输入动态验证码」;错码 401 `TOTP_INVALID`「动态验证码错误」。`totpSetupRequired = raw("totp_required") && !totp_enabled`。
- 注册:`raw("email_verify")` 为真 → `email_verified=0`,写 `email_codes`(6 位数字码,`expires_at=now+600`,`sent_at=now`,`attempts=0`),`c.executionCtx.waitUntil(sendCode(...))`;响应 `user.emailVerified` 相应。
- `POST /api/verify-email {code}`:限流 `("verify_email", user.id, 10)` → 429 `CODE_RATE_LIMITED`;无行 400 `CODE_INVALID`「验证码不存在,请重新发送」;过期 400 `CODE_EXPIRED`;错码 `attempts+1`,≥5 删行并 400 `CODE_INVALID`「验证码错误次数过多,请重新发送」,否则 400 `CODE_INVALID`「验证码错误」;对 → `email_verified=1` 删行 `{ ok, emailVerified: true }`。
- `POST /api/verify-email/resend`:已验证直接 ok;`sent_at` 距今 <60 s → 429 `RESEND_TOO_SOON`;写新码(attempts 归零)后 `await sendCode`,失败 502 `MAIL_ERROR`。
- `POST /api/2fa/setup`:已启用 409 `TOTP_ALREADY_ENABLED`;生成密钥写 `totp_secret`;返回 `{ secret, otpauthUrl }`。`enable {code}`:无密钥 400 `TOTP_NOT_PENDING`;错码 400 `TOTP_INVALID`;成功 `totp_enabled=1`。`disable {code}`:`raw("totp_required")` → 403 `TOTP_ENFORCED`;未启用或错码 400 `TOTP_INVALID`;成功清空。
- 管理:`verify`(置 1 并删 `email_codes`)、`2fa/reset`(清密钥、置 0、删该用户全部令牌)、`revoke`(删令牌,返回 `{ ok, revoked: changes }`);均 0 行 → 404 `not_found`。

- [ ] **Step 3: 测试**

`test/totp.test.ts`:上面的向量与窗口。`test/verify-2fa.test.ts`(用 `cloudflare:test` 的 `fetchMock` 拦截 `https://api.resend.com`,或不配 `RESEND_API_KEY` 走日志路径并从 D1 读码):开邮箱验证 → 注册 `emailVerified:false` → 错码 ×5 后行被删 → 重发 60 s 内 429 → 从 D1 读码验证成功 → `/api/me` 为 true;TOTP:setup → 用 `totpCodeAt` 算码 enable → 无码登录 401 `TOTP_REQUIRED` → 带码 200 → 强制开关开时 disable 403 → 管理员 reset 后旧令牌 401。

- [ ] **Step 4: 跑通并提交**

`pnpm test && pnpm typecheck`;`git commit -m "feat(worker): TOTP 双因素、邮箱验证(Resend/日志)、管理员标记与重置"`。

---

### Task 4: 存储配置、S3 探测与用量、凭据下发(R2 临时凭据 / STS / 共享回退)

**Files:** `src/storage.ts`、`src/creds.ts`、`src/routes/admin.ts`(storage GET/POST/test、usage、stats 改读配置)、`src/routes/account.ts`(`GET /api/sync/credentials`)、`test/storage.test.ts`、`test/credentials.test.ts`。

**Interfaces:**
- `StorageConfig { endpoint, region, bucket, prefix, accessKey, secretKey, pathStyle, stsMode }`;`loadStorage(db, env) -> { cfg, source: "db" | "env" }`(DB 键 `storage_config` JSON 优先,否则 env 初值;`normalize`:endpoint 去尾 `/`,`prefix` 规范为空或 `xxx/`,stsMode 非法回 auto);`prefixValid(p)`(拒绝 `..`、`\`、非 `[A-Za-z0-9._/-]`);`publicJson(cfg, source)`(`hasSecret`、不含 secret、`configured`);`userPrefix(cfg, userId) = `${cfg.prefix}users/${userId}``。
- `s3Client(cfg)` 基于 `aws4fetch`:`new AwsClient({ accessKeyId, secretAccessKey, service: "s3", region })`;`objectUrl(cfg, key)`:pathStyle → `${endpoint}/${bucket}/${key}`,否则把 bucket 作为子域插到 endpoint 主机前;`listUrl(cfg, prefix, token?)` → `?list-type=2&prefix=...&max-keys=1000[&continuation-token=...]`。
- `probe(cfg, env) -> { ok, latencyMs, list, put, delete, sts }`(语义同 Rust;`bucket()` 构造失败在 TS 里不存在,略;`sts` 在 `stsMode != "off"` 时调用 `issueCredentials` 试签发,失败写原因)。
- `usage(cfg, userId) -> { objects, bytes }`(分页累加,`<Contents>` 里 `<Size>`)。
- `issueCredentials(cfg, env, userId) -> Promise<Creds>`:`Creds { endpoint, bucket, region, accessKey, secretKey, sessionToken?, expiration?, prefix, pathStyle }`。逻辑:`stsMode==="off"` → 共享;否则若 `isR2(cfg.endpoint)`(host 以 `.r2.cloudflarestorage.com` 结尾)且有 `R2_ACCOUNT_ID`/`R2_API_TOKEN` → `r2TempCredentials`;否则 `stsAssumeRole`(移植 `sts.rs`:aws4fetch `service:"sts"`,POST `${endpoint}/` 表单 `Action=AssumeRole&Version=2011-06-15&DurationSeconds=3600&RoleArn=arn:aws:iam::000000000000:role/vela-sync&RoleSessionName=kite-{uid}&Policy=<JSON>`,正则取 `<AccessKeyId>` 等);失败:`require` → 抛 `sts_error`(502),`auto` → 回退共享凭据并 `console.warn`。
- `r2TempCredentials`:`POST https://api.cloudflare.com/client/v4/accounts/${R2_ACCOUNT_ID}/r2/temp-access-credentials`,Bearer `R2_API_TOKEN`,body `{ bucket, parentAccessKeyId: cfg.accessKey, permission: "object-read-write", ttlSeconds: 3600, prefixes: [`${userPrefix}/`] }`;成功取 `result.accessKeyId/secretAccessKey/sessionToken`,`expiration` = now+3600 的 ISO 串;`success:false` 抛错含 `errors[0].message`。
- `GET /api/sync/credentials` 检查顺序(与 Rust 终审后一致):`STORAGE_UNCONFIGURED`(503)→ 平台 `SYNC_DISABLED`(`effective("sync")`,403)→ 用户 `SYNC_DISABLED`(403,「该账户的云同步已被平台停用,请联系管理员」)→ `EMAIL_UNVERIFIED`(403)→ `TOTP_SETUP_REQUIRED`(403)→ 下发。
- 管理:`GET /api/admin/storage`;`POST /api/admin/storage`(body 同 Rust `StorageBody`,空 secret 沿用;endpoint 须 http(s) 开头否则 400 `INVALID_ENDPOINT`;bucket/accessKey/secret 非空且 `prefixValid` 否则 400 `INVALID_STORAGE`;保存到 `storage_config`;返回 publicJson(source "db"));`POST /api/admin/storage/test`(用表单值探测);`GET /api/admin/users/:email/usage`;`GET /api/admin/stats` 改读 `loadStorage`。

- [ ] **Step 1: 实现上述模块**(代码由实现者按接口写;XML 解析用简单正则 `/<Key>(.*?)<\/Key>/g`、`/<Size>(\d+)<\/Size>/g`、`/<NextContinuationToken>(.*?)<\//`;`AwsClient.fetch(url, { method, body, headers })` 会自动签名。)

- [ ] **Step 2: 测试**

`test/storage.test.ts`:`normalize`(`/prod//`→`prod/`、endpoint 去尾斜杠、stsMode 非法→auto)、`prefixValid`(`../evil`、`a\b` false,`prod/v2` true)、`publicJson` 无 `secretKey`;管理接口:GET 初值 `source:"env"`;POST 空 secret 沿用、`../evil` 400、非 http 400;保存后 `/api/admin/stats` 的 bucket 立刻变化。
`test/credentials.test.ts`:用 `fetchMock`(`import { fetchMock } from "cloudflare:test"; fetchMock.activate(); fetchMock.disableNetConnect();`)拦截 `https://api.cloudflare.com` 返回 `{ success:true, result:{ accessKeyId:"ak", secretAccessKey:"sk", sessionToken:"st" } }`,配置 endpoint 为 `https://acct.r2.cloudflarestorage.com` + `R2_ACCOUNT_ID/R2_API_TOKEN` 绑定 → `/api/sync/credentials` 返回 `sessionToken:"st"` 且 `prefix` 为 `users/{id}`;拦截返回 `success:false` 时 `stsMode=require` → 502 `sts_error`,`auto` → 回退共享凭据(无 sessionToken);检查顺序:未配置存储 → 503;停用用户 + 强制 2FA → 用户级 `SYNC_DISABLED`;`probe` 用 fetchMock 模拟 S3 端点(list 200 / put 200 / delete 204)→ `ok:true`,put 403 → `ok:false` 且 `put` 有原因。

- [ ] **Step 3: 跑通并提交**

`pnpm test && pnpm typecheck`;`git commit -m "feat(worker): 存储配置入库、S3 探测与用量、R2 临时凭据/STS/共享回退的凭据下发"`。

---

### Task 5: 交付 —— 本地冒烟、CI、文档

**Files:** `server-worker/README.md`、`.github/workflows/worker-deploy.yml`、`README.md`(顶层加一行指向)、`docs/deploy/cloudflare-workers.md`(状态改为「已实现」,部署步骤对齐脚本名)。

- [ ] **Step 1: 冒烟**

`pnpm dev`(`wrangler dev --local`,自动建本地 D1/KV)后用 curl 走一遍:`/api/health`、`/api/features`、管理 settings、注册→登录→`/api/me`、TOTP setup/enable/登录带码、`/api/admin/storage` 保存(endpoint 填 `http://127.0.0.1:9`)→ `/api/admin/storage/test` 返回 `ok:false` 且各项有原因、`/api/sync/credentials` 各错误码顺序。把命令与输出写进报告。`curl -s http://127.0.0.1:8787/admin | head -3` 是控制台 HTML。

- [ ] **Step 2: CI**

`.github/workflows/worker-deploy.yml`:`on: push: tags: ["worker-v*"]` 与 `workflow_dispatch`;步骤:checkout → pnpm/action-setup → `pnpm install --frozen-lockfile`(在 `server-worker/`)→ `pnpm test` → `pnpm typecheck` → `pnpm sync-admin` → `npx wrangler d1 migrations apply kite-server --remote` → `npx wrangler deploy`,环境变量 `CLOUDFLARE_API_TOKEN`、`CLOUDFLARE_ACCOUNT_ID` 来自 secrets。

- [ ] **Step 3: 文档**

`server-worker/README.md`:一次性初始化命令(d1 create / kv create / r2 bucket create / migrations apply / secret put ×5)、`vars` 说明、R2 两种令牌怎么建、自定义域、本地开发与测试、从 Rust 版迁移提示(口令需重设)。顶层 README 管理平台段加「也可部署到 Cloudflare Workers,见 `server-worker/README.md`」。

- [ ] **Step 4: 提交**

`git commit -m "docs(worker): 冒烟记录、部署工作流与 README"`。

---

## 自检记录

- 契约覆盖:Rust 版全部 26 条路由(公开 6、账户 8、管理 12)在 T1–T4 各有归属;`scheduled` 清理令牌为新增(Rust 版按需过滤)。
- 类型一致:`apiError/requireAdmin/nowTs/clientIp`(T1)被 T2–T4 使用;`raw/effective`(T1)被 T2–T4 使用;`rateLimitOk`(T2)被 T3 使用;`authenticate/AuthedUser`(T2)被 T3–T4 使用;`loadStorage/userPrefix/issueCredentials`(T4)自洽。
- 占位扫描:T4 Step 1 让实现者按接口写实现,接口签名、URL、字段、错误码均已给全。
