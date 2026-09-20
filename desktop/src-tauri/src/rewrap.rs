//! 设备密钥迁移:把用历史 device.key 加密的存量数据改用当前设备密钥重新加密。
//!
//! 什么时候会有两把密钥:钥匙串里已经有设备密钥,磁盘上却还留着 device.key。
//! 典型来源是带 `KITE_NO_KEYCHAIN=1` 跑过开发/自检版本(那会走文件回退),
//! 之后又用正常版本启动 —— 于是同一个库里混了两批不同密钥的密文,
//! 只认钥匙串那把就会报「解密失败(设备密钥不匹配?)」。
//!
//! 迁移原则:**先验证再落库,全部成功才删旧密钥**。任何一条没搞定就保留
//! device.key,下次启动重试 —— 宁可多留一个文件,也不能让密文变成解不开的垃圾。

use base64::Engine;

use crate::db::Db;
use crate::error::Result;
use crate::vault::Vault;

/// kv 里以 base64 存放的 vault 密文(非 `ai_key_*` 的那些固定键)
///
/// **新增任何一处 `vault.encrypt` 落库都必须在这里登记**(表里的列则加进下面的扫描)。
/// 漏登记不会报错,只会让那条密文在 device.key 被删掉之后永远解不开。
const KV_ENCRYPTED: &[&str] = &["account_token_enc", "sync_mk_enc"];
const KV_AI_KEY_PREFIX: &str = "ai_key_";

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// 返回重新加密的条目数。没有历史密钥时直接返回 0。
pub fn run(db: &Db, vault: &Vault) -> Result<usize> {
    if !vault.has_legacy() {
        return Ok(0);
    }

    let mut rewrapped = 0usize;
    let mut all_ok = true;

    // ---- SSH 私钥 ----
    for meta in db.list_keys()? {
        let Ok(blob) = db.get_key_private_enc(&meta.id) else {
            all_ok = false;
            continue;
        };
        match rewrap(vault, &blob) {
            Rewrap::AlreadyCurrent => {}
            Rewrap::Migrated(new_blob) => {
                db.set_key_private_enc(&meta.id, &new_blob)?;
                rewrapped += 1;
            }
            Rewrap::Failed => {
                eprintln!("[rewrap] 密钥 {} 两把设备密钥都解不开,保留原样", meta.name);
                all_ok = false;
            }
        }
    }

    // ---- kv 里的密文(AI 模型密钥、账户令牌)----
    let mut kv_keys = db.kv_keys_with_prefix(KV_AI_KEY_PREFIX)?;
    kv_keys.extend(KV_ENCRYPTED.iter().map(|s| s.to_string()));
    for key in kv_keys {
        let Some(encoded) = db.kv_get(&key)? else { continue };
        let Ok(blob) = b64().decode(&encoded) else {
            // 不是 base64,不该出现在这批键里;跳过而不是当失败,免得永远删不掉旧密钥
            continue;
        };
        match rewrap(vault, &blob) {
            Rewrap::AlreadyCurrent => {}
            Rewrap::Migrated(new_blob) => {
                db.kv_set(&key, &b64().encode(&new_blob))?;
                rewrapped += 1;
            }
            Rewrap::Failed => {
                eprintln!("[rewrap] {key} 两把设备密钥都解不开,保留原样");
                all_ok = false;
            }
        }
    }

    // ---- ACME 证书(账户密钥、证书私钥;在 certs 表里,不走 kv)----
    for mut sec in db.list_cert_secrets()? {
        let id = sec.id.clone();
        let mut changed = false;
        for (blob, what) in [
            (&mut sec.account_key_enc, "账户密钥"),
            (&mut sec.key_pem_enc, "证书私钥"),
        ] {
            let Some(current) = blob.as_deref() else { continue };
            match rewrap(vault, current) {
                Rewrap::AlreadyCurrent => {}
                Rewrap::Migrated(new_blob) => {
                    *blob = Some(new_blob);
                    changed = true;
                    rewrapped += 1;
                }
                Rewrap::Failed => {
                    eprintln!("[rewrap] 证书 {id} 的{what}两把设备密钥都解不开,保留原样");
                    all_ok = false;
                }
            }
        }
        if changed {
            db.set_cert_secrets(&sec)?;
        }
    }

    // 全部就位才敢删 device.key
    if all_ok {
        vault.retire_legacy()?;
        if rewrapped > 0 {
            println!("[rewrap] 已用当前设备密钥重新加密 {rewrapped} 条,历史 device.key 已移除");
        }
    } else {
        eprintln!("[rewrap] 存在未能迁移的条目,保留 device.key 以便下次重试");
    }
    Ok(rewrapped)
}

enum Rewrap {
    /// 已经是当前设备密钥加密的,不用动
    AlreadyCurrent,
    Migrated(Vec<u8>),
    Failed,
}

fn rewrap(vault: &Vault, blob: &[u8]) -> Rewrap {
    if vault.decrypt_current(blob).is_ok() {
        return Rewrap::AlreadyCurrent;
    }
    // decrypt 会回退到历史密钥
    let Ok(plain) = vault.decrypt(blob) else { return Rewrap::Failed };
    let Ok(new_blob) = vault.encrypt(&plain) else { return Rewrap::Failed };
    // 落库前先确认新密文真能用当前密钥解回原文,避免写进去一堆解不开的东西
    match vault.decrypt_current(&new_blob) {
        Ok(check) if check == plain => Rewrap::Migrated(new_blob),
        _ => Rewrap::Failed,
    }
}
