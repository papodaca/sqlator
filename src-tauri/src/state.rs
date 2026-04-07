use dashmap::DashMap;
use sqlator_core::config::ConfigManager;
use sqlator_core::credentials::{CredentialStore, StorageMode};
use sqlator_core::db::DbManager;
use sqlator_core::models::TableMeta;
use sqlator_core::ssh::TunnelHandle;
use std::sync::Arc;
use std::time::Instant;

pub struct AppState {
    pub config: ConfigManager,
    pub db: DbManager,
    pub tunnels: DashMap<String, TunnelHandle>,
    pub credentials: Arc<CredentialStore>,
    /// Cache: key = "connection_id:schema:table_name" → (TableMeta, expiry)
    pub schema_cache: DashMap<String, (TableMeta, Instant)>,
}

impl AppState {
    pub fn new() -> Result<Self, sqlator_core::error::CoreError> {
        let config = ConfigManager::new("sqlator")?;

        // Resolve vault path: same directory as connections.json
        let vault_path = dirs::config_dir()
            .expect("cannot resolve config dir")
            .join("sqlator")
            .join("vault.enc");

        // Determine active storage mode from persisted setting
        let stored_mode = config.get_storage_mode()?;
        let mode = match stored_mode.as_deref() {
            Some("vault") => StorageMode::Vault,
            Some("keyring") => StorageMode::Keyring,
            // Auto-detect: prefer keyring when available, else vault
            _ => {
                if CredentialStore::keyring_available() {
                    StorageMode::Keyring
                } else {
                    StorageMode::Vault
                }
            }
        };

        let credentials = Arc::new(CredentialStore::new(vault_path, mode));

        // Apply persisted vault timeout
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
}
