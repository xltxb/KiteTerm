//! 密钥库:生成/导入 SSH 密钥,私钥以设备密钥加密后落库(XChaCha20-Poly1305)。
//! 设备密钥当前存于应用数据目录 0600 文件;TODO: 迁移到系统 Keychain(生物识别解锁)。

use std::path::Path;

use chacha20poly1305::aead::{Aead, KeyInit, OsRng as AeadOsRng};
use chacha20poly1305::{AeadCore, XChaCha20Poly1305, XNonce};
use russh::keys::ssh_key::getrandom::SysRng;
use russh::keys::ssh_key::rand_core::UnwrapErr;
use russh::keys::ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey};

use crate::error::{AppError, Result};

const DEVICE_KEY_FILE: &str = "device.key";

pub struct Vault {
    cipher: XChaCha20Poly1305,
}

impl Vault {
    /// 设备密钥优先存系统 Keychain(macOS 钥匙串 / Windows 凭据管理器);
    /// 旧版 device.key 文件自动迁移后删除;Keychain 不可用时回退 0600 文件存储。
    ///
    /// `VELA_NO_KEYCHAIN=1` 强制走文件回退 —— 用于 CI / 集成测试:每次重新编译都会
    /// 产生新二进制,系统钥匙串会视其为新程序而弹出授权框并阻塞无人值守的进程。
    pub fn open(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join(DEVICE_KEY_FILE);
        if std::env::var("VELA_NO_KEYCHAIN").is_err() {
            if let Ok(entry) = keyring::Entry::new("com.vela.term", "device-key") {
                // 1) Keychain 已有
                if let Ok(hex) = entry.get_password() {
                    if let Some(key) = decode_key_hex(&hex) {
                        return Ok(Self { cipher: XChaCha20Poly1305::new((&key).into()) });
                    }
                }
                // 2) 旧版 device.key 文件迁移进 Keychain
                if path.exists() {
                    let raw = std::fs::read(&path)?;
                    let key: [u8; 32] =
                        raw.try_into().map_err(|_| AppError("device.key 已损坏".into()))?;
                    if entry.set_password(&encode_key_hex(&key)).is_ok() {
                        let _ = std::fs::remove_file(&path);
                    }
                    return Ok(Self { cipher: XChaCha20Poly1305::new((&key).into()) });
                }
                // 3) 全新生成并写入 Keychain
                let key = XChaCha20Poly1305::generate_key(&mut AeadOsRng);
                let key_bytes: [u8; 32] = key.into();
                if entry.set_password(&encode_key_hex(&key_bytes)).is_ok() {
                    return Ok(Self { cipher: XChaCha20Poly1305::new((&key_bytes).into()) });
                }
            }
        }
        // Keychain 不可用或被显式跳过:回退 0600 文件
        let key_bytes: [u8; 32] = if path.exists() {
            let raw = std::fs::read(&path)?;
            raw.try_into().map_err(|_| AppError("device.key 已损坏".into()))?
        } else {
            let key = XChaCha20Poly1305::generate_key(&mut AeadOsRng);
            std::fs::write(&path, key.as_slice())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            }
            key.into()
        };
        Ok(Self { cipher: XChaCha20Poly1305::new((&key_bytes).into()) })
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut AeadOsRng);
        let ct = self
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| AppError(format!("加密失败: {e}")))?;
        let mut out = nonce.to_vec();
        out.extend(ct);
        Ok(out)
    }

    pub fn decrypt(&self, blob: &[u8]) -> Result<Vec<u8>> {
        if blob.len() < 24 {
            return Err(AppError("密文格式错误".into()));
        }
        let (nonce, ct) = blob.split_at(24);
        self.cipher
            .decrypt(XNonce::from_slice(nonce), ct)
            .map_err(|e| AppError(format!("解密失败(设备密钥不匹配?): {e}")))
    }
}

fn encode_key_hex(key: &[u8; 32]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_key_hex(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

pub struct GeneratedKey {
    pub algo: String,
    pub fingerprint: String,
    pub public_openssh: String,
    pub private_pem: String,
}

pub fn generate(algo: &str, comment: &str) -> Result<GeneratedKey> {
    let algorithm = match algo {
        "ed25519" => Algorithm::Ed25519,
        "rsa4096" => Algorithm::Rsa { hash: None },
        other => return Err(AppError(format!("不支持的算法: {other}"))),
    };
    let mut rng = UnwrapErr(SysRng);
    let mut key = PrivateKey::random(&mut rng, algorithm)
        .map_err(|e| AppError(format!("生成密钥失败: {e}")))?;
    key.set_comment(comment);
    key_to_generated(algo, &key)
}

pub fn import(pem: &str) -> Result<GeneratedKey> {
    let key = PrivateKey::from_openssh(pem)
        .map_err(|e| AppError(format!("解析私钥失败(需 OpenSSH 格式,且不带口令): {e}")))?;
    if key.is_encrypted() {
        return Err(AppError("暂不支持带口令的私钥,请先用 ssh-keygen -p -N '' 去除口令".into()));
    }
    let algo = match key.algorithm() {
        Algorithm::Ed25519 => "ed25519",
        Algorithm::Rsa { .. } => "rsa4096",
        other => return Err(AppError(format!("暂不支持的密钥类型: {other}"))),
    };
    key_to_generated(algo, &key)
}

fn key_to_generated(algo: &str, key: &PrivateKey) -> Result<GeneratedKey> {
    let public = key.public_key();
    let fingerprint = public.fingerprint(HashAlg::Sha256).to_string();
    let public_openssh = public
        .to_openssh()
        .map_err(|e| AppError(format!("导出公钥失败: {e}")))?;
    let private_pem = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| AppError(format!("导出私钥失败: {e}")))?
        .to_string();
    Ok(GeneratedKey {
        algo: algo.to_string(),
        fingerprint,
        public_openssh,
        private_pem,
    })
}

pub fn parse_private(pem: &str) -> Result<PrivateKey> {
    PrivateKey::from_openssh(pem).map_err(|e| AppError(format!("私钥解析失败: {e}")))
}
