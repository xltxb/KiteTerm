# KiteTerm 管理平台 —— Cloudflare Workers 版

与 `server/`(Rust axum + SQLite,VPS 部署)API 契约完全一致的另一实现:TypeScript(Hono)+ D1 + R2/S3 + KV,
桌面端零改动,只需把「设置 → 管理平台」的地址指向这个 Worker 的域名。设计取舍见 `docs/deploy/cloudflare-workers.md`。

## 一次性初始化

需要 Cloudflare 账号与 **Workers 付费档**(口令 PBKDF2 60 万轮、TOTP 校验的 CPU 时间超出免费档 10 ms/请求上限)。

```bash
cd server-worker && pnpm install
npx wrangler login

npx wrangler d1 create kite-server
# 把输出里的 database_id 填进 wrangler.toml 的 [[d1_databases]]

npx wrangler kv namespace create RATE
# 把输出里的 id 填进 wrangler.toml 的 [[kv_namespaces]]

npx wrangler r2 bucket create kite-sync
# 桶名需与 wrangler.toml [vars] 里的 S3_BUCKET 一致

npx wrangler d1 migrations apply kite-server --remote
# 等价于 pnpm migrate:remote

for s in KITE_ADMIN_KEY S3_ACCESS_KEY S3_SECRET_KEY R2_API_TOKEN RESEND_API_KEY; do
  npx wrangler secret put $s
done
# RESEND_API_KEY 可留空(不设置就走日志兜底,见下文「邮件」)

pnpm test && pnpm deploy
npx wrangler tail   # 看首个请求日志,确认部署成功
```

## vars(明文,写在 `wrangler.toml` `[vars]`)与 secrets(`wrangler secret put`)

| 名称 | 类别 | 默认 | 说明 |
|---|---|---|---|
| `KITE_REGISTRATION` | var | `open` | 注册策略初值:`open`/`invite`/`closed`;之后以管理控制台「用户与同步」页设置为准(存 D1,优先于此变量) |
| `KITE_FEATURE_GATING` | var | `off` | 功能门禁总闸;关闭时 8 个产品类开关全部失效放行,3 个安全策略类开关不受影响 |
| `KITE_REGISTER_LIMIT` | var | `5` | 同一 IP 每小时注册尝试上限(KV 滑动窗口) |
| `S3_ENDPOINT` | var | 空 | S3 兼容端点,R2 形如 `https://<account_id>.r2.cloudflarestorage.com`;仅作 D1 无存储配置时的初值,控制台保存后以 D1 为准 |
| `S3_BUCKET` | var | `kite-sync` | 桶名 |
| `S3_REGION` | var | `auto` | R2 固定 `auto`;其余 S3 兼容存储按实际区域填 |
| `S3_PREFIX` | var | 空 | 对象路径前缀,规范化后必以 `/` 结尾 |
| `S3_PATH_STYLE` | var | `on` | MinIO/自建网关需开启;AWS S3/R2 关闭(填 `off`) |
| `KITE_STS` | var | `auto` | `auto`(优先限权临时凭据,失败回退共享凭据)/`require`(失败即报错,生产建议)/`off`(直接共享凭据) |
| `R2_ACCOUNT_ID` | var | 空 | R2 临时凭据 API 用;留空则从 R2 endpoint 子域自动解析,仅需显式覆盖时填写 |
| `KITE_SMTP_FROM` | var | 空 | Resend 发信人地址 |
| `KITE_ADMIN_KEY` | secret | — | 管理控制台/管理 API 密钥,`X-Admin-Key` 头 |
| `S3_ACCESS_KEY` | secret | — | S3 Access Key |
| `S3_SECRET_KEY` | secret | — | S3 Secret Key |
| `R2_API_TOKEN` | secret | — | 见下文「R2 的两种令牌」,仅 R2 端点需要 |
| `RESEND_API_KEY` | secret | — | 不设置则验证码只打进 `wrangler tail` 日志,不发邮件 |

## R2 的两种令牌

云同步用到 R2 时需要两把互不相同的密钥/令牌,职责不要混:

1. **S3 API 令牌**(`S3_ACCESS_KEY` / `S3_SECRET_KEY`):Cloudflare 面板「R2 → 管理 API 令牌」创建,权限限定到 `kite-sync` 单个桶,读写权限。这是 `aws4fetch` 走 S3 兼容协议签名请求(列桶/探测/用量统计/共享凭据回退)用的密钥。
2. **`R2_API_TOKEN`**:另一个 Cloudflare API Token(面板「我的个人资料 → API 令牌」),只勾 **Workers R2 Storage: Edit** 权限,用于调用 `POST /accounts/{id}/r2/temp-access-credentials` 签发按 `users/{id}/*` 收敛的限权临时凭据(固定 1 小时)。不需要账号级其它权限。

