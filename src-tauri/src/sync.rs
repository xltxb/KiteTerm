//! 云同步 v2:oplog 增量同步(文档 3.5)。
//!
//! 桶内布局:
//!   manifest.json                 设备清单(明文 JSON;文档要求密文+If-Match,标记为后续)
//!   oplog/{device_id}/{seq}.bin   逐条 XChaCha20-Poly1305 密文,AAD 绑定对象路径防重放/错位;
//!                                 seq 零填充保证 LIST 字典序 == 数值序;append-only
//!
//! 拉取:LIST 其他设备前缀,按本地游标(sync_cursors)增量下载;
//! 合并:实体级 LWW —— lamport 时钟决胜,持平比 device_id;删除为墓碑条目。
//! 压实(30 天前 oplog → snapshot)与墓碑清理:后续版本。

use std::sync::Arc;

use argon2::Argon2;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::XChaCha20Poly1305;
use s3::creds::Credentials;
use s3::{Bucket, Region};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::db::{Db, OplogRow};
use crate::error::{AppError, Result};
use crate::models::*;
use crate::vault::Vault;

const KV_CONFIG: &str = "sync_config";
const KV_SECRET: &str = "sync_secret_enc";
const KV_MK: &str = "sync_mk_enc";
const KV_LAST: &str = "sync_last_at";
const KV_PUSHED: &str = "sync_pushed_seq";

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

#[derive(Serialize, Deserialize, Default)]
struct Manifest {
    devices: Vec<SyncDevice>,
    updated_at: String,
}

/// 桶内 oplog 条目的明文结构(整体加密后落桶)
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireOp {
    entity: String,
    entity_id: String,
    op: String,
    payload: Option<serde_json::Value>,
    lamport: i64,
    device_id: String,
    created_at: String,
}

pub fn get_config(db: &Db) -> Result<SyncConfig> {
    match db.kv_get(KV_CONFIG)? {
        Some(json) => Ok(serde_json::from_str(&json)?),
        None => Ok(SyncConfig {
            endpoint: String::new(),
            bucket: String::new(),
            region: "auto".into(),
            access_key: String::new(),
            has_secret: false,
            scopes: SyncScopes { hosts: true, keys: true, snippets: true, settings: false },
            secret_key: None,
            master_password: None,
        }),
    }
}

pub fn save_config(db: &Db, vault: &Vault, mut cfg: SyncConfig) -> Result<()> {
    let old = get_config(db)?;
    if let Some(secret) = cfg.secret_key.take() {
        if !secret.is_empty() {
            db.kv_set(KV_SECRET, &b64().encode(vault.encrypt(secret.as_bytes())?))?;
            cfg.has_secret = true;
        }
    }
    if let Some(pw) = cfg.master_password.take() {
        if !pw.is_empty() {
            // Argon2id 派生主密钥;盐与桶绑定(endpoint+bucket),换桶即换密钥空间
            let salt = format!("vela-sync:{}:{}", cfg.endpoint, cfg.bucket);
            let mut mk = [0u8; 32];
            Argon2::default()
                .hash_password_into(pw.as_bytes(), salt.as_bytes(), &mut mk)
                .map_err(|e| AppError(format!("派生主密钥失败: {e}")))?;
            db.kv_set(KV_MK, &b64().encode(vault.encrypt(&mk)?))?;
        }
    }
    db.kv_set(KV_CONFIG, &serde_json::to_string(&cfg)?)?;

    // 分项开关 关→开:为该类存量实体补发 put 条目,否则老数据进不了增量流
    if cfg.scopes.hosts && !old.scopes.hosts {
        for h in db.list_hosts()? {
            let p = db.host_payload(&h)?;
            db.oplog_append("host", &h.id.clone(), "put", Some(p))?;
        }
        for f in db.list_forwards()? {
            db.oplog_append("forward", &f.id.clone(), "put", Some(serde_json::to_string(&f)?))?;
        }
    }
    if cfg.scopes.snippets && !old.scopes.snippets {
        for s in db.list_snippets()? {
            db.oplog_append("snippet", &s.id.clone(), "put", Some(serde_json::to_string(&s)?))?;
        }
    }
    if cfg.scopes.keys && !old.scopes.keys {
        for k in db.list_keys()? {
            if k.sync_enabled {
                let p = db.key_payload(&k.id)?;
                db.oplog_append("key", &k.id, "put", Some(p))?;
            }
        }
    }
    if cfg.scopes.settings && !old.scopes.settings {
        if let Some(s) = db.kv_get("settings")? {
            db.oplog_append("settings", "app", "put", Some(s))?;
        }
    }
    Ok(())
}

