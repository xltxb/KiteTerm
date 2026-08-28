//! 密钥库:生成/导入 SSH 密钥,私钥以设备密钥加密后落库(XChaCha20-Poly1305)。
//! 设备密钥当前存于应用数据目录 0600 文件;TODO: 迁移到系统 Keychain(生物识别解锁)。

use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit, OsRng as AeadOsRng};
use chacha20poly1305::{AeadCore, XChaCha20Poly1305, XNonce};
use russh::keys::ssh_key::getrandom::SysRng;
use russh::keys::ssh_key::rand_core::UnwrapErr;
use russh::keys::ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey};

use crate::error::{AppError, Result};

const DEVICE_KEY_FILE: &str = "device.key";

pub struct Vault {
    cipher: XChaCha20Poly1305,
    /// 历史遗留的 device.key。
    ///
    /// 钥匙串里已经有设备密钥、磁盘上却还留着 device.key 时,说明这台机器曾用
    /// 文件密钥加密过数据(典型来源:带 `KITE_NO_KEYCHAIN=1` 跑过开发/自检版本,
    /// 之后又用正常版本启动)。两批密文混在同一个库里,只认钥匙串那把会解不开
    /// 其中一半。这里把旧密钥留作兜底,解密时按「先新后旧」依次尝试。
    legacy: Option<XChaCha20Poly1305>,
    legacy_path: Option<PathBuf>,
}

impl Vault {
    /// 设备密钥优先存系统 Keychain(macOS 钥匙串 / Windows 凭据管理器);
    /// 旧版 device.key 文件自动迁移后删除;Keychain 不可用时回退 0600 文件存储。
    ///
    /// `KITE_NO_KEYCHAIN=1` 强制走文件回退 —— 用于 CI / 集成测试:每次重新编译都会
    /// 产生新二进制,系统钥匙串会视其为新程序而弹出授权框并阻塞无人值守的进程。
    pub fn open(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join(DEVICE_KEY_FILE);
        if std::env::var("KITE_NO_KEYCHAIN").is_err() {
            if let Ok(entry) = keyring::Entry::new("com.kiteterm.app", "device-key") {
                // 1) Keychain 已有 —— 若 device.key 仍在,说明存在用旧密钥加密的存量数据,
                //    留作解密兜底(见 Vault::legacy),等 rewrap 迁移完再删文件
                if let Ok(hex) = entry.get_password() {
                    if let Some(key) = decode_key_hex(&hex) {
                        return Ok(Self::with_device_key(&key, data_dir));
                    }
                }
                // 2) 旧版 device.key 文件迁移进 Keychain(同一把密钥,无需兜底)
                if path.exists() {
                    let raw = std::fs::read(&path)?;
                    let key: [u8; 32] =
                        raw.try_into().map_err(|_| AppError("device.key 已损坏".into()))?;
                    if entry.set_password(&encode_key_hex(&key)).is_ok() {
                        let _ = std::fs::remove_file(&path);
                    }
                    return Ok(Self::with_device_key(&key, data_dir));
                }
                // 3) 全新生成并写入 Keychain
                let key = XChaCha20Poly1305::generate_key(&mut AeadOsRng);
                let key_bytes: [u8; 32] = key.into();
                if entry.set_password(&encode_key_hex(&key_bytes)).is_ok() {
                    return Ok(Self::with_device_key(&key_bytes, data_dir));
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
        // 这条路径的 device.key 就是当前设备密钥本身,不是需要迁移的历史密钥
        Ok(Self {
            cipher: XChaCha20Poly1305::new((&key_bytes).into()),
            legacy: None,
            legacy_path: None,
        })
    }

    /// 用给定的设备密钥构造。数据目录里若还留着 device.key,自动收作历史密钥兜底
    /// —— 走 Keychain 的三个分支都经由这里,自检也用它构造两把密钥并存的场景。
    pub fn with_device_key(key: &[u8; 32], data_dir: &Path) -> Self {
        let (legacy, legacy_path) = read_legacy_key(&data_dir.join(DEVICE_KEY_FILE));
        Self { cipher: XChaCha20Poly1305::new(key.into()), legacy, legacy_path }
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

    /// 只用当前设备密钥解密 —— 迁移时用它判断某条密文是否已经是新密钥加密的。
    pub fn decrypt_current(&self, blob: &[u8]) -> Result<Vec<u8>> {
        let (nonce, ct) = split_blob(blob)?;
        self.cipher
            .decrypt(XNonce::from_slice(nonce), ct)
            .map_err(|e| AppError(format!("解密失败(设备密钥不匹配?): {e}")))
    }

    /// 先用当前设备密钥解,失败再试历史 device.key(见 Vault::legacy)。
    pub fn decrypt(&self, blob: &[u8]) -> Result<Vec<u8>> {
        let (nonce, ct) = split_blob(blob)?;
        if let Ok(plain) = self.cipher.decrypt(XNonce::from_slice(nonce), ct) {
            return Ok(plain);
        }
        if let Some(legacy) = &self.legacy {
            if let Ok(plain) = legacy.decrypt(XNonce::from_slice(nonce), ct) {
                return Ok(plain);
            }
        }
        Err(AppError("解密失败(设备密钥不匹配?)".into()))
    }

    /// 磁盘上是否还留着需要迁移的历史 device.key
    pub fn has_legacy(&self) -> bool {
        self.legacy.is_some()
    }

    /// 存量密文都改用当前设备密钥之后,删掉历史 device.key。
    /// 只有迁移全部成功才调用 —— 删早了就再也解不开了。
    pub fn retire_legacy(&self) -> Result<()> {
        if let Some(path) = &self.legacy_path {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

fn split_blob(blob: &[u8]) -> Result<(&[u8], &[u8])> {
    if blob.len() < 24 {
        return Err(AppError("密文格式错误".into()));
    }
    Ok(blob.split_at(24))
}

/// device.key 存在就读出来当兜底密钥;读不出(损坏/权限)按没有处理,
/// 不能让它挡住应用启动。
fn read_legacy_key(path: &Path) -> (Option<XChaCha20Poly1305>, Option<PathBuf>) {
    if !path.exists() {
        return (None, None);
    }
    let Ok(raw) = std::fs::read(path) else { return (None, None) };
    let Ok(key) = <[u8; 32]>::try_from(raw) else { return (None, None) };
    (Some(XChaCha20Poly1305::new((&key).into())), Some(path.to_path_buf()))
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
