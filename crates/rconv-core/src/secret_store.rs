use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use keyring::Entry;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json;
use thiserror::Error;

use crate::config::config_directory;

const SERVICE_NAME: &str = "com.convocations.app";
const MASTER_KEY_FILE: &str = "secret.key";
const ACCOUNT_PREFIX: &str = "convocations-";
const FALLBACK_DIR: &str = "secrets";
const FALLBACK_EXTENSION: &str = ".json";

/// Reference to a persisted secret, allowing retrieval from the backing store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "backend", rename_all = "kebab-case")]
pub enum SecretReference {
    /// Secret is stored in the host operating system's secure keyring.
    Keyring { account: String },
    /// Secret is stored in config.toml encrypted with the local master key.
    LocalEncrypted { nonce: String, ciphertext: String },
}

#[derive(Debug, Error)]
pub enum SecretStoreError {
    #[error("keyring operation failed: {0}")]
    Keyring(String),
    #[error("local encryption failed: {0}")]
    Crypto(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("base64 error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("serialization error: {0}")]
    Serde(String),
}

#[derive(Debug, Serialize, Deserialize)]
struct FallbackSecret {
    nonce: String,
    ciphertext: String,
}

fn label_from_account(account: &str) -> &str {
    account.strip_prefix(ACCOUNT_PREFIX).unwrap_or(account)
}

fn fallback_secret_path(label: &str) -> PathBuf {
    config_directory()
        .join(FALLBACK_DIR)
        .join(format!("{label}{FALLBACK_EXTENSION}"))
}

fn store_fallback_secret(label: &str, secret: &str) -> Result<(), SecretStoreError> {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return Err(SecretStoreError::Crypto(
            "cannot store empty secret".to_string(),
        ));
    }

    let (nonce, ciphertext) = encrypt_with_local_key(trimmed.as_bytes())?;
    let payload = FallbackSecret {
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    };

    let path = fallback_secret_path(label);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let encoded =
        serde_json::to_string(&payload).map_err(|err| SecretStoreError::Serde(err.to_string()))?;
    fs::write(path, encoded)?;
    Ok(())
}

fn fallback_secret_exists(label: &str) -> bool {
    fallback_secret_path(label).exists()
}

fn load_fallback_secret(label: &str) -> Result<Option<String>, SecretStoreError> {
    let path = fallback_secret_path(label);
    let raw = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) => match err.kind() {
            io::ErrorKind::NotFound => return Ok(None),
            _ => return Err(SecretStoreError::Io(err)),
        },
    };

    let payload: FallbackSecret =
        serde_json::from_str(&raw).map_err(|err| SecretStoreError::Serde(err.to_string()))?;
    let nonce_bytes = STANDARD.decode(payload.nonce)?;
    let cipher_bytes = STANDARD.decode(payload.ciphertext)?;
    let plaintext = decrypt_with_local_key(&nonce_bytes, &cipher_bytes)?;
    Ok(Some(String::from_utf8_lossy(&plaintext).to_string()))
}

fn delete_fallback_secret(label: &str) -> Result<(), SecretStoreError> {
    let path = fallback_secret_path(label);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) => match err.kind() {
            io::ErrorKind::NotFound => Ok(()),
            _ => Err(SecretStoreError::Io(err)),
        },
    }
}

fn ensure_fallback_secret(label: &str, secret: &str) {
    if fallback_secret_exists(label) {
        return;
    }
    if let Err(err) = store_fallback_secret(label, secret) {
        eprintln!(
            "[Convocations] Failed to create fallback secret for {}: {}",
            label, err
        );
    }
}

/// Persist a secret using the most secure backend available.
pub fn store_secret(label: &str, secret: &str) -> Result<SecretReference, SecretStoreError> {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return Err(SecretStoreError::Crypto(
            "cannot store empty secret".to_string(),
        ));
    }

    let account = format!("{ACCOUNT_PREFIX}{label}");
    match Entry::new(SERVICE_NAME, &account) {
        Ok(entry) => {
            if let Err(err) = entry.set_password(trimmed) {
                eprintln!(
                    "[Convocations] keyring set_password failed for {}: {}. Falling back to local encryption.",
                    label, err
                );
            } else {
                if let Err(err) = store_fallback_secret(label, trimmed) {
                    eprintln!(
                        "[Convocations] Failed to persist fallback secret for {}: {}",
                        label, err
                    );
                }
                return Ok(SecretReference::Keyring { account });
            }
        }
        Err(err) => {
            eprintln!(
                "[Convocations] keyring unavailable for {}: {}. Falling back to local encryption.",
                label, err
            );
        }
    }

    let (nonce, ciphertext) = encrypt_with_local_key(trimmed.as_bytes())?;
    if let Err(err) = delete_fallback_secret(label) {
        eprintln!(
            "[Convocations] Failed to remove fallback secret for {}: {}",
            label, err
        );
    }
    Ok(SecretReference::LocalEncrypted {
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    })
}

