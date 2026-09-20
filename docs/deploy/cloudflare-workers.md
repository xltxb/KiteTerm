# KiteTerm 管理平台 —— Cloudflare Workers 部署方案

> 状态:**已实现**(2026-09-06)。现有 `server/`(Rust axum + SQLite)保持为 VPS 部署形态;Workers 形态是**同一 API 契约的另一实现**,放在 `server-worker/`,桌面端零改动。全部 26 条路由、D1 迁移、KV 限流、PBKDF2/TOTP、Resend/日志兜底邮件、S3 存储配置与探测、R2 临时凭据/通用 STS/共享凭据回退均已落地;`pnpm test` 79 用例、`pnpm typecheck` 通过。初始化步骤、vars/secrets 清单、R2 令牌申请、自定义域、迁移与已知限制见 `server-worker/README.md`(本文档保留设计取舍与背景,操作以该 README 为准)。

## 1. 结论

- 用 **TypeScript(Hono)+ D1 + R2 + KV** 重写服务端,不用 workers-rs:Workers 上没有文件系统、原生 TCP 与常驻进程,现有 Rust 的持久化/网络/限流四层都要换,保留 Rust 没有收益。
- 需要 **Workers 付费档**(每月 5 美元):口令哈希(PBKDF2 60 万轮)与 TOTP 校验都要 CPU,免费档每请求 10 ms 的 CPU 上限会超;付费档 30 s。
- 假设是**全新部署**,没有存量 Argon2 口令哈希需要迁移。若要从 VPS 版迁移用户,见 §8。

## 2. 组件映射

| 现有(Rust / VPS) | Workers 形态 | 说明 |
|---|---|---|
| SQLite 文件(rusqlite) | **D1** | SQLite 方言,`platform_settings / users / tokens / invites / invite_uses / email_codes` 六张表原样迁移 |
| `RwLock<StorageConfig>` 热配置 | D1 `platform_settings` 每请求读取 | Workers 无进程状态;一条 `SELECT` 开销可忽略 |
| 内存滑动窗口限流(注册 / TOTP / 验证码) | **KV**(键带 TTL 计数)或 Workers 限流绑定 | KV 最终一致,允许少量超发;严格场景改 Durable Object |
| STS `AssumeRole`(MinIO) | **R2 临时访问凭据 API** | `POST /accounts/{id}/r2/temp-access-credentials`,可限定 bucket + 前缀 + TTL,语义与现有 session policy 一致;非 R2 存储沿用共享凭据回退 |
| rust-s3 探测桶 | `aws4fetch` 签名 + `fetch` | 测试连接 / 用量统计 |
| lettre SMTP | **Resend** HTTP API(或 MailChannels) | 未配置时验证码打进 `wrangler tail` 日志,行为与现在一致 |
| Argon2id | **PBKDF2-SHA256**(WebCrypto,600 000 轮,16 字节盐) | 哈希格式 `pbkdf2$<iter>$<salt_b64>$<hash_b64>` |
| `hmac`+`sha1` TOTP | WebCrypto `HMAC-SHA1` | RFC 6238 向量测试原样移植 |
| 环境变量 | Worker `vars`(非密)+ `secrets`(密) | 见 §5 |
| `include_str!("admin.html")` | Workers 静态资源 `[assets]` | `/admin` 直接由资源层返回,不占 CPU |
| 启动时建表迁移 | `wrangler d1 migrations apply` | 迁移文件版本化,部署前执行 |

## 3. 目录与技术栈

