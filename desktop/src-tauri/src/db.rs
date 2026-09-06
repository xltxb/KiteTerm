use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::models::*;

pub struct Db {
    pub conn: Mutex<Connection>,
    pub data_dir: PathBuf,
}

/// certs 表完整行(含 pem/私钥等敏感字段,仅后端使用,不对外暴露)
#[derive(Debug, Clone)]
pub struct CertRow {
    pub id: String,
    pub domains: Vec<String>,
    pub challenge: String,
    pub ca: String,
    pub host_id: String,
    pub deploy_path: String,
    pub reload_cmd: String,
    pub not_after: i64,
    pub auto_renew: bool,
    pub state: String,
    pub account_key_enc: Option<Vec<u8>>,
    pub cert_pem: Option<String>,
    pub chain_pem: Option<String>,
    /// 证书私钥 PEM,经 vault 加密后落库(供重新部署到同主机/其他主机)
    pub key_pem_enc: Option<Vec<u8>>,
    pub created_at: String,
}

/// 本地变更日志条目(增量同步用)
#[derive(Debug, Clone)]
pub struct OplogRow {
    pub seq: i64,
    pub entity: String,
    pub entity_id: String,
    pub op: String,
    pub payload: Option<String>,
    pub lamport: i64,
    pub created_at: String,
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn group_payload(name: &str) -> String {
    serde_json::json!({ "name": name }).to_string()
}

impl Db {
    pub fn open(data_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        let conn = Connection::open(data_dir.join("kite.db"))?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS groups (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, sort INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS hosts (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, addr TEXT NOT NULL,
                port INTEGER NOT NULL DEFAULT 22, user TEXT NOT NULL,
                group_id TEXT NOT NULL, tags TEXT NOT NULL DEFAULT '[]',
                key_id TEXT, last_connected_at TEXT, sort INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS keys (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, algo TEXT NOT NULL,
                fingerprint TEXT NOT NULL, public_key TEXT NOT NULL,
                private_key_enc BLOB NOT NULL,
                sync_enabled INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS snippets (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, command TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]', use_count INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS forwards (
                id TEXT PRIMARY KEY, kind TEXT NOT NULL, local_port INTEGER NOT NULL,
                target TEXT NOT NULL DEFAULT '', via_host_id TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 0,
                bytes_up INTEGER NOT NULL DEFAULT 0, bytes_down INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS recordings (
                id TEXT PRIMARY KEY, session_host TEXT NOT NULL, name TEXT NOT NULL,
                path TEXT NOT NULL, duration_ms INTEGER NOT NULL DEFAULT 0,
                started_at TEXT NOT NULL, sync_enabled INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS kv (
                key TEXT PRIMARY KEY, value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS known_hosts (
                host_id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, first_seen TEXT NOT NULL
            );
            -- 增量同步:本地变更日志(payload 本地存明文 — 本地库中实体表本就是明文,
            -- 端到端加密发生在推送时;与文档 3.3 的 payload_enc 差异见 sync.rs 注释)
            CREATE TABLE IF NOT EXISTS oplog (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                entity TEXT NOT NULL, entity_id TEXT NOT NULL,
                op TEXT NOT NULL, payload TEXT,
                lamport INTEGER NOT NULL, created_at TEXT NOT NULL
            );
            -- 每台远端设备已拉取到的 seq 游标
            CREATE TABLE IF NOT EXISTS sync_cursors (
                device_id TEXT PRIMARY KEY, last_seq INTEGER NOT NULL
            );
            -- 实体级版本(LWW 依据):最后一次变更的 lamport 与来源设备
            CREATE TABLE IF NOT EXISTS sync_versions (
                entity TEXT NOT NULL, entity_id TEXT NOT NULL,
                lamport INTEGER NOT NULL, device_id TEXT NOT NULL, op TEXT NOT NULL,
                PRIMARY KEY (entity, entity_id)
            );
            -- ACME 证书:domains 以 JSON 文本存取(仿 hosts.tags);account_key_enc/cert_pem/chain_pem 为敏感列
            CREATE TABLE IF NOT EXISTS certs (
                id TEXT PRIMARY KEY, domains TEXT NOT NULL DEFAULT '[]',
                challenge TEXT NOT NULL, ca TEXT NOT NULL, host_id TEXT NOT NULL,
                deploy_path TEXT NOT NULL DEFAULT '', reload_cmd TEXT NOT NULL DEFAULT '',
                not_after INTEGER NOT NULL DEFAULT 0, auto_renew INTEGER NOT NULL DEFAULT 1,
                state TEXT NOT NULL DEFAULT 'active',
                account_key_enc BLOB, cert_pem TEXT, chain_pem TEXT, key_pem_enc BLOB,
                created_at TEXT NOT NULL
            );
            "#,
        )?;
        let db = Self { conn: Mutex::new(conn), data_dir };
        db.seed_groups()?;
        db.ensure_device_id()?;
        Ok(db)
    }

    fn ensure_device_id(&self) -> Result<()> {
        if self.kv_get("device_id")?.is_none() {
            self.kv_set("device_id", &uid())?;
        }
        Ok(())
    }

    pub fn device_id(&self) -> String {
        self.kv_get("device_id").ok().flatten().unwrap_or_default()
    }

    fn seed_groups(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM groups", [], |r| r.get(0))?;
        if n == 0 {
            for (i, name) in ["生产 PRODUCTION", "数据 DATA", "个人 PERSONAL"].iter().enumerate() {
                conn.execute(
                    "INSERT INTO groups (id, name, sort) VALUES (?1, ?2, ?3)",
                    params![uid(), name, i as i64],
                )?;
            }
        }
        Ok(())
    }

    // ---- kv ----
    pub fn kv_get(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn kv_del(&self, key: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM kv WHERE key = ?1", [key])?;
        Ok(())
    }

    /// 按前缀列出 kv 的键(设备密钥迁移时用来找出所有 `ai_key_*` 密文)
    pub fn kv_keys_with_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT key FROM kv WHERE key LIKE ?1 || '%'")?;
        let rows = stmt.query_map([prefix], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn kv_set(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    // ---- groups ----
    pub fn list_groups(&self) -> Result<Vec<HostGroup>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, name, sort FROM groups ORDER BY sort")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(HostGroup { id: r.get(0)?, name: r.get(1)?, sort: r.get(2)? })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 新建分组(同名幂等)。同步用分组名做 entity_id,跨设备天然一致。
    pub fn create_group(&self, name: &str) -> Result<HostGroup> {
        let name = name.trim();
        if name.is_empty() {
            return Err(crate::error::AppError("分组名不能为空".into()));
        }
        if let Some(g) = self.list_groups()?.into_iter().find(|g| g.name == name) {
            return Ok(g);
        }
        let id = uid();
        {
            let conn = self.conn.lock().unwrap();
            let max_sort: i64 =
                conn.query_row("SELECT COALESCE(MAX(sort),0) FROM groups", [], |r| r.get(0))?;
            conn.execute(
                "INSERT INTO groups (id, name, sort) VALUES (?1, ?2, ?3)",
                params![id, name, max_sort + 1],
            )?;
        }
        self.oplog_append("group", name, "put", Some(group_payload(name)))?;
        self.list_groups()?
            .into_iter()
            .find(|g| g.id == id)
            .ok_or_else(|| crate::error::AppError("分组创建后读取失败".into()))
    }

    pub fn rename_group(&self, id: &str, new_name: &str) -> Result<()> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err(crate::error::AppError("分组名不能为空".into()));
        }
        let groups = self.list_groups()?;
        let old = groups
            .iter()
            .find(|g| g.id == id)
            .ok_or_else(|| crate::error::AppError("分组不存在".into()))?;
        if old.name == new_name {
            return Ok(());
        }
        if groups.iter().any(|g| g.name == new_name) {
            return Err(crate::error::AppError(format!("已存在同名分组:{new_name}")));
        }
        let old_name = old.name.clone();
        {
            let conn = self.conn.lock().unwrap();
            conn.execute("UPDATE groups SET name = ?2 WHERE id = ?1", params![id, new_name])?;
        }
        // 顺序要紧:对端按 seq 依次应用,必须先建新组、再迁主机,最后才删空的旧组
        // ——否则删旧组时对端主机仍在其中,删除会被跳过而残留空分组
        self.oplog_append("group", new_name, "put", Some(group_payload(new_name)))?;
        for h in self.list_hosts()?.into_iter().filter(|h| h.group_id == id) {
            let payload = self.host_payload(&h)?;
            self.oplog_append("host", &h.id, "put", Some(payload))?;
        }
        self.oplog_append("group", &old_name, "del", None)?;
        Ok(())
    }

    /// 删除分组:组内主机迁到另一分组(不删主机);至少保留一个分组
    pub fn delete_group(&self, id: &str) -> Result<()> {
        let groups = self.list_groups()?;
        if groups.len() <= 1 {
            return Err(crate::error::AppError("至少保留一个分组".into()));
        }
        let target = groups
            .iter()
            .find(|g| g.id == id)
            .ok_or_else(|| crate::error::AppError("分组不存在".into()))?;
        let name = target.name.clone();
        let fallback = groups
            .iter()
            .find(|g| g.id != id)
            .expect("至少两个分组")
            .id
            .clone();
        let moved: Vec<String> = self
            .list_hosts()?
            .into_iter()
            .filter(|h| h.group_id == id)
            .map(|h| h.id)
            .collect();
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE hosts SET group_id = ?2 WHERE group_id = ?1",
                params![id, fallback],
            )?;
            conn.execute("DELETE FROM groups WHERE id = ?1", [id])?;
        }
        // 同 rename:先让对端把主机迁走,分组变空后删除才会生效
        let hosts = self.list_hosts()?;
        for host_id in moved {
            if let Some(h) = hosts.iter().find(|h| h.id == host_id) {
                let payload = self.host_payload(h)?;
                self.oplog_append("host", &h.id, "put", Some(payload))?;
            }
        }
        self.oplog_append("group", &name, "del", None)?;
        Ok(())
    }

    // ---- hosts ----
    pub fn list_hosts(&self) -> Result<Vec<Host>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, addr, port, user, group_id, tags, key_id, last_connected_at
             FROM hosts ORDER BY sort, name",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let tags: String = r.get(6)?;
                Ok(Host {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    addr: r.get(2)?,
                    port: r.get::<_, i64>(3)? as u16,
                    user: r.get(4)?,
                    group_id: r.get(5)?,
                    tags: serde_json::from_str(&tags).unwrap_or_default(),
                    key_id: r.get(7)?,
                    last_connected_at: r.get(8)?,
                    latency_ms: None,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_host(&self, id: &str) -> Result<Host> {
        self.list_hosts()?
            .into_iter()
            .find(|h| h.id == id)
            .ok_or_else(|| format!("主机不存在: {id}").into())
    }

    pub fn save_host(&self, input: HostInput) -> Result<Host> {
        let id = input.id.clone().unwrap_or_else(uid);
        let exists = input.id.is_some();
        let conn = self.conn.lock().unwrap();
        if exists {
            conn.execute(
                "UPDATE hosts SET name=?2, addr=COALESCE(?3, addr), port=COALESCE(?4, port),
                 user=COALESCE(?5, user), group_id=COALESCE(?6, group_id),
                 tags=COALESCE(?7, tags), key_id=?8, updated_at=?9 WHERE id=?1",
                params![
                    id,
                    input.name,
                    input.addr,
                    input.port.map(|p| p as i64),
                    input.user,
                    input.group_id,
                    input.tags.map(|t| serde_json::to_string(&t).unwrap()),
                    input.key_id,
                    now()
                ],
            )?;
        } else {
            let default_group: String = conn.query_row(
                "SELECT id FROM groups ORDER BY sort LIMIT 1",
                [],
                |r| r.get(0),
            )?;
            conn.execute(
                "INSERT INTO hosts (id, name, addr, port, user, group_id, tags, key_id, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id,
                    input.name,
                    input.addr.unwrap_or_default(),
                    input.port.unwrap_or(22) as i64,
                    input.user.unwrap_or_else(|| "root".into()),
                    input.group_id.unwrap_or(default_group),
                    serde_json::to_string(&input.tags.unwrap_or_default()).unwrap(),
                    input.key_id,
                    now()
                ],
            )?;
        }
        drop(conn);
        let host = self.get_host(&id)?;
        let payload = self.host_payload(&host)?;
        self.oplog_append("host", &id, "put", Some(payload))?;
        Ok(host)
    }

    pub fn delete_host(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM hosts WHERE id = ?1", [id])?;
        drop(conn);
        self.oplog_append("host", id, "del", None)?;
        Ok(())
    }

    pub fn touch_host_connected(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE hosts SET last_connected_at = ?2 WHERE id = ?1",
            params![id, now()],
        )?;
        Ok(())
    }

    // ---- keys ----
    pub fn list_keys(&self) -> Result<Vec<SshKeyMeta>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT k.id, k.name, k.algo, k.fingerprint, k.public_key, k.sync_enabled, k.created_at,
                    (SELECT COUNT(*) FROM hosts h WHERE h.key_id = k.id)
             FROM keys k ORDER BY k.created_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SshKeyMeta {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    algo: r.get(2)?,
                    fingerprint: r.get(3)?,
                    public_key: r.get(4)?,
                    sync_enabled: r.get::<_, i64>(5)? != 0,
                    created_at: r.get(6)?,
                    used_by_hosts: r.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_key(
        &self,
        name: &str,
        algo: &str,
        fingerprint: &str,
        public_key: &str,
        private_key_enc: &[u8],
    ) -> Result<String> {
        let id = uid();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO keys (id, name, algo, fingerprint, public_key, private_key_enc, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![id, name, algo, fingerprint, public_key, private_key_enc, now()],
        )?;
        Ok(id)
    }

    pub fn get_key_private_enc(&self, id: &str) -> Result<Vec<u8>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row(
            "SELECT private_key_enc FROM keys WHERE id = ?1",
            [id],
            |r| r.get(0),
        )?)
    }

    /// 只换私钥密文,不动其它列 —— 设备密钥迁移时重新加密用
    pub fn set_key_private_enc(&self, id: &str, blob: &[u8]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE keys SET private_key_enc = ?2 WHERE id = ?1", params![id, blob])?;
        Ok(())
    }

    pub fn get_key_public(&self, id: &str) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row("SELECT public_key FROM keys WHERE id = ?1", [id], |r| r.get(0))?)
    }

