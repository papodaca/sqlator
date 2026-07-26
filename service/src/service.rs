//! Shared application service owned by every frontend adapter.
//!
//! Owns config, pools, profile+target tunnels, credentials, and the schema TTL
//! cache.

use crate::error::ServiceError;
use crate::tunnels::{ManagedTunnel, TunnelKey};
use dashmap::DashMap;
use sqlator_core::config::ConfigManager;
use sqlator_core::credentials::{CredentialStore, StorageMode};
use sqlator_core::db::DbManager;
use sqlator_core::models::TableMeta;
use std::sync::Arc;
use std::time::Instant;

/// Central application layer between `sqlator-core` and frontends.
pub struct AppService {
    pub(crate) config: ConfigManager,
    pub(crate) db: DbManager,
    /// Tunnel registry keyed by [`TunnelKey`] `(profile_id, target_host, target_port)`.
    pub(crate) tunnels: DashMap<TunnelKey, ManagedTunnel>,
    /// Reverse index: connection id → registry key (for disconnect / VTE lookup).
    pub(crate) connection_tunnels: DashMap<String, TunnelKey>,
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
            connection_tunnels: DashMap::new(),
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

    pub fn tunnels(&self) -> &DashMap<TunnelKey, ManagedTunnel> {
        &self.tunnels
    }

    pub fn credentials(&self) -> &Arc<CredentialStore> {
        &self.credentials
    }

    pub fn schema_cache(&self) -> &DashMap<String, (TableMeta, Instant)> {
        &self.schema_cache
    }
}