非 R2 存储(自建 MinIO 等)不需要 `R2_API_TOKEN` 与 `R2_ACCOUNT_ID`,`KITE_STS` 走通用 STS `AssumeRole` 分支或直接 `off` 回退共享凭据。

## 自定义域

`wrangler.toml` 加:

```toml
routes = [{ pattern = "kite.example.com", custom_domain = true }]
```

`pnpm deploy` 后桌面端「设置 → 管理平台」填 `https://kite.example.com`。

## 本地开发与测试

```bash
pnpm dev         # wrangler dev --local,自动建本地 D1/KV,访问 http://127.0.0.1:8787
pnpm test        # vitest run(@cloudflare/vitest-plugin,直连 workerd 里的本地 D1/KV),79 用例
pnpm typecheck   # tsc --noEmit
```

本地开发的密钥(`KITE_ADMIN_KEY` 等)放项目根的 `server-worker/.dev.vars`(不提交,已在 `.gitignore`),例如:

```
KITE_ADMIN_KEY=devadminkey
S3_ACCESS_KEY=devaccesskey
S3_SECRET_KEY=devsecretkey
```

注意:`.dev.vars` 会被 `pnpm test` 的 vitest-plugin 一并读取,若其中设置了 `S3_ACCESS_KEY`/`S3_SECRET_KEY`,会导致依赖"未配置存储"默认值的用例失败(`storage.test.ts`)。跑测试前建议临时移走或清空 `.dev.vars` 里的 S3 字段,仅本地手工冒烟时再加回。

`/admin` 是内嵌的管理控制台静态页,构建/开发/部署前都会先跑 `pnpm sync-admin`(`scripts/sync-admin.sh`),从 `server/src/admin.html` 复制到 `server-worker/public/`,保持前端单一来源、不手改。

`pnpm-workspace.yaml` 仅用于 pnpm 的 build 脚本审批记录(`allowBuilds`),与 monorepo 结构无关。

## 从 VPS 版(`server/`)迁移

- **口令必须重设**:VPS 版用 Argon2id,这里用 PBKDF2-SHA256(600000 轮),两种哈希互不兼容。迁移用户数据后,老用户首次登录会因哈希格式不认得而失败;两版都没有「忘记密码」找回接口,管理控制台也没有删除用户的入口(`/api/register` 用 `INSERT OR IGNORE` 按邮箱去重,只清空 `password_hash` 并不能让原邮箱重新注册),需要管理员直接在 D1 执行 SQL 删除该用户所在行(如 `wrangler d1 execute kite-server --remote --command "DELETE FROM users WHERE email='...'"`),再引导其用同一邮箱重新注册。当前实现也没有自动识别 `argon2$` 前缀并转发校验的兼容层。
- **数据迁移**(可选):`sqlite3 kite-server.db .dump` 去掉 `CREATE TABLE` 只留 `INSERT`,`wrangler d1 execute kite-server --remote --file dump.sql` 导入 D1。
- **对象数据**:桶内对象是客户端主密钥加密的密文,原样复制到 R2 即可(如 `rclone copy`),`S3_PREFIX` 保持一致。

## 已知限制

- **限流按 `CF-Connecting-IP`**:取不到时退到 `X-Forwarded-For` 首段,再取不到全部归入 `unknown` 桶共享限额。跑在 Cloudflare 边缘时 `CF-Connecting-IP` 总是存在,不受影响;若把这个 Worker 放在别的代理后面自定义转发,需确认转发链路会设置这两个头之一。
- **KV 计数最终一致**:限流窗口值存在 KV,短暂的并发下可能有少量超发,不是安全边界失效(注册/TOTP/验证码限流都只是防暴力枚举的降噪手段)。需要严格计数时可把 `rateLimitOk` 换成 Durable Object。
- **口令哈希算法与 VPS 版不同**:PBKDF2-SHA256(60 万轮)而非 Argon2id,见上文「迁移」。
- **S3 Secret 与 TOTP 密钥明文落 D1**:与 VPS 版明文放环境变量/SQLite 的强度一致,不是本版本引入的新风险,但同样需要限制谁能访问 D1 控制台。
- **R2 临时凭据 TTL 固定 1 小时**:`ttlSeconds: 3600` 写死在 `src/creds.ts`,未做成可配置项(R2 API 允许 15 分钟 ~ 7 天)。
- **`authorization` 头的 `bearer` 大小写宽松**:`/^Bearer\s+/i` 不区分大小写匹配前缀;VPS 版(`server/src/main.rs`)用 `strip_prefix("Bearer ")` 严格匹配大小写与单个空格。
- **缺字段/类型错误的请求体返回 400 而非 422**:本版本手工校验并统一走 `{error:{code,message}}` 的 400;VPS 版用 axum 的 `Json<T>` 提取器,反序列化失败时框架默认返回 422 Unprocessable Entity。
