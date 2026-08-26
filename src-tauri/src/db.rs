use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::models::*;

pub struct Db {
    pub conn: Mutex<Connection>,
    pub data_dir: PathBuf,
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl Db {
    pub fn open(data_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        let conn = Connection::open(data_dir.join("vela.db"))?;
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
            "#,
        )?;
        let db = Self { conn: Mutex::new(conn), data_dir };
        db.seed_groups()?;
        Ok(db)
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
        self.get_host(&id)
    }

    pub fn delete_host(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM hosts WHERE id = ?1", [id])?;
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
        Ok(())
    }

    pub fn delete_key(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM keys WHERE id = ?1", [id])?;
        conn.execute("UPDATE hosts SET key_id = NULL WHERE key_id = ?1", [id])?;
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
        Ok(self
            .list_snippets()?
            .into_iter()
            .find(|s| s.id == id)
            .expect("snippet just saved"))
    }

    pub fn bump_snippet(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE snippets SET use_count = use_count + 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn delete_snippet(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM snippets WHERE id = ?1", [id])?;
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
        Ok(self
            .list_forwards()?
            .into_iter()
            .find(|f| f.id == id)
            .expect("forward just saved"))
    }

    pub fn set_forward_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE forwards SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, enabled as i64, now()],
        )?;
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
}