```
server-worker/
  wrangler.toml
  package.json            hono, aws4fetch, vitest, @cloudflare/vitest-plugin
  migrations/0001_init.sql
  scripts/sync-admin.sh   复制 server/src/admin.html → public/(admin.html + index.html),保持单一来源
  public/                 sync-admin 生成,不入库
  src/index.ts            Hono app 装配 + scheduled(每日清理过期令牌,替代 Rust 版按需过滤)
  src/env.ts              Env 类型 + featureGating/registrationDefault
  src/http.ts             apiError/requireAdmin/nowTs/clientIp/bearerToken
  src/settings.ts         platform_settings 键值读写
  src/features.ts         11 项开关注册表(与 server/src/features.rs 同表)
  src/crypto.ts           PBKDF2 口令哈希、TOTP(RFC 6238)、base32、常量时间比较
  src/auth.ts             令牌签发、authenticate、邀请码
  src/ratelimit.ts        KV 滑动窗口限流
  src/storage.ts          存储配置读写、prefix 校验、aws4fetch 探测(probe)/ 用量统计
  src/creds.ts            R2 临时凭据 / 通用 STS AssumeRole / 共享凭据回退
  src/mail.ts             Resend,未配置时验证码打日志
  src/routes/public.ts    /api/health、/api/features、/api/register/policy
  src/routes/account.ts   注册/登录/登出/me/同步凭据/邮箱验证/2FA(8 条)
  src/routes/admin.ts     设置/邀请码/用户/统计/存储配置与探测(12 条)
  test/*.test.ts          @cloudflare/vitest-plugin 跑在 workerd 里,直连本地 D1/KV(79 用例)
```

## 4. wrangler.toml

现状(`server-worker/wrangler.toml`),`database_id`/`kv id` 两处占位需在 `wrangler d1 create` / `wrangler kv namespace create RATE` 后填入实际输出:

