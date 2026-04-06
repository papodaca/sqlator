use crate::error::CoreError;
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use argon2::{Argon2, Algorithm, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

// ── On-disk format ────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct VaultFile {
    version: u32,
    /// base64-encoded 32-byte Argon2id salt
    salt: String,
    /// base64-encoded 12-byte AES-GCM nonce
    nonce: String,
    /// base64-encoded AES-GCM ciphertext of the JSON entry map
    ciphertext: String,
}

// ── In-memory unlocked state ─────────────────────────────────────────────────

struct UnlockedVault {
    entries: HashMap<String, String>,
    /// AES-256-GCM key derived from master password
    key: [u8; 32],
    /// Salt stored here so we can re-write the file without needing the password again
    salt: [u8; 32],
    last_activity: Instant,
}

// ── Public backend ────────────────────────────────────────────────────────────

pub struct VaultBackend {
    path: PathBuf,
    state: Mutex<Option<UnlockedVault>>,
    /// Idle timeout in seconds; 0 = never expire
    timeout_secs: Mutex<u64>,
}

impl VaultBackend {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: Mutex::new(None),
            timeout_secs: Mutex::new(15 * 60),
        }
    }

    pub fn set_timeout(&self, secs: u64) {
        *self.timeout_secs.lock().unwrap() = secs;
    }

    pub fn timeout_secs(&self) -> u64 {
        *self.timeout_secs.lock().unwrap()
    }

    /// Returns true if the vault file exists on disk.
    pub fn is_initialized(&self) -> bool {
        self.path.exists()
    }

    /// Returns true when the in-memory state is absent or has timed out.
    pub fn is_locked(&self) -> bool {
        let guard = self.state.lock().unwrap();
        match &*guard {
            None => true,
            Some(u) => {
                let timeout = *self.timeout_secs.lock().unwrap();
                timeout > 0 && u.last_activity.elapsed().as_secs() >= timeout
            }
        }
    }

    /// Create a new vault encrypted with `password`. Leaves the vault unlocked.
    pub fn create(&self, password: &str) -> Result<(), CoreError> {
        if self.path.exists() {
            return Err(CoreError {
                message: "Vault already exists. Delete it first or use unlock.".into(),
                code: "VAULT_EXISTS".into(),
            });
        }

        let mut salt = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut salt);
        let key = derive_key(password, &salt)?;

        let entries: HashMap<String, String> = HashMap::new();
        let vault_file = make_vault_file(&entries, &key, &salt)?;
        write_vault_atomic(&self.path, &vault_file)?;

        *self.state.lock().unwrap() = Some(UnlockedVault {
            entries,
            key,
            salt,
            last_activity: Instant::now(),
        });
        Ok(())
    }

    /// Decrypt the vault with `password` and keep the plaintext in memory.
    pub fn unlock(&self, password: &str) -> Result<(), CoreError> {
        let vault_file = read_vault(&self.path)?;

        let salt = decode_32(&vault_file.salt, "salt")?;
        let key = derive_key(password, &salt)?;

        let nonce_bytes = B64.decode(&vault_file.nonce).map_err(vault_corrupt)?;
        if nonce_bytes.len() != 12 {
            return Err(vault_corrupt("nonce must be 12 bytes"));
        }
        let ciphertext = B64.decode(&vault_file.ciphertext).map_err(vault_corrupt)?;

        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = cipher.decrypt(nonce, ciphertext.as_ref()).map_err(|_| CoreError {
            message: "Wrong password or corrupted vault".into(),
            code: "VAULT_WRONG_PASSWORD".into(),
        })?;

        let entries: HashMap<String, String> =
            serde_json::from_slice(&plaintext).map_err(|e| vault_corrupt(e.to_string()))?;

        *self.state.lock().unwrap() = Some(UnlockedVault {
            entries,
            key,
            salt,
            last_activity: Instant::now(),
        });
        Ok(())
    }

    /// Clear the in-memory plaintext.
    pub fn lock(&self) {
        *self.state.lock().unwrap() = None;
    }

    /// Return a snapshot of all entries — used for migration.
    /// Requires the vault to be unlocked.
    pub fn drain_entries(&self) -> Result<HashMap<String, String>, CoreError> {
        let mut guard = self.state.lock().unwrap();
        check_and_touch(&mut guard, &self.timeout_secs)?;
        Ok(guard.as_ref().unwrap().entries.clone())
    }
}

// ── CredentialBackend implementation ─────────────────────────────────────────

