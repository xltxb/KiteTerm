//! 云同步 v1:S3 兼容桶 + 端到端加密快照。
//! 每设备推送 snapshot/{device_id}.bin(XChaCha20-Poly1305,密钥由主密码 Argon2id 派生);
//! 拉取其他设备快照按实体 LWW(updated_at)合并。oplog 增量与压实见开发文档 3.5,后续版本实现。

use std::sync::Arc;

use argon2::Argon2;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::XChaCha20Poly1305;
use s3::creds::Credentials;
use s3::{Bucket, Region};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::*;
use crate::vault::Vault;

const KV_CONFIG: &str = "sync_config";
const KV_SECRET: &str = "sync_secret_enc";
const KV_MK: &str = "sync_mk_enc";
const KV_LAST: &str = "sync_last_at";
const KV_DEVICE: &str = "device_id";

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

#[derive(Serialize, Deserialize, Default)]
struct Manifest {
    devices: Vec<SyncDevice>,
    updated_at: String,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    device_id: String,
    created_at: String,
    hosts: Option<Vec<Host>>,
    groups: Option<Vec<HostGroup>>,
    snippets: Option<Vec<Snippet>>,
    keys: Option<Vec<SnapshotKey>>,
    settings: Option<AppSettings>,
}

#[derive(Serialize, Deserialize)]
struct SnapshotKey {
    meta: SshKeyMeta,
    /// 主密码派生密钥加密后的私钥 PEM(base64)
    private_pem_enc: String,
}

pub fn device_id(db: &Db) -> Result<String> {
    if let Some(id) = db.kv_get(KV_DEVICE)? {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    db.kv_set(KV_DEVICE, &id)?;
    Ok(id)
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
    let me = device_id(db)?;
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

fn seal(cipher: &XChaCha20Poly1305, plain: &[u8]) -> Result<Vec<u8>> {
    use chacha20poly1305::aead::AeadCore;
    let nonce = XChaCha20Poly1305::generate_nonce(&mut chacha20poly1305::aead::OsRng);
    let ct = cipher.encrypt(&nonce, plain).map_err(|e| AppError(format!("加密失败: {e}")))?;
    let mut out = nonce.to_vec();
    out.extend(ct);
    Ok(out)
}

fn unseal(cipher: &XChaCha20Poly1305, blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < 24 {
        return Err(AppError("密文格式错误".into()));
    }
    let (nonce, ct) = blob.split_at(24);
    cipher
        .decrypt(chacha20poly1305::XNonce::from_slice(nonce), ct)
        .map_err(|_| AppError("解密失败 — 主密码不一致?".into()))
}

pub async fn sync_now(app: AppHandle, db: Arc<Db>, vault: Arc<Vault>) -> Result<()> {
    let emit = |syncing: bool, last: Option<String>| {
        let devices = get_status(&db).map(|s| s.devices).unwrap_or_default();
        let _ = app.emit(
            "sync://state",
            SyncStatus { configured: true, syncing, last_sync_at: last, devices },
        );
    };
    emit(true, db.kv_get(KV_LAST)?);

    let result = do_sync(&db, &vault).await;
    let now = chrono::Local::now().format("%H:%M").to_string();
    if result.is_ok() {
        db.kv_set(KV_LAST, &now)?;
    }
    emit(false, db.kv_get(KV_LAST)?);
    result
}

async fn do_sync(db: &Db, vault: &Vault) -> Result<()> {
    let (bucket, cfg) = open_bucket(db, vault)?;
    let cipher = master_cipher(db, vault)?;
    let me = device_id(db)?;

    // ---- push:本机快照 ----
    let snapshot = build_snapshot(db, vault, &cipher, &cfg, &me)?;
    let blob = seal(&cipher, &serde_json::to_vec(&snapshot)?)?;
    bucket
        .put_object(format!("snapshot/{me}.bin"), &blob)
        .await
        .map_err(|e| AppError(format!("上传快照失败: {e}")))?;

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

    // ---- pull:其他设备快照,LWW 合并 ----
    let listing = bucket
        .list("snapshot/".into(), None)
        .await
        .map_err(|e| AppError(format!("列举快照失败: {e}")))?;
    for page in listing {
        for obj in page.contents {
            if obj.key == format!("snapshot/{me}.bin") {
                continue;
            }
            let Ok(resp) = bucket.get_object(&obj.key).await else { continue };
            let Ok(plain) = unseal(&cipher, resp.bytes()) else { continue };
            let Ok(remote) = serde_json::from_slice::<Snapshot>(&plain) else { continue };
            merge_snapshot(db, vault, &cipher, remote)?;
        }
    }
    Ok(())
}

fn build_snapshot(
    db: &Db,
    vault: &Vault,
    cipher: &XChaCha20Poly1305,
    cfg: &SyncConfig,
    me: &str,
) -> Result<Snapshot> {
    let keys = if cfg.scopes.keys {
        let mut out = Vec::new();
        for meta in db.list_keys()?.into_iter().filter(|k| k.sync_enabled) {
            let pem = vault.decrypt(&db.get_key_private_enc(&meta.id)?)?;
            out.push(SnapshotKey {
                meta,
                private_pem_enc: b64().encode(seal(cipher, &pem)?),
            });
        }
        Some(out)
    } else {
        None
    };
    Ok(Snapshot {
        device_id: me.to_string(),
        created_at: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        hosts: cfg.scopes.hosts.then(|| db.list_hosts()).transpose()?,
        groups: cfg.scopes.hosts.then(|| db.list_groups()).transpose()?,
        snippets: cfg.scopes.snippets.then(|| db.list_snippets()).transpose()?,
        keys,
        settings: None, // 设置项同步后续版本接入
    })
}

fn merge_snapshot(
    db: &Db,
    vault: &Vault,
    cipher: &XChaCha20Poly1305,
    remote: Snapshot,
) -> Result<()> {
    // 简化 LWW:本地不存在的实体直接导入;已存在的保留本地(后续版本比较 lamport/updated_at)
    if let Some(hosts) = remote.hosts {
        let local: std::collections::HashSet<String> =
            db.list_hosts()?.into_iter().map(|h| h.id).collect();
        for h in hosts {
            if !local.contains(&h.id) {
                db.save_host(HostInput {
                    id: None,
                    name: h.name,
                    addr: Some(h.addr),
                    port: Some(h.port),
                    user: Some(h.user),
                    group_id: None, // 分组映射从简:入默认分组
                    tags: Some(h.tags),
                    key_id: None,
                })?;
            }
        }
    }
    if let Some(snips) = remote.snippets {
        let local: std::collections::HashSet<String> =
            db.list_snippets()?.into_iter().map(|s| s.id).collect();
        for s in snips {
            if !local.contains(&s.id) {
                db.save_snippet(SnippetInput {
                    id: Some(s.id),
                    name: s.name,
                    command: s.command,
                    tags: Some(s.tags),
                })?;
            }
        }
    }
    if let Some(keys) = remote.keys {
        let local: std::collections::HashSet<String> =
            db.list_keys()?.into_iter().map(|k| k.fingerprint).collect();
        for k in keys {
            if !local.contains(&k.meta.fingerprint) {
                let pem = unseal(
                    cipher,
                    &b64().decode(&k.private_pem_enc).map_err(|e| AppError(e.to_string()))?,
                )?;
                let enc = vault.encrypt(&pem)?;
                db.insert_key(&k.meta.name, &k.meta.algo, &k.meta.fingerprint, &k.meta.public_key, &enc)?;
                db.set_key_sync(&k.meta.id, true).ok();
            }
        }
    }
    Ok(())
}
