mod keyring_backend;
mod vault_backend;

pub use keyring_backend::KeyringBackend;
pub use vault_backend::VaultBackend;

use crate::error::CoreError;
use std::path::PathBuf;
use std::sync::Mutex;

// ── Trait ─────────────────────────────────────────────────────────────────────

pub trait CredentialBackend: Send + Sync {
    fn store(&self, key: &str, secret: &str) -> Result<(), CoreError>;
    fn get(&self, key: &str) -> Result<Option<String>, CoreError>;
    fn delete(&self, key: &str) -> Result<(), CoreError>;
}

// ── Storage mode ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageMode {
    Keyring,
    Vault,
}

impl std::fmt::Display for StorageMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageMode::Keyring => write!(f, "keyring"),
            StorageMode::Vault => write!(f, "vault"),
        }
    }
}

impl std::str::FromStr for StorageMode {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "keyring" => Ok(StorageMode::Keyring),
            "vault" => Ok(StorageMode::Vault),
            other => Err(CoreError {
                message: format!("Unknown storage mode: {other}"),
                code: "INVALID_MODE".into(),
            }),
        }
    }
}

// ── CredentialStore ───────────────────────────────────────────────────────────

/// Unified credential store delegating to keyring or encrypted vault.
pub struct CredentialStore {
    pub vault: VaultBackend,
    keyring: KeyringBackend,
    mode: Mutex<StorageMode>,
}

impl CredentialStore {
    pub fn new(vault_path: PathBuf, mode: StorageMode) -> Self {
        Self {
            vault: VaultBackend::new(vault_path),
            keyring: KeyringBackend,
            mode: Mutex::new(mode),
        }
    }

    pub fn store_credential(&self, profile_id: &str, kind: &str, secret: &str) -> Result<(), CoreError> {
        let key = credential_key(profile_id, kind);
        self.active()?.store(&key, secret)
    }

    pub fn get_credential(&self, profile_id: &str, kind: &str) -> Result<Option<String>, CoreError> {
        let key = credential_key(profile_id, kind);
        self.active()?.get(&key)
    }

    pub fn delete_credential(&self, profile_id: &str, kind: &str) -> Result<(), CoreError> {
        let key = credential_key(profile_id, kind);
        self.active()?.delete(&key)
    }

    pub fn delete_all_credentials(&self, profile_id: &str) -> Result<(), CoreError> {
        self.delete_credential(profile_id, "password")?;
        self.delete_credential(profile_id, "passphrase")?;
        Ok(())
    }

    pub fn mode(&self) -> StorageMode {
        self.mode.lock().unwrap().clone()
    }

    pub fn set_mode(&self, mode: StorageMode) {
        *self.mode.lock().unwrap() = mode;
    }

    /// Copy credentials for `profile_ids` from current mode to `new_mode`.
    /// Vault must be unlocked when it is involved as source or destination.
    pub fn migrate_to(&self, new_mode: &StorageMode, profile_ids: &[String]) -> Result<Vec<String>, CoreError> {
        if self.mode() == *new_mode {
            return Ok(vec![]);
        }
        let mut migrated = vec![];
        for id in profile_ids {
            let mut any = false;
            for kind in &["password", "passphrase"] {
                let key = credential_key(id, kind);
                let secret = match self.mode() {
                    StorageMode::Keyring => self.keyring.get(&key)?,
                    StorageMode::Vault => self.vault.get(&key)?,
                };
                if let Some(s) = secret {
                    any = true;
                    match new_mode {
                        StorageMode::Keyring => self.keyring.store(&key, &s)?,
                        StorageMode::Vault => self.vault.store(&key, &s)?,
                    }
                }
            }
            if any {
                migrated.push(id.clone());
            }
        }
        Ok(migrated)
    }

    pub fn keyring_available() -> bool {
        KeyringBackend::available()
    }

    fn active(&self) -> Result<&dyn CredentialBackend, CoreError> {
        match *self.mode.lock().unwrap() {
            StorageMode::Keyring => Ok(&self.keyring),
            StorageMode::Vault => Ok(&self.vault),
        }
    }
}

pub fn credential_key(profile_id: &str, kind: &str) -> String {
    format!("ssh-profile:{}:{}", profile_id, kind)
}

// ── VaultSettings ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VaultSettings {
    /// Idle timeout in seconds (0 = never)
    pub timeout_secs: u64,
}

impl Default for VaultSettings {
    fn default() -> Self {
        Self { timeout_secs: 15 * 60 }
    }
}