impl super::CredentialBackend for VaultBackend {
    fn store(&self, key: &str, secret: &str) -> Result<(), CoreError> {
        let vault_file = {
            let mut guard = self.state.lock().unwrap();
            check_and_touch(&mut guard, &self.timeout_secs)?;
            let u = guard.as_mut().unwrap();
            u.entries.insert(key.to_string(), secret.to_string());
            make_vault_file(&u.entries, &u.key, &u.salt)?
        };
        write_vault_atomic(&self.path, &vault_file)
    }

    fn get(&self, key: &str) -> Result<Option<String>, CoreError> {
        let mut guard = self.state.lock().unwrap();
        check_and_touch(&mut guard, &self.timeout_secs)?;
        Ok(guard.as_ref().unwrap().entries.get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<(), CoreError> {
        let vault_file = {
            let mut guard = self.state.lock().unwrap();
            check_and_touch(&mut guard, &self.timeout_secs)?;
            let u = guard.as_mut().unwrap();
            u.entries.remove(key);
            make_vault_file(&u.entries, &u.key, &u.salt)?
        };
        write_vault_atomic(&self.path, &vault_file)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn check_and_touch(
    guard: &mut Option<UnlockedVault>,
    timeout_secs: &Mutex<u64>,
) -> Result<(), CoreError> {
    match guard {
        None => Err(CoreError {
            message: "Vault is locked".into(),
            code: "VAULT_LOCKED".into(),
        }),
        Some(u) => {
            let timeout = *timeout_secs.lock().unwrap();
            if timeout > 0 && u.last_activity.elapsed().as_secs() >= timeout {
                *guard = None;
                Err(CoreError {
                    message: "Vault session timed out — please unlock again".into(),
                    code: "VAULT_TIMED_OUT".into(),
                })
            } else {
                u.last_activity = Instant::now();
                Ok(())
            }
        }
    }
}

fn derive_key(password: &str, salt: &[u8; 32]) -> Result<[u8; 32], CoreError> {
    // OWASP-recommended minimum Argon2id params
    let params = Params::new(19456, 2, 1, Some(32)).map_err(|e| CoreError {
        message: format!("Argon2 params error: {e}"),
        code: "KDF_ERROR".into(),
    })?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| CoreError {
            message: format!("Key derivation failed: {e}"),
            code: "KDF_ERROR".into(),
        })?;
    Ok(key)
}

fn make_vault_file(
    entries: &HashMap<String, String>,
    key: &[u8; 32],
    salt: &[u8; 32],
) -> Result<VaultFile, CoreError> {
    let plaintext = serde_json::to_vec(entries).map_err(|e| CoreError {
        message: format!("Vault serialize error: {e}"),
        code: "VAULT_ERROR".into(),
    })?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let ciphertext = cipher.encrypt(&nonce, plaintext.as_ref()).map_err(|e| CoreError {
        message: format!("Encryption failed: {e}"),
        code: "VAULT_ERROR".into(),
    })?;
    Ok(VaultFile {
        version: 1,
        salt: B64.encode(salt),
        nonce: B64.encode(nonce.as_slice()),
        ciphertext: B64.encode(&ciphertext),
    })
}

fn read_vault(path: &PathBuf) -> Result<VaultFile, CoreError> {
    let data = std::fs::read_to_string(path).map_err(|e| CoreError {
        message: format!("Cannot read vault file: {e}"),
        code: "VAULT_IO_ERROR".into(),
    })?;
    serde_json::from_str(&data).map_err(|e| vault_corrupt(e.to_string()))
}

fn write_vault_atomic(path: &PathBuf, vault: &VaultFile) -> Result<(), CoreError> {
    let data = serde_json::to_string_pretty(vault).map_err(|e| CoreError {
        message: format!("Vault serialize error: {e}"),
        code: "VAULT_ERROR".into(),
    })?;
    // Atomic write: temp file → rename
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &data).map_err(|e| CoreError {
        message: format!("Vault write failed: {e}"),
        code: "VAULT_IO_ERROR".into(),
    })?;
    std::fs::rename(&tmp, path).map_err(|e| CoreError {
        message: format!("Vault rename failed: {e}"),
        code: "VAULT_IO_ERROR".into(),
    })?;
    Ok(())
}

fn decode_32(b64: &str, field: &str) -> Result<[u8; 32], CoreError> {
    let bytes = B64.decode(b64).map_err(vault_corrupt)?;
    if bytes.len() != 32 {
        return Err(vault_corrupt(format!("{field} must be 32 bytes")));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

fn vault_corrupt(msg: impl ToString) -> CoreError {
    CoreError {
        message: format!("Vault file corrupted: {}", msg.to_string()),
        code: "VAULT_CORRUPT".into(),
    }
}