```toml
name = "kite-server"
main = "src/index.ts"
compatibility_date = "2026-09-01"
compatibility_flags = ["nodejs_compat"]

[assets]
directory = "./public"

[[d1_databases]]
binding = "DB"
database_name = "kiteterm"
database_id = "00000000-0000-0000-0000-000000000000"   # wrangler d1 create kiteterm 后替换

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

`KITE_SMTP_FROM`(Resend 发件人地址)与 `R2_ACCOUNT_ID` 是明文 `vars`,不是 secret;生产环境建议把 `KITE_REGISTRATION` 改回 `invite`、`KITE_STS` 改 `require`(R2 场景),`S3_ENDPOINT`/`R2_ACCOUNT_ID` 填实际值。密钥清单与 R2 两种令牌的区别见 `server-worker/README.md`。

## 5. 接口对照

所有路径、请求体、响应体与错误码(`{error:{code,message}}`,大写下划线)与 `server/` 完全一致,桌面端与控制台不改。差异只在实现:

| 路径 | Workers 实现要点 |
|---|---|
| `GET /api/features` | D1 读 `feature_*`,`enforced` 来自 `KITE_FEATURE_GATING` |
| `POST /api/register` | KV 限流 `register:<ip>`(`CF-Connecting-IP`)→ PBKDF2 → D1 事务(`batch`)写 users/invite_uses → 若开邮箱验证则写 email_codes 并 `ctx.waitUntil(sendCode())` |
| `POST /api/login` | PBKDF2 校验 → TOTP(KV 限流 `totp:<user>:<ip>` 10 次/小时)→ 发令牌 → 多设备关闭时删其它令牌 |
| `GET /api/sync/credentials` | 检查顺序不变;`stsMode != off` 且端点为 R2 时调临时凭据 API(prefix `{prefix}users/{id}/`,TTL 3600),否则回退共享凭据 |
| `POST /api/admin/storage/test` | aws4fetch:`ListObjectsV2(prefix)` → `PutObject(.kite-probe)` → `DeleteObject`;R2 时再试一次临时凭据签发 |
| `GET /api/admin/users/{email}/usage` | `ListObjectsV2` 分页累加 |
| `/admin`、`/` | 静态资源 |

限流键统一 `"<bucket>:<key>"`,值为窗口内计数,`expirationTtl = 3600`;超限返回与现在相同的 429 码。

## 6. D1 迁移

`migrations/0001_init.sql` 直接取自 `server/src/main.rs` 的建表语句 + 三个 `add_column_if_missing` 列(在新库里直接写进 `CREATE TABLE`):

```sql
CREATE TABLE users (
  id TEXT PRIMARY KEY, email TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL,
  sync_enabled INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL,
  email_verified INTEGER NOT NULL DEFAULT 1, totp_secret TEXT,
  totp_enabled INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE tokens (token TEXT PRIMARY KEY, user_id TEXT NOT NULL, expires_at INTEGER NOT NULL);
CREATE INDEX tokens_user ON tokens(user_id);
CREATE TABLE platform_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE invites (code TEXT PRIMARY KEY, note TEXT NOT NULL DEFAULT '', max_uses INTEGER NOT NULL DEFAULT 0,
  used_count INTEGER NOT NULL DEFAULT 0, expires_at INTEGER NOT NULL DEFAULT 0,
  disabled INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
CREATE TABLE invite_uses (code TEXT NOT NULL, user_id TEXT NOT NULL, used_at INTEGER NOT NULL);
CREATE TABLE email_codes (user_id TEXT PRIMARY KEY, code TEXT NOT NULL, expires_at INTEGER NOT NULL,
  sent_at INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0);
```

过期令牌用 Cron Trigger(`[triggers] crons = ["0 4 * * *"]`)每日清理一次,替代现在的按需过滤。

## 7. 部署步骤

实际脚本名与顺序(`package.json` scripts,与 `.github/workflows/worker-deploy.yml` 一致):

```bash
cd server-worker && pnpm install
npx wrangler login
npx wrangler d1 create kiteterm              # 把 database_id 填进 wrangler.toml
npx wrangler kv namespace create RATE           # 把 id 填进 wrangler.toml
npx wrangler r2 bucket create kite-sync
npx wrangler d1 migrations apply kiteterm --remote   # 等价于 pnpm migrate:remote
for s in KITE_ADMIN_KEY S3_ACCESS_KEY S3_SECRET_KEY R2_API_TOKEN RESEND_API_KEY; do
  npx wrangler secret put $s
done
pnpm test && pnpm deploy   # deploy 脚本内部先跑 pnpm sync-admin 再 wrangler deploy
npx wrangler tail          # 看首个请求日志
```

之后的部署由 CI 完成:打 tag `worker-v*`(或手动触发 `worker-deploy` workflow)即依次执行
`pnpm install --frozen-lockfile → pnpm test → pnpm typecheck → pnpm sync-admin → wrangler d1 migrations apply --remote → wrangler deploy`,
`CLOUDFLARE_API_TOKEN`/`CLOUDFLARE_ACCOUNT_ID` 取自仓库 secrets。本机没有登录态,不要在本地执行 `wrangler deploy` 等 `--remote` 命令。

R2 的 S3 API 凭据与 `R2_API_TOKEN` 的区别、自定义域配置、本地开发与测试见 `server-worker/README.md`。

## 8. 从 VPS 版迁移(可选)

- 数据:`sqlite3 kite-server.db .dump` 去掉 `CREATE TABLE`,只留 `INSERT`,`wrangler d1 execute kiteterm --remote --file dump.sql`。
- 口令:Workers 版用 PBKDF2-SHA256,VPS 版用 Argon2id,两种哈希互不兼容,当前实现**没有**识别旧哈希前缀并转发校验的兼容层——迁移后的用户用旧密码登录会直接收到 `bad_credentials`。需要引导用户走邮箱验证码重设密码,或由管理员在控制台手工处理(见 `server-worker/README.md`「从 VPS 版迁移」)。
- 桶内数据是客户端主密钥加密的密文,换到 R2 只需原样复制对象(`rclone copy`),`prefix` 保持一致。

## 9. 限制与成本

- 付费档每月 5 美元含 1000 万请求;D1 免费额度 500 万行读/天;KV 1000 次写/天免费(限流写入量与登录注册次数同量级,个人/小团队远够);R2 10 GB 免费。
- 单次请求 CPU 30 s、子请求 1000 次:用量统计对超大前缀分页可能触顶,控制台「查看用量」保持按需点击。
- KV 最终一致:限流可能短暂超发几次,不是安全边界失效;需要严格计数时把限流改成 Durable Object。
- R2 临时凭据 TTL 最短 15 分钟、最长 7 天;现用 1 小时。

## 10. 实施阶段

1. **骨架**:Hono 路由、D1 迁移、静态控制台、`/api/features` 与 `/api/admin/settings|invites|users` —— 控制台可用。
2. **账户**:注册/登录/令牌(PBKDF2)、KV 限流、TOTP、多设备、`/api/me`。
3. **存储与凭据**:存储配置、aws4fetch 探测、R2 临时凭据 + 共享回退、用量统计。
4. **邮件**:Resend 发码、验证/重发、管理员手工标记。
5. **交付**:vitest 覆盖 RFC 6238 向量、限流、检查顺序;CI(`wrangler deploy` on tag `worker-v*`);README 部署段。

每阶段结束都能单独部署验收;桌面端只需把平台地址指向 Worker 域名。
