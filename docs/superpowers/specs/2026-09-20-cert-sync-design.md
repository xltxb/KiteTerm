# 证书接入云同步设计

日期:2026-09-20
状态:已确认
来源:`docs/superpowers/specs/2026-09-07-ssl-certificates-acme.md`(F11)「证书随配置加密同步」
      「账户密钥自动生成并随配置同步」两条未实现;代码审查发现 `scope_allows` 无 cert 分支、
      `upsert_cert` 不写 oplog,而 `CertsScreen` 反向声明「仅保存在本机」。

## 目标

把 `cert` 接成一等同步实体:证书本体、证书私钥与 ACME 账户密钥经主密码端到端加密后跨设备同步,
新设备拉到即可直接部署到目标主机,无需重新向 CA 申请。自动续期在多设备下只由一台负责。

## 非目标

- 不改主密钥/设备密钥的加密层次。`key` 实体已确立范式,证书照抄。
- 不新增平台功能开关。证书同步复用既有的 `key_sync`。
- 不做证书的跨账户共享,不做归属设备的自动故障转移(归属机报废由用户手动接管)。

## 已定决策

| 决策 | 取值 | 理由 |
|---|---|---|
| 同步范围 | 新增独立 `certs`,默认开 | 证书含私钥,值得能单独关掉;体积小且规格要求同步,故默认开 |
| 平台开关 | 复用 `key_sync` | 证书私钥也是私钥,平台关掉密钥同步时不应外发;开关仍为 11 项,服务端零改动 |
| 续期归属 | `certs.owner_device_id` | 每台设备都跑 `spawn_renew_loop`,不加归属会重复签发,烧 CA 的「同域名每周 5 张」配额 |
| payload | 全量(含 `cert_pem`/`chain_pem`/`key_pem`) | 能重新部署才是同步的意义,且省 CA 配额 |

## 数据模型

`certs` 表加 `owner_device_id TEXT NOT NULL DEFAULT ''`。该表在存量库里已建好,而 `db.rs` 只有
`CREATE TABLE IF NOT EXISTS`、**没有任何补列机制**,故先加一个幂等的 `add_column_if_missing`
(`PRAGMA table_info` 查一次 + `ALTER TABLE`),再用它加列。存量证书该列为空 → 视为本机接管,
首次启动写入本机 device id。

线上 payload 照抄 `key`(`db.rs` `key_payload` + `sync.rs` `key_payload_plaintext`):

```
出网:  {meta, accountKeyDevEnc, keyPemDevEnc, certPem, chainPem}
       → 解设备密钥层 → {meta, accountKey, keyPem, certPem, chainPem}
       → 整条由主密钥 seal(AAD 绑对象路径)
入网:  apply_wire 用本机设备密钥重新加密 accountKey / keyPem 落库
```

`meta` 含 `owner_device_id`,故归属变更随同步自然传播。

## 后端(`desktop/src-tauri`)

`cert` 是新实体,须在所有按实体分支处登记:

| 位置 | 改动 |
|---|---|
| `sync.rs` `scope_allows` | `"cert" => cfg.scopes.certs && flags.key_sync` |
| `sync.rs` `apply_wire` | 新增 `"cert"`,重新加密两列后 `upsert_cert_raw`(`_raw` = 不写 oplog,避免把远端条目当本地变更再推回去,与 `upsert_host_raw` 同义) |
| `sync.rs` `build_snapshot` | 新增 `"cert"`,走 `cert_payload` + 解设备密钥层 |
| `db.rs` `delete_entity_raw` | 新增 `"cert"` → `DELETE FROM certs` |
| `db.rs` `upsert_cert` / `delete_cert` | 补 `oplog_append("cert", ...)` |
| `db.rs` `bootstrap_oplog` | 补 cert 存量条目 |
| `models.rs` `SyncScopes` | 加 `certs: bool`,`#[serde(default = "default_true")]`(见下) |

`#[serde(default)]` 对 `bool` 取 `false`,与「默认开」相悖 —— 存量配置反序列化后证书范围会静默关闭。
故用 `default = "default_true"`(`features.rs` 已有同名助手可参照),使存量设备升级后与新装一致:
证书范围默认开,下次同步即开始上传。这是「默认开」的应有后果,须在发版说明里点出。

补发两条路径共用一个 `backfill_certs`(与 `backfill_keys` 并列):范围开关关→开(`save_config`)、
平台 `key_sync` 关→开(`sync_once`)。

续期:`spawn_renew_loop` 的判据加 `c.owner_device_id == db.device_id()`。非归属设备不进续期分支,
顺带消掉无会话设备反复打 `ERR_NO_SESSION` 的日志噪音。A、B 同时接管时按 lamport 收敛到一个赢家。

## 前端(`desktop/src`)

- `ScopeCard.tsx`:`SCOPES` 加一行 `certs`(标签「SSL 证书」,badge「E2E 加密」);
  `blocked()` 补 `(key === "certs" && !flags.keySync)`,表现与 keys 一致。
- `CertsScreen.tsx`:每张证书卡标出「本机自动续期」或「由设备 X 自动续期」,后者把「续期」
  改为接管语义。设备名取桶内 `manifest.devices`;拉不到退化显示 device id 前 8 位,不藏功能。
- `CertsScreen.tsx` 脚注「证书与私钥仅保存在本机」改为「账户密钥与证书随主机配置端到端加密同步」;
  证书范围关闭时回落到原话。

## 测试

- `sync_check` 加 step11(复用现有 A/B 设备与平台/MinIO,不引入新依赖):**不走 `acme.rs`**,
  直接往 A 写一张证书行,否则自检就依赖真实 CA 与活跃 SSH 会话。A 写入 → 同步 → B 校验 meta 与
  `cert_pem` 一致、`key_pem_enc` 能用 **B 的设备密钥**解开且明文与 A 相同、`owner_device_id == A`;
  B 接管 → 回同步 → A 看到归属变 B。
- 单测(无外部依赖):`add_column_if_missing` 幂等;「非归属设备不进续期」判定;
  `cert_payload` ↔ `apply_wire` 往返后私钥比特一致。
- 前端不额外加测 —— `pnpm test` 目前只覆盖 `nlDetect`,本次前端改动无逻辑分支。

## 与设备密钥迁移的关联

`apply_wire` 写入的证书密文落在 `certs.account_key_enc` / `key_pem_enc` —— 正是
`rewrap.rs` 曾漏扫、于 2026-09-20 修复的两列。同步接上后这条路径会被高频写入,
`rewrap` 的覆盖必须保持完整,否则「device.key 被删后密文解不开」会以更大规模复现。
`rewrap_check` 已覆盖这两列,不需新增用例,但改动 `rewrap` 或 `certs` 表时须同时确认。
