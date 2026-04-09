use dashmap::DashMap;
use sqlator_core::config::ConfigManager;
use sqlator_core::credentials::{CredentialStore, StorageMode};
use sqlator_core::db::DbManager;
use sqlator_core::models::TableMeta;
use sqlator_core::ssh::TunnelHandle;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

pub struct AppState {
    /// Wrapped in a Mutex because ConfigManager reads/writes to disk and is
    /// not internally synchronized. Multiple concurrent requests could race
    /// without this.
    pub config: Mutex<ConfigManager>,
    pub db: DbManager,
    pub tunnels: DashMap<String, TunnelHandle>,
    pub credentials: Arc<CredentialStore>,
    /// Cache: key = "connection_id:schema:table_name" → (TableMeta, expiry)
    pub schema_cache: DashMap<String, (TableMeta, Instant)>,
}

impl AppState {
    pub fn new() -> Result<Self, sqlator_core::error::CoreError> {
        let config = ConfigManager::new("sqlator")?;

        let vault_path = dirs::config_dir()
            .expect("cannot resolve config dir")
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
            config: Mutex::new(config),
            db: DbManager::new(),
            tunnels: DashMap::new(),
            credentials,
            schema_cache: DashMap::new(),
        })
    }
}