/// Retrieve a secret based on the stored reference.
pub fn load_secret(reference: &SecretReference) -> Result<Option<String>, SecretStoreError> {
    match reference {
        SecretReference::Keyring { account } => {
            let label = label_from_account(account);
            match Entry::new(SERVICE_NAME, account) {
                Ok(entry) => match entry.get_password() {
                    Ok(value) => {
                        if !value.trim().is_empty() {
                            ensure_fallback_secret(label, &value);
                            Ok(Some(value))
                        } else {
                            load_fallback_secret(label)
                        }
                    }
                    Err(keyring::Error::NoEntry) => {
                        if fallback_secret_exists(label) {
                            eprintln!(
                                "[Convocations] keyring entry for {} missing; using encrypted fallback.",
                                label
                            );
                            load_fallback_secret(label)
                        } else {
                            Ok(None)
                        }
                    }
                    Err(err) => {
                        eprintln!(
                            "[Convocations] keyring get_password failed for {}: {}. Attempting fallback.",
                            label, err
                        );
                        match load_fallback_secret(label) {
                            Ok(Some(secret)) => {
                                eprintln!("[Convocations] Using encrypted fallback for {}.", label);
                                Ok(Some(secret))
                            }
                            Ok(None) => Err(SecretStoreError::Keyring(err.to_string())),
                            Err(fallback_err) => Err(fallback_err),
                        }
                    }
                },
                Err(err) => {
                    eprintln!(
                        "[Convocations] keyring unavailable while loading {}: {}. Attempting fallback.",
                        label, err
                    );
                    match load_fallback_secret(label) {
                        Ok(Some(secret)) => {
                            eprintln!("[Convocations] Using encrypted fallback for {}.", label);
                            Ok(Some(secret))
                        }
                        Ok(None) => Err(SecretStoreError::Keyring(err.to_string())),
                        Err(fallback_err) => Err(fallback_err),
                    }
                }
            }
        }
        SecretReference::LocalEncrypted { nonce, ciphertext } => {
            let nonce_bytes = STANDARD.decode(nonce)?;
            let cipher_bytes = STANDARD.decode(ciphertext)?;
            let plaintext = decrypt_with_local_key(&nonce_bytes, &cipher_bytes)?;
            Ok(Some(String::from_utf8_lossy(&plaintext).to_string()))
        }
    }
}

/// Delete a secret from its backing store.
pub fn delete_secret(reference: &SecretReference) -> Result<(), SecretStoreError> {
    match reference {
        SecretReference::Keyring { account } => {
            let label = label_from_account(account);
            match Entry::new(SERVICE_NAME, account) {
                Ok(entry) => match entry.delete_password() {
                    Ok(()) | Err(keyring::Error::NoEntry) => (),
                    Err(err) => return Err(SecretStoreError::Keyring(err.to_string())),
                },
                Err(err) => return Err(SecretStoreError::Keyring(err.to_string())),
            }
            delete_fallback_secret(label)
        }
        SecretReference::LocalEncrypted { .. } => Ok(()),
    }
}

fn encrypt_with_local_key(plaintext: &[u8]) -> Result<([u8; 12], Vec<u8>), SecretStoreError> {
    let key = get_or_create_master_key()?;
    let cipher = ChaCha20Poly1305::new_from_slice(&key)
        .map_err(|err| SecretStoreError::Crypto(err.to_string()))?;

    let mut nonce_bytes = [0u8; 12];
    let mut rng = rand::rng();
    rng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from(nonce_bytes);

    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|err| SecretStoreError::Crypto(err.to_string()))?;
    Ok((nonce_bytes, ciphertext))
}

fn decrypt_with_local_key(nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, SecretStoreError> {
    if nonce.len() != 12 {
        return Err(SecretStoreError::Crypto(
            "invalid nonce length for chacha20poly1305".to_string(),
        ));
    }
    let key = get_or_create_master_key()?;
    let cipher = ChaCha20Poly1305::new_from_slice(&key)
        .map_err(|err| SecretStoreError::Crypto(err.to_string()))?;
    let mut nonce_array = [0u8; 12];
    nonce_array.copy_from_slice(nonce);
    let nonce = Nonce::from(nonce_array);
    cipher
        .decrypt(&nonce, ciphertext)
        .map_err(|err| SecretStoreError::Crypto(err.to_string()))
}

fn get_or_create_master_key() -> Result<[u8; 32], SecretStoreError> {
    let path = master_key_path();
    if path.exists() {
        let bytes = fs::read(&path)?;
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
        eprintln!(
            "[Convocations] master key at {} had unexpected length {}; regenerating.",
            path.display(),
            bytes.len()
        );
    }

    let mut key = [0u8; 32];
    let mut rng = rand::rng();
    rng.fill_bytes(&mut key);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_key_file(&path, &key)?;
    Ok(key)
}

fn write_key_file(path: &PathBuf, key: &[u8]) -> Result<(), SecretStoreError> {
    let mut file = fs::File::create(path)?;
    file.write_all(key)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn master_key_path() -> PathBuf {
    config_directory().join(MASTER_KEY_FILE)
}