pub fn get_status(db: &Db) -> Result<SyncStatus> {
    let cfg = get_config(db)?;
    let configured = !cfg.endpoint.is_empty() && !cfg.bucket.is_empty() && cfg.has_secret;
    let devices = db
        .kv_get("sync_manifest")?
        .and_then(|m| serde_json::from_str::<Manifest>(&m).ok())
        .map(|m| m.devices)
        .unwrap_or_default();
    let me = db.device_id();
    let devices = devices
        .into_iter()
        .map(|mut d| {
            d.current = d.id == me;
            d
        })
        .collect();
    Ok(SyncStatus {
        configured,
        syncing: false,
        last_sync_at: db.kv_get(KV_LAST)?,
        devices,
    })
}

fn open_bucket(db: &Db, vault: &Vault) -> Result<(Box<Bucket>, SyncConfig)> {
    let cfg = get_config(db)?;
    if cfg.endpoint.is_empty() || cfg.bucket.is_empty() {
        return Err(AppError("尚未配置同步存储".into()));
    }
    let secret_enc = db
        .kv_get(KV_SECRET)?
        .ok_or_else(|| AppError("缺少 Secret Key".into()))?;
    let secret = String::from_utf8(
        vault.decrypt(&b64().decode(secret_enc).map_err(|e| AppError(e.to_string()))?)?,
    )
    .map_err(|_| AppError("Secret 解码失败".into()))?;
    let region = Region::Custom { region: cfg.region.clone(), endpoint: cfg.endpoint.clone() };
    let creds = Credentials::new(Some(&cfg.access_key), Some(&secret), None, None, None)
        .map_err(|e| AppError(format!("凭据错误: {e}")))?;
    let bucket = Bucket::new(&cfg.bucket, region, creds)
        .map_err(|e| AppError(format!("bucket 初始化失败: {e}")))?
        .with_path_style();
    Ok((bucket, cfg))
}

fn master_cipher(db: &Db, vault: &Vault) -> Result<XChaCha20Poly1305> {
    let mk_enc = db
        .kv_get(KV_MK)?
        .ok_or_else(|| AppError("尚未设置主密码 — 在同步配置中输入主密码".into()))?;
    let mk = vault.decrypt(&b64().decode(mk_enc).map_err(|e| AppError(e.to_string()))?)?;
    let mk: [u8; 32] = mk.try_into().map_err(|_| AppError("主密钥损坏".into()))?;
    Ok(XChaCha20Poly1305::new((&mk).into()))
}

fn seal(cipher: &XChaCha20Poly1305, plain: &[u8], aad: &str) -> Result<Vec<u8>> {
    use chacha20poly1305::aead::AeadCore;
    let nonce = XChaCha20Poly1305::generate_nonce(&mut chacha20poly1305::aead::OsRng);
    let ct = cipher
        .encrypt(&nonce, Payload { msg: plain, aad: aad.as_bytes() })
        .map_err(|e| AppError(format!("加密失败: {e}")))?;
    let mut out = nonce.to_vec();
    out.extend(ct);
    Ok(out)
}

