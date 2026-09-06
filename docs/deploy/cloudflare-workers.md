# KiteTerm 管理平台 —— Cloudflare Workers 部署方案

> 状态:方案(2026-09-06)。现有 `server/`(Rust axum + SQLite)保持为 VPS 部署形态;Workers 形态是**同一 API 契约的另一实现**,放在新目录 `server-worker/`,桌面端零改动。

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
  package.json          hono, aws4fetch, otpauth(或自写 HMAC), vitest, @cloudflare/vitest-pool-workers
  migrations/0001_init.sql
  public/admin.html     ← 复用 server/src/admin.html(构建时复制,保持单一来源)
  src/index.ts          Hono app,路由与现有 /api/* 一一对应
  src/features.ts       11 项开关注册表(与 server/src/features.rs 同表)
  src/storage.ts        存储配置读写、prefix 校验、aws4fetch 探测
  src/creds.ts          R2 临时凭据 / 共享凭据回退
  src/auth.ts           PBKDF2、令牌、TOTP、限流
  src/mail.ts           Resend
  test/*.test.ts        用 vitest-pool-workers 跑在 workerd 里,直连 D1/KV 的本地实现
```

## 4. wrangler.toml(草案)

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
database_id = "<wrangler d1 create 输出>"

[[kv_namespaces]]
binding = "RATE"
id = "<wrangler kv namespace create RATE 输出>"

[vars]
KITE_REGISTRATION = "invite"
KITE_FEATURE_GATING = "off"
KITE_REGISTER_LIMIT = "5"
S3_ENDPOINT = "https://<account_id>.r2.cloudflarestorage.com"
S3_BUCKET = "kite-sync"
S3_REGION = "auto"
S3_PATH_STYLE = "on"
KITE_STS = "require"           # R2 走临时凭据;非 R2 存储可设 auto
R2_ACCOUNT_ID = "<account_id>"
```

密钥用 `wrangler secret put` 注入:`KITE_ADMIN_KEY`、`S3_ACCESS_KEY`、`S3_SECRET_KEY`、`R2_API_TOKEN`(仅需 R2 临时凭据签发权限)、`RESEND_API_KEY`、`KITE_SMTP_FROM`(发件人,可放 vars)。

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

```bash
cd server-worker && pnpm install
wrangler login
wrangler d1 create kite-server                 # 把 database_id 填进 wrangler.toml
wrangler kv namespace create RATE              # 把 id 填进 wrangler.toml
wrangler r2 bucket create kite-sync
wrangler d1 migrations apply kite-server --remote
for s in KITE_ADMIN_KEY S3_ACCESS_KEY S3_SECRET_KEY R2_API_TOKEN RESEND_API_KEY; do wrangler secret put $s; done
pnpm test && wrangler deploy
wrangler tail                                   # 看首个请求日志
```

R2 的 S3 API 凭据:在 Cloudflare 面板「R2 → 管理 API 令牌」创建一对只限 `kite-sync` 桶的 Access Key,填 `S3_ACCESS_KEY/S3_SECRET_KEY`;`R2_API_TOKEN` 是另一个 API Token,只勾「Workers R2 Storage: Edit」用于签发临时凭据。

自定义域:`wrangler.toml` 加 `routes = [{ pattern = "kite.example.com", custom_domain = true }]`,桌面端「设置 → 管理平台」填 `https://kite.example.com`。

## 8. 从 VPS 版迁移(可选)

- 数据:`sqlite3 kite-server.db .dump` 去掉 `CREATE TABLE`,只留 `INSERT`,`wrangler d1 execute kite-server --remote --file dump.sql`。
- 口令:Argon2 哈希无法在 Workers 内校验(CPU 与 WASM 体积都不划算)。两种做法:① 迁移后要求用户重设密码(登录时对 `argon2$` 前缀的哈希返回 `PASSWORD_RESET_REQUIRED`,走邮件验证码重设);② 迁移窗口期内 VPS 与 Worker 并跑,Worker 对 Argon2 用户反向调用 VPS `/api/login` 校验并顺手改写成 PBKDF2。
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
