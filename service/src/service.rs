//! Shared application service owned by every frontend adapter.
//!
//! Owns config, pools, shared SSH sessions / per-target forwards, credentials,
//! and the schema TTL cache.

use crate::connection_source::{
    ConfigConnectionSource, ConnectionSource, SingleDbConnectionSource, SingleDbInfo,
};
use crate::error::ServiceError;
use crate::tunnels::{ManagedForward, ManagedSession, TunnelKey};
use dashmap::DashMap;
use sqlator_core::config::ConfigManager;
use sqlator_core::credentials::{CredentialStore, StorageMode};
use sqlator_core::db::DbManager;
use sqlator_core::models::TableMeta;
use std::sync::Arc;
use std::time::Instant;

/// Central application layer between `sqlator-core` and frontends.
pub struct AppService {
    pub(crate) config: Arc<ConfigManager>,
    pub(crate) db: Arc<DbManager>,
    /// Authenticated SSH sessions keyed by profile id (shared across targets).
    pub(crate) sessions: DashMap<String, ManagedSession>,
    /// Local forwards keyed by [`TunnelKey`] `(profile_id, target_host, target_port)`.
    pub(crate) tunnels: DashMap<TunnelKey, ManagedForward>,
    /// Reverse index: connection id → registry key (for disconnect / VTE lookup).
    pub(crate) connection_tunnels: DashMap<String, TunnelKey>,
    pub(crate) credentials: Arc<CredentialStore>,
    /// Cache: key = `connection_id:schema:table_name` → `(TableMeta, expiry)`.
    pub(crate) schema_cache: DashMap<String, (TableMeta, Instant)>,
    /// Multi-db config store or fixed single-db singleton.
    pub(crate) connection_source: Arc<dyn ConnectionSource>,
}

impl AppService {
    /// Construct with the same on-disk layout as today's Tauri/web `AppState`.
    pub fn new() -> Result<Self, ServiceError> {
        Self::with_app_name("sqlator")
    }

    /// Construct using a custom config app name (isolated fixtures / tests).
    pub fn with_app_name(app_name: &str) -> Result<Self, ServiceError> {
        let config = Arc::new(ConfigManager::new(app_name)?);
        let connection_source: Arc<dyn ConnectionSource> =
            Arc::new(ConfigConnectionSource::new(Arc::clone(&config)));
        Self::from_parts(config, connection_source)
    }

    /// Single-database mode (web `-c` / future GTK `--config`).
    ///
    /// Pre-connects the pool so the first page load has no cold-start delay.
    pub async fn with_single_db(url: String, name: String) -> Result<Self, ServiceError> {
        let config = Arc::new(ConfigManager::new("sqlator")?);
        let source = SingleDbConnectionSource::new(url.clone(), name)?;
        let connection_id = source
            .single_db_info()
            .expect("single-db source always has info")
            .connection_id
            .clone();
        let connection_source: Arc<dyn ConnectionSource> = Arc::new(source);
        let service = Self::from_parts(config, connection_source)?;
        service.db.connect(&connection_id, &url).await?;
        Ok(service)
    }

    fn from_parts(
        config: Arc<ConfigManager>,
        connection_source: Arc<dyn ConnectionSource>,
    ) -> Result<Self, ServiceError> {
        let vault_path = config.vault_path();

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
            db: Arc::new(DbManager::new()),
            sessions: DashMap::new(),
            tunnels: DashMap::new(),
            connection_tunnels: DashMap::new(),
            credentials,
            schema_cache: DashMap::new(),
            connection_source,
        })
    }

    pub fn config(&self) -> &ConfigManager {
        &self.config
    }

    pub fn db(&self) -> &DbManager {
        &self.db
    }

    /// Cloneable handle for concurrent execute + cancel tasks.
    pub fn db_handle(&self) -> Arc<DbManager> {
        Arc::clone(&self.db)
    }

    pub fn sessions(&self) -> &DashMap<String, ManagedSession> {
        &self.sessions
    }

    pub fn tunnels(&self) -> &DashMap<TunnelKey, ManagedForward> {
        &self.tunnels
    }

    pub fn credentials(&self) -> &Arc<CredentialStore> {
        &self.credentials
    }

    pub fn schema_cache(&self) -> &DashMap<String, (TableMeta, Instant)> {
        &self.schema_cache
    }

    pub fn connection_source(&self) -> &dyn ConnectionSource {
        self.connection_source.as_ref()
    }

    /// Single-db chrome metadata when running with a fixed connection source.
    pub fn single_db_info(&self) -> Option<&SingleDbInfo> {
        self.connection_source.single_db_info()
    }

    /// Guard for connection CRUD / import.
    pub fn require_multi_db(&self) -> Result<(), ServiceError> {
        self.connection_source.require_multi_db()
    }
}

/// Run blocking config/vault work off the async runtime.
pub(crate) async fn run_blocking<T, F>(f: F) -> Result<T, ServiceError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ServiceError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ServiceError::app("BLOCKING_JOIN", format!("background task failed: {e}")))?
}