fn unseal(cipher: &XChaCha20Poly1305, blob: &[u8], aad: &str) -> Result<Vec<u8>> {
    if blob.len() < 24 {
        return Err(AppError("密文格式错误".into()));
    }
    let (nonce, ct) = blob.split_at(24);
    cipher
        .decrypt(
            chacha20poly1305::XNonce::from_slice(nonce),
            Payload { msg: ct, aad: aad.as_bytes() },
        )
        .map_err(|_| AppError("解密失败 — 主密码不一致或对象被移动?".into()))
}

fn scope_allows(cfg: &SyncConfig, entity: &str) -> bool {
    match entity {
        "host" | "forward" => cfg.scopes.hosts,
        "key" => cfg.scopes.keys,
        "snippet" => cfg.scopes.snippets,
        "settings" => cfg.scopes.settings,
        _ => false,
    }
}

fn oplog_object_key(device: &str, seq: i64) -> String {
    format!("oplog/{device}/{seq:010}.bin")
}

/// 本地 oplog 条目 → 桶内明文结构(key 实体把设备密钥层换成明文,随后整体主密钥加密)
fn to_wire(vault: &Vault, me: &str, row: &OplogRow) -> Result<WireOp> {
    let mut payload: Option<serde_json::Value> =
        row.payload.as_deref().map(serde_json::from_str).transpose()?;
    if row.entity == "key" && row.op == "put" {
        if let Some(p) = payload.as_mut() {
            let dev_enc = p["privateDevEnc"]
                .as_str()
                .ok_or_else(|| AppError("key payload 缺少私钥".into()))?;
            let pem = vault.decrypt(
                &b64().decode(dev_enc).map_err(|e| AppError(e.to_string()))?,
            )?;
            p["privatePem"] = serde_json::Value::String(
                String::from_utf8(pem).map_err(|_| AppError("私钥解码失败".into()))?,
            );
            p.as_object_mut().unwrap().remove("privateDevEnc");
        }
    }
    Ok(WireOp {
        entity: row.entity.clone(),
        entity_id: row.entity_id.clone(),
        op: row.op.clone(),
        payload,
        lamport: row.lamport,
        device_id: me.to_string(),
        created_at: row.created_at.clone(),
    })
}

/// 应用一条远端条目(LWW 判定已通过)
fn apply_wire(db: &Db, vault: &Vault, wire: &WireOp) -> Result<()> {
    if wire.op == "del" {
        return db.delete_entity_raw(&wire.entity, &wire.entity_id);
    }
    let payload = wire
        .payload
        .as_ref()
        .ok_or_else(|| AppError("put 条目缺少 payload".into()))?;
    match wire.entity.as_str() {
        "host" => {
            let mut host: Host = serde_json::from_value(payload["host"].clone())?;
            let group_name = payload["groupName"].as_str().unwrap_or_default();
            host.group_id = db.group_by_name_or_create(group_name)?;
            db.upsert_host_raw(&host)?;
        }
        "snippet" => {
            let s: Snippet = serde_json::from_value(payload.clone())?;
            db.upsert_snippet_raw(&s)?;
        }
        "forward" => {
            let f: ForwardRule = serde_json::from_value(payload.clone())?;
            db.upsert_forward_raw(&f)?;
        }
        "key" => {
            let meta: SshKeyMeta = serde_json::from_value(payload["meta"].clone())?;
            let pem = payload["privatePem"]
                .as_str()
                .ok_or_else(|| AppError("key 条目缺少私钥".into()))?;
            let dev_enc = vault.encrypt(pem.as_bytes())?;
            db.upsert_key_raw(&meta, &dev_enc)?;
        }
        "settings" => {
            db.kv_set("settings", &payload.to_string())?;
        }
        other => return Err(AppError(format!("未知实体类型: {other}"))),
    }
    Ok(())
}

pub async fn sync_now(app: AppHandle, db: Arc<Db>, vault: Arc<Vault>) -> Result<()> {
    let emit = |syncing: bool| {
        let mut status = get_status(&db).unwrap_or(SyncStatus {
            configured: true,
            syncing,
            last_sync_at: None,
            devices: vec![],
        });
        status.syncing = syncing;
        let _ = app.emit("sync://state", status);
    };
    emit(true);
    let result = sync_once(&db, &vault).await;
    if result.is_ok() {
        db.kv_set(KV_LAST, &chrono::Local::now().format("%H:%M").to_string())?;
    }
    emit(false);
    result
}