    pub fn set_key_sync(&self, id: &str, enabled: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE keys SET sync_enabled = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, enabled as i64, now()],
        )?;
        drop(conn);
        // 加入同步集 → put;移出(仅本地)→ 墓碑,其他设备删除该密钥
        if enabled {
            let payload = self.key_payload(id)?;
            self.oplog_append("key", id, "put", Some(payload))?;
        } else {
            self.oplog_append("key", id, "del", None)?;
        }
        Ok(())
    }

    pub fn delete_key(&self, id: &str) -> Result<()> {
        let was_synced = self.list_keys()?.iter().any(|k| k.id == id && k.sync_enabled);
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM keys WHERE id = ?1", [id])?;
        conn.execute("UPDATE hosts SET key_id = NULL WHERE key_id = ?1", [id])?;
        drop(conn);
        if was_synced {
            self.oplog_append("key", id, "del", None)?;
        }
        Ok(())
    }

    // ---- snippets ----
    pub fn list_snippets(&self) -> Result<Vec<Snippet>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, command, tags, use_count, updated_at FROM snippets ORDER BY use_count DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let tags: String = r.get(3)?;
                Ok(Snippet {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    command: r.get(2)?,
                    tags: serde_json::from_str(&tags).unwrap_or_default(),
                    use_count: r.get(4)?,
                    updated_at: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn save_snippet(&self, input: SnippetInput) -> Result<Snippet> {
        let id = input.id.clone().unwrap_or_else(uid);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO snippets (id, name, command, tags, use_count, updated_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5)
             ON CONFLICT(id) DO UPDATE SET name=?2, command=?3, tags=?4, updated_at=?5",
            params![
                id,
                input.name,
                input.command,
                serde_json::to_string(&input.tags.unwrap_or_default()).unwrap(),
                now()
            ],
        )?;
        drop(conn);
        let saved = self
            .list_snippets()?
            .into_iter()
            .find(|s| s.id == id)
            .expect("snippet just saved");
        self.oplog_append("snippet", &id, "put", Some(serde_json::to_string(&saved)?))?;
        Ok(saved)
    }

    pub fn bump_snippet(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE snippets SET use_count = use_count + 1 WHERE id = ?1", [id])?;
        drop(conn);
        if let Some(s) = self.list_snippets()?.into_iter().find(|s| s.id == id) {
            self.oplog_append("snippet", id, "put", Some(serde_json::to_string(&s)?))?;
        }
        Ok(())
    }

    pub fn delete_snippet(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM snippets WHERE id = ?1", [id])?;
        drop(conn);
        self.oplog_append("snippet", id, "del", None)?;
        Ok(())
    }

    // ---- forwards ----
    pub fn list_forwards(&self) -> Result<Vec<ForwardRule>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, kind, local_port, target, via_host_id, enabled, bytes_up, bytes_down
             FROM forwards ORDER BY updated_at",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ForwardRule {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    local_port: r.get::<_, i64>(2)? as u16,
                    target: r.get(3)?,
                    via_host_id: r.get(4)?,
                    enabled: r.get::<_, i64>(5)? != 0,
                    bytes_up: r.get::<_, i64>(6)? as u64,
                    bytes_down: r.get::<_, i64>(7)? as u64,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_forward(&self, input: &ForwardInput) -> Result<ForwardRule> {
        let id = uid();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO forwards (id, kind, local_port, target, via_host_id, enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)",
            params![id, input.kind, input.local_port as i64, input.target, input.via_host_id, now()],
        )?;
        drop(conn);
        let saved = self
            .list_forwards()?
            .into_iter()
            .find(|f| f.id == id)
            .expect("forward just saved");
        self.oplog_append("forward", &id, "put", Some(serde_json::to_string(&saved)?))?;
        Ok(saved)
    }

    pub fn set_forward_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE forwards SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, enabled as i64, now()],
        )?;
        drop(conn);
        if let Some(f) = self.list_forwards()?.into_iter().find(|f| f.id == id) {
            self.oplog_append("forward", id, "put", Some(serde_json::to_string(&f)?))?;
        }
        Ok(())
    }

    pub fn add_forward_traffic(&self, id: &str, up: u64, down: u64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE forwards SET bytes_up = bytes_up + ?2, bytes_down = bytes_down + ?3 WHERE id = ?1",
            params![id, up as i64, down as i64],
        )?;
        Ok(())
    }

    pub fn delete_forward(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM forwards WHERE id = ?1", [id])?;
        drop(conn);
        self.oplog_append("forward", id, "del", None)?;
        Ok(())
    }

    // ---- certs(ACME 证书)----
    pub fn list_certs(&self) -> Result<Vec<CertInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, domains, challenge, ca, host_id, deploy_path, reload_cmd,
                    not_after, auto_renew, state, created_at
             FROM certs ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let domains: String = r.get(1)?;
                Ok(CertInfo {
                    id: r.get(0)?,
                    domains: serde_json::from_str(&domains).unwrap_or_default(),
                    challenge: r.get(2)?,
                    ca: r.get(3)?,
                    host_id: r.get(4)?,
                    deploy_path: r.get(5)?,
                    reload_cmd: r.get(6)?,
                    not_after: r.get(7)?,
                    auto_renew: r.get::<_, i64>(8)? != 0,
                    state: r.get(9)?,
                    created_at: r.get(10)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_cert_full(&self, id: &str) -> Result<CertRow> {
        let conn = self.conn.lock().unwrap();
        let row = conn.query_row(
            "SELECT id, domains, challenge, ca, host_id, deploy_path, reload_cmd,
                    not_after, auto_renew, state, account_key_enc, cert_pem, chain_pem, key_pem_enc, created_at
             FROM certs WHERE id = ?1",
            [id],
            |r| {
                let domains: String = r.get(1)?;
                Ok(CertRow {
                    id: r.get(0)?,
                    domains: serde_json::from_str(&domains).unwrap_or_default(),
                    challenge: r.get(2)?,
                    ca: r.get(3)?,
                    host_id: r.get(4)?,
                    deploy_path: r.get(5)?,
                    reload_cmd: r.get(6)?,
                    not_after: r.get(7)?,
                    auto_renew: r.get::<_, i64>(8)? != 0,
                    state: r.get(9)?,
                    account_key_enc: r.get(10)?,
                    cert_pem: r.get(11)?,
                    chain_pem: r.get(12)?,
                    key_pem_enc: r.get(13)?,
                    created_at: r.get(14)?,
                })
            },
        )?;
        Ok(row)
    }

    pub fn upsert_cert(&self, row: &CertRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO certs (id, domains, challenge, ca, host_id, deploy_path, reload_cmd,
                                 not_after, auto_renew, state, account_key_enc, cert_pem, chain_pem, key_pem_enc, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id) DO UPDATE SET
                domains=?2, challenge=?3, ca=?4, host_id=?5, deploy_path=?6, reload_cmd=?7,
                not_after=?8, auto_renew=?9, state=?10, account_key_enc=?11, cert_pem=?12, chain_pem=?13, key_pem_enc=?14",
            params![
                row.id,
                serde_json::to_string(&row.domains).unwrap(),
                row.challenge,
                row.ca,
                row.host_id,
                row.deploy_path,
                row.reload_cmd,
                row.not_after,
                row.auto_renew as i64,
                row.state,
                row.account_key_enc,
                row.cert_pem,
                row.chain_pem,
                row.key_pem_enc,
                row.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_cert(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM certs WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---- oplog(增量同步,文档 3.5)----

    /// 记录一次本地变更:lamport 递增、写 oplog、更新实体版本
    pub fn oplog_append(
        &self,
        entity: &str,
        entity_id: &str,
        op: &str,
        payload: Option<String>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let lamport: i64 = conn
            .query_row("SELECT value FROM kv WHERE key = 'lamport'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
            + 1;
        conn.execute(
            "INSERT INTO kv (key, value) VALUES ('lamport', ?1)
             ON CONFLICT(key) DO UPDATE SET value = ?1",
            [lamport.to_string()],
        )?;
        let device: String = conn
            .query_row("SELECT value FROM kv WHERE key = 'device_id'", [], |r| r.get(0))
            .optional()?
            .unwrap_or_default();
        conn.execute(
            "INSERT INTO oplog (entity, entity_id, op, payload, lamport, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![entity, entity_id, op, payload, lamport, now()],
        )?;
        conn.execute(
            "INSERT INTO sync_versions (entity, entity_id, lamport, device_id, op)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(entity, entity_id) DO UPDATE SET lamport=?3, device_id=?4, op=?5",
            params![entity, entity_id, lamport, device, op],
        )?;
        Ok(lamport)
    }

    pub fn oplog_after(&self, seq: i64) -> Result<Vec<OplogRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT seq, entity, entity_id, op, payload, lamport, created_at
             FROM oplog WHERE seq > ?1 ORDER BY seq",
        )?;
        let rows = stmt
            .query_map([seq], |r| {
                Ok(OplogRow {
                    seq: r.get(0)?,
                    entity: r.get(1)?,
                    entity_id: r.get(2)?,
                    op: r.get(3)?,
                    payload: r.get(4)?,
                    lamport: r.get(5)?,
                    created_at: r.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn cursor_get(&self, device_id: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT last_seq FROM sync_cursors WHERE device_id = ?1",
                [device_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    pub fn cursor_set(&self, device_id: &str, seq: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sync_cursors (device_id, last_seq) VALUES (?1, ?2)
             ON CONFLICT(device_id) DO UPDATE SET last_seq = ?2",
            params![device_id, seq],
        )?;
        Ok(())
    }

    /// LWW 判定:远端条目是否应当应用(lamport 更大,持平比 device_id);
    /// 应用则同步记录新版本并推进本地 lamport 时钟
    pub fn version_accept_remote(
        &self,
        entity: &str,
        entity_id: &str,
        lamport: i64,
        device_id: &str,
        op: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let cur: Option<(i64, String)> = conn
            .query_row(
                "SELECT lamport, device_id FROM sync_versions WHERE entity = ?1 AND entity_id = ?2",
                params![entity, entity_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let accept = match cur {
            None => true,
            Some((l, d)) => (lamport, device_id) > (l, d.as_str()),
        };
        if accept {
            conn.execute(
                "INSERT INTO sync_versions (entity, entity_id, lamport, device_id, op)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(entity, entity_id) DO UPDATE SET lamport=?3, device_id=?4, op=?5",
                params![entity, entity_id, lamport, device_id, op],
            )?;
            // lamport 时钟对齐:local = max(local, remote)
            let local: i64 = conn
                .query_row("SELECT value FROM kv WHERE key = 'lamport'", [], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if lamport > local {
                conn.execute(
                    "INSERT INTO kv (key, value) VALUES ('lamport', ?1)
                     ON CONFLICT(key) DO UPDATE SET value = ?1",
                    [lamport.to_string()],
                )?;
            }
        }
        Ok(accept)
    }

    /// 当前 lamport 时钟值
    pub fn lamport_now(&self) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row("SELECT value FROM kv WHERE key = 'lamport'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }

    /// 全量实体版本(压实快照用):entity, entity_id, lamport, device_id, op
    pub fn list_sync_versions(&self) -> Result<Vec<(String, String, i64, String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT entity, entity_id, lamport, device_id, op FROM sync_versions ORDER BY entity, entity_id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 首次同步:为存量实体合成 put 条目,保证老数据也能进入 oplog 流
    pub fn bootstrap_oplog(&self) -> Result<()> {
        if self.kv_get("oplog_bootstrapped")?.is_some() {
            return Ok(());
        }
        for h in self.list_hosts()? {
            let payload = self.host_payload(&h)?;
            self.oplog_append("host", &h.id.clone(), "put", Some(payload))?;
        }
        for s in self.list_snippets()? {
            self.oplog_append("snippet", &s.id.clone(), "put", Some(serde_json::to_string(&s)?))?;
        }
        for f in self.list_forwards()? {
            self.oplog_append("forward", &f.id.clone(), "put", Some(serde_json::to_string(&f)?))?;
        }
        for k in self.list_keys()? {
            if k.sync_enabled {
                let payload = self.key_payload(&k.id)?;
                self.oplog_append("key", &k.id, "put", Some(payload))?;
            }
        }
        self.kv_set("oplog_bootstrapped", "1")?;
        Ok(())
    }

    /// host 实体的 oplog payload:实体 + 分组名(跨设备按名映射分组)
    pub fn host_payload(&self, h: &Host) -> Result<String> {
        let group_name = self
            .list_groups()?
            .into_iter()
            .find(|g| g.id == h.group_id)
            .map(|g| g.name)
            .unwrap_or_default();
        Ok(serde_json::to_string(&serde_json::json!({ "host": h, "groupName": group_name }))?)
    }

    /// key 实体的 oplog payload:元数据 + 设备密钥加密的私钥(推送时换主密钥加密)
    pub fn key_payload(&self, key_id: &str) -> Result<String> {
        use base64::Engine;
        let meta = self
            .list_keys()?
            .into_iter()
            .find(|k| k.id == key_id)
            .ok_or_else(|| crate::error::AppError(format!("密钥不存在: {key_id}")))?;
        let enc = self.get_key_private_enc(key_id)?;
        Ok(serde_json::to_string(&serde_json::json!({
            "meta": meta,
            "privateDevEnc": base64::engine::general_purpose::STANDARD.encode(enc),
        }))?)
    }

    /// 按名取或建分组(应用远端 host 时用)
    pub fn group_by_name_or_create(&self, name: &str) -> Result<String> {
        if name.is_empty() {
            let conn = self.conn.lock().unwrap();
            return Ok(conn.query_row("SELECT id FROM groups ORDER BY sort LIMIT 1", [], |r| r.get(0))?);
        }
        if let Some(g) = self.list_groups()?.into_iter().find(|g| g.name == name) {
            return Ok(g.id);
        }
        let id = uid();
        let conn = self.conn.lock().unwrap();
        let max_sort: i64 = conn.query_row("SELECT COALESCE(MAX(sort),0) FROM groups", [], |r| r.get(0))?;
        conn.execute(
            "INSERT INTO groups (id, name, sort) VALUES (?1, ?2, ?3)",
            params![id, name, max_sort + 1],
        )?;
        Ok(id)
    }

    // ---- 远端条目应用(不追加 oplog,不递增 lamport)----

    pub fn upsert_host_raw(&self, h: &Host) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO hosts (id, name, addr, port, user, group_id, tags, key_id, last_connected_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET name=?2, addr=?3, port=?4, user=?5, group_id=?6, tags=?7, key_id=?8",
            params![
                h.id, h.name, h.addr, h.port as i64, h.user, h.group_id,
                serde_json::to_string(&h.tags).unwrap(), h.key_id, h.last_connected_at, now()
            ],
        )?;
        Ok(())
    }

    pub fn upsert_snippet_raw(&self, s: &Snippet) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO snippets (id, name, command, tags, use_count, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET name=?2, command=?3, tags=?4, use_count=?5, updated_at=?6",
            params![s.id, s.name, s.command, serde_json::to_string(&s.tags).unwrap(), s.use_count, s.updated_at],
        )?;
        Ok(())
    }

    pub fn upsert_forward_raw(&self, f: &ForwardRule) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO forwards (id, kind, local_port, target, via_host_id, enabled, bytes_up, bytes_down, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, ?7)
             ON CONFLICT(id) DO UPDATE SET kind=?2, local_port=?3, target=?4, via_host_id=?5, enabled=?6, updated_at=?7",
            params![f.id, f.kind, f.local_port as i64, f.target, f.via_host_id, f.enabled as i64, now()],
        )?;
        Ok(())
    }

    /// 应用远端密钥:同 id 覆盖;同指纹不同 id 视为已存在,跳过
    pub fn upsert_key_raw(&self, meta: &SshKeyMeta, private_key_enc: &[u8]) -> Result<()> {
        let existing = self.list_keys()?;
        if existing.iter().any(|k| k.fingerprint == meta.fingerprint && k.id != meta.id) {
            return Ok(());
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO keys (id, name, algo, fingerprint, public_key, private_key_enc, sync_enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET name=?2, algo=?3, fingerprint=?4, public_key=?5, private_key_enc=?6, sync_enabled=1, updated_at=?8",
            params![meta.id, meta.name, meta.algo, meta.fingerprint, meta.public_key, private_key_enc, meta.created_at, now()],
        )?;
        Ok(())
    }

    pub fn delete_entity_raw(&self, entity: &str, entity_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        match entity {
            "host" => { conn.execute("DELETE FROM hosts WHERE id = ?1", [entity_id])?; }
            "snippet" => { conn.execute("DELETE FROM snippets WHERE id = ?1", [entity_id])?; }
            "forward" => { conn.execute("DELETE FROM forwards WHERE id = ?1", [entity_id])?; }
            "key" => {
                conn.execute("DELETE FROM keys WHERE id = ?1", [entity_id])?;
                conn.execute("UPDATE hosts SET key_id = NULL WHERE key_id = ?1", [entity_id])?;
            }
            // 分组以名字为 entity_id;仅删空分组,避免把本机仍在用的分组删掉
            "group" => {
                conn.execute(
                    "DELETE FROM groups WHERE name = ?1
                     AND id NOT IN (SELECT DISTINCT group_id FROM hosts)
                     AND (SELECT COUNT(*) FROM groups) > 1",
                    [entity_id],
                )?;
            }
            _ => {}
        }
        Ok(())
    }

    // ---- known_hosts(TOFU 指纹)----
    pub fn known_host_fingerprint(&self, host_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT fingerprint FROM known_hosts WHERE host_id = ?1",
                [host_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn record_known_host(&self, host_id: &str, fingerprint: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO known_hosts (host_id, fingerprint, first_seen) VALUES (?1, ?2, ?3)
             ON CONFLICT(host_id) DO UPDATE SET fingerprint = ?2, first_seen = ?3",
            params![host_id, fingerprint, now()],
        )?;
        Ok(())
    }

    pub fn forget_known_host(&self, host_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM known_hosts WHERE host_id = ?1", [host_id])?;
        Ok(())
    }

    // ---- recordings ----
    pub fn list_recordings(&self) -> Result<Vec<Recording>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, session_host, name, duration_ms, started_at, sync_enabled
             FROM recordings ORDER BY started_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Recording {
                    id: r.get(0)?,
                    session_host: r.get(1)?,
                    name: r.get(2)?,
                    duration_ms: r.get::<_, i64>(3)? as u64,
                    started_at: r.get(4)?,
                    sync_enabled: r.get::<_, i64>(5)? != 0,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_recording_path(&self, id: &str) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row("SELECT path FROM recordings WHERE id = ?1", [id], |r| r.get(0))?)
    }

    pub fn insert_recording(&self, session_host: &str, name: &str, path: &str, duration_ms: u64) -> Result<String> {
        let id = uid();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO recordings (id, session_host, name, path, duration_ms, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, session_host, name, path, duration_ms as i64, now()],
        )?;
        Ok(id)
    }

    pub fn set_recording_sync(&self, id: &str, enabled: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE recordings SET sync_enabled = ?2 WHERE id = ?1", params![id, enabled as i64])?;
        Ok(())
    }

    pub fn recording_exists(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row("SELECT 1 FROM recordings WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
    }

    /// 从云端拉下来的录制:沿用远端 id,标记为已同步
    pub fn insert_recording_with_id(&self, id: &str, session_host: &str, name: &str, path: &str, duration_ms: u64, started_at: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO recordings (id, session_host, name, path, duration_ms, started_at, sync_enabled)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
            params![id, session_host, name, path, duration_ms as i64, started_at],
        )?;
        Ok(())
    }

    /// (id, path, sync_enabled) —— 同步上传用
    pub fn recording_paths(&self) -> Result<Vec<(String, String, bool)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, path, sync_enabled FROM recordings")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
