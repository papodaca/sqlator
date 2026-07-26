//! Shared application service owned by every frontend adapter.
//!
//! Owns config, pools, profile-keyed tunnels, credentials, and the schema TTL
//! cache. Module methods will be filled in by later Phase 1 children; this
//! skeleton only establishes the type and construction path.

use crate::error::ServiceError;
use dashmap::DashMap;
use sqlator_core::config::ConfigManager;
use sqlator_core::credentials::{CredentialStore, StorageMode};
use sqlator_core::db::DbManager;
use sqlator_core::models::TableMeta;
use sqlator_core::ssh::TunnelHandle;
use std::sync::Arc;
use std::time::Instant;

/// Central application layer between `sqlator-core` and frontends.
pub struct AppService {
    pub(crate) config: ConfigManager,
    pub(crate) db: DbManager,
    /// Tunnel registry keyed by **SSH profile id** (refcount wiring comes later).
    pub(crate) tunnels: DashMap<String, TunnelHandle>,
    pub(crate) credentials: Arc<CredentialStore>,
    /// Cache: key = `connection_id:schema:table_name` → `(TableMeta, expiry)`.
    pub(crate) schema_cache: DashMap<String, (TableMeta, Instant)>,
}

impl AppService {
    /// Construct with the same on-disk layout as today's Tauri/web `AppState`.
    pub fn new() -> Result<Self, ServiceError> {
        let config = ConfigManager::new("sqlator")?;

        let vault_path = dirs::config_dir()
            .ok_or_else(|| {
                ServiceError::app("CONFIG_ERROR", "Could not determine config directory")
            })?
            .join("sqlator")
            .join("vault.enc");

        let stored_mode = config.get_storage_mode()?;
        let mode = match stored_mode.as_deref() {
            Some("vault") => StorageMode::Vault,
            Some("keyring") => StorageMode::Keyring,
            _ => {
                if CredentialStore::keyring_available() {
                    StorageMode::Keyring
                } else {
                    StorageMode::Vault
                }
            }
        };

        let credentials = Arc::new(CredentialStore::new(vault_path, mode));
        let timeout = config.get_vault_timeout_secs()?;
        credentials.vault.set_timeout(timeout);

        Ok(Self {
            config,
            db: DbManager::new(),
            tunnels: DashMap::new(),
            credentials,
            schema_cache: DashMap::new(),
        })
    }

    pub fn config(&self) -> &ConfigManager {
        &self.config
    }

    pub fn db(&self) -> &DbManager {
        &self.db
    }

    pub fn tunnels(&self) -> &DashMap<String, TunnelHandle> {
        &self.tunnels
    }

    pub fn credentials(&self) -> &Arc<CredentialStore> {
        &self.credentials
    }

    pub fn schema_cache(&self) -> &DashMap<String, (TableMeta, Instant)> {
        &self.schema_cache
    }
}
