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
use thiserror::Error;

use crate::config::config_directory;

const SERVICE_NAME: &str = "com.convocations.app";
const MASTER_KEY_FILE: &str = "secret.key";

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
}

/// Persist a secret using the most secure backend available.
pub fn store_secret(label: &str, secret: &str) -> Result<SecretReference, SecretStoreError> {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return Err(SecretStoreError::Crypto(
            "cannot store empty secret".to_string(),
        ));
    }

    let account = format!("convocations-{label}");
    match Entry::new(SERVICE_NAME, &account) {
        Ok(entry) => {
            if let Err(err) = entry.set_password(trimmed) {
                eprintln!(
                    "[Convocations] keyring set_password failed for {}: {}. Falling back to local encryption.",
                    label, err
                );
            } else {
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
    Ok(SecretReference::LocalEncrypted {
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    })
}

/// Retrieve a secret based on the stored reference.
pub fn load_secret(reference: &SecretReference) -> Result<Option<String>, SecretStoreError> {
    match reference {
        SecretReference::Keyring { account } => match Entry::new(SERVICE_NAME, account) {
            Ok(entry) => match entry.get_password() {
                Ok(value) => Ok(Some(value)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(err) => Err(SecretStoreError::Keyring(err.to_string())),
            },
            Err(err) => Err(SecretStoreError::Keyring(err.to_string())),
        },
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
        SecretReference::Keyring { account } => match Entry::new(SERVICE_NAME, account) {
            Ok(entry) => match entry.delete_password() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(err) => Err(SecretStoreError::Keyring(err.to_string())),
            },
            Err(err) => Err(SecretStoreError::Keyring(err.to_string())),
        },
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