/// 无 UI 依赖的一次完整推拉(集成测试亦复用)
pub async fn sync_once(db: &Db, vault: &Vault) -> Result<()> {
    let (bucket, cfg) = open_bucket(db, vault)?;
    let cipher = master_cipher(db, vault)?;
    let me = db.device_id();

    db.bootstrap_oplog()?;

    // ---- push:未推送的本地条目,逐条加密上传(append-only,幂等)----
    let pushed: i64 = db
        .kv_get(KV_PUSHED)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    for row in db.oplog_after(pushed)? {
        if scope_allows(&cfg, &row.entity) {
            let wire = to_wire(vault, &me, &row)?;
            let key = oplog_object_key(&me, row.seq);
            let blob = seal(&cipher, &serde_json::to_vec(&wire)?, &key)?;
            bucket
                .put_object(&key, &blob)
                .await
                .map_err(|e| AppError(format!("上传 {key} 失败: {e}")))?;
        }
        db.kv_set(KV_PUSHED, &row.seq.to_string())?;
    }

    // ---- manifest:设备清单(读-改-写)----
    let mut manifest: Manifest = match bucket.get_object("manifest.json").await {
        Ok(resp) => serde_json::from_slice(resp.bytes()).unwrap_or_default(),
        Err(_) => Manifest::default(),
    };
    let host_name = whoami::fallible::hostname().unwrap_or_else(|_| "此设备".into());
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    manifest.devices.retain(|d| d.id != me);
    manifest.devices.push(SyncDevice {
        id: me.clone(),
        name: host_name,
        kind: "laptop".into(),
        last_seen_at: now.clone(),
        current: false,
    });
    manifest.updated_at = now;
    bucket
        .put_object("manifest.json", &serde_json::to_vec(&manifest)?)
        .await
        .map_err(|e| AppError(format!("上传 manifest 失败: {e}")))?;
    db.kv_set("sync_manifest", &serde_json::to_string(&manifest)?)?;

    // ---- pull:其他设备前缀,按游标增量下载并 LWW 合并 ----
    let listing = bucket
        .list("oplog/".into(), None)
        .await
        .map_err(|e| AppError(format!("列举 oplog 失败: {e}")))?;
    // (device, seq, object_key),仅取游标之后的
    let mut pending: Vec<(String, i64, String)> = Vec::new();
    for page in listing {
        for obj in page.contents {
            let parts: Vec<&str> = obj.key.split('/').collect();
            if parts.len() != 3 || parts[0] != "oplog" {
                continue;
            }
            let dev = parts[1].to_string();
            if dev == me {
                continue;
            }
            let Ok(seq) = parts[2].trim_end_matches(".bin").parse::<i64>() else { continue };
            if seq > db.cursor_get(&dev)? {
                pending.push((dev, seq, obj.key.clone()));
            }
        }
    }
    // 同设备内按 seq 升序应用
    pending.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    for (dev, seq, key) in pending {
        let resp = bucket
            .get_object(&key)
            .await
            .map_err(|e| AppError(format!("下载 {key} 失败: {e}")))?;
        match unseal(&cipher, resp.bytes(), &key)
            .and_then(|plain| serde_json::from_slice::<WireOp>(&plain).map_err(Into::into))
        {
            Ok(wire) => {
                if scope_allows(&cfg, &wire.entity)
                    && db.version_accept_remote(
                        &wire.entity,
                        &wire.entity_id,
                        wire.lamport,
                        &wire.device_id,
                        &wire.op,
                    )?
                {
                    apply_wire(db, vault, &wire)?;
                }
            }
            Err(e) => eprintln!("[sync] 跳过损坏条目 {key}: {e}"),
        }
        db.cursor_set(&dev, seq)?;
    }
    Ok(())
}
