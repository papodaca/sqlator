use dashmap::DashMap;
use sqlator_core::config::ConfigManager;
use sqlator_core::credentials::{CredentialStore, StorageMode};
use sqlator_core::db::DbManager;
use sqlator_core::models::TableMeta;
use sqlator_core::ssh::TunnelHandle;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// Fixed connection ID used in single-db mode.
pub const SINGLE_DB_CONN_ID: &str = "__single_db__";

/// Populated when the server starts with `-c <config-file>`.
#[derive(Debug, Clone)]
pub struct SingleDbConfig {
    pub connection_id: String,
    pub connection_name: String,
    pub url: String,
}

pub struct AppState {
    /// Wrapped in a Mutex because ConfigManager reads/writes to disk and is
    /// not internally synchronized.
    pub config: Mutex<ConfigManager>,
    pub db: DbManager,
    pub tunnels: DashMap<String, TunnelHandle>,
    pub credentials: Arc<CredentialStore>,
    /// Cache: key = "connection_id:schema:table_name" → (TableMeta, expiry)
    pub schema_cache: DashMap<String, (TableMeta, Instant)>,
    /// Some(_) when started with `-c <file>` (single-database admin mode).
    pub single_db: Option<SingleDbConfig>,
}

impl AppState {
    pub fn new() -> Result<Self, sqlator_core::error::CoreError> {
        Self::new_inner(None)
    }

    /// Create state pre-wired to a single database.
    /// The database pool is connected eagerly so the first page load is instant.
    pub async fn new_with_single_db(url: String, name: String) -> Result<Self, sqlator_core::error::CoreError> {
        let cfg = SingleDbConfig {
            connection_id: SINGLE_DB_CONN_ID.to_string(),
            connection_name: name,
            url: url.clone(),
        };
        let state = Self::new_inner(Some(cfg))?;
        // Pre-connect so the first query has no cold-start delay
        state.db.connect(SINGLE_DB_CONN_ID, &url).await?;
        Ok(state)
    }

    fn new_inner(single_db: Option<SingleDbConfig>) -> Result<Self, sqlator_core::error::CoreError> {
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
            single_db,
        })
    }
}

/// Parse a connection URL from a config file.
/// Supports:
///   - JSON: `{"url": "...", "name": "..."}` (name optional)
///   - YAML-style line:  `url: postgres://...`
///   - Plain text: the URL itself
pub fn parse_config_file(path: &std::path::Path) -> Result<(String, String), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read config file '{}': {}", path.display(), e))?;
    let content = content.trim();

    // Try JSON first
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(content) {
        if let Some(url) = v.get("url").and_then(|u| u.as_str()) {
            let name = v
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("Database")
                .to_string();
            return Ok((url.to_string(), name));
        }
        return Err("JSON config file must contain a 'url' field".into());
    }

    // Try YAML-style `url: <value>` line
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("url:") {
            let url = rest.trim().trim_matches('"').trim_matches('\'');
            if !url.is_empty() {
                return Ok((url.to_string(), "Database".to_string()));
            }
        }
        if let Some(rest) = line.strip_prefix("name:") {
            // will be overridden if found after url: in a full scan
            let _ = rest.trim();
        }
    }

    // Also try a two-pass YAML scan for both url and name
    let mut url_found: Option<String> = None;
    let mut name_found: Option<String> = None;
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("url:") {
            url_found = Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
        if let Some(rest) = line.strip_prefix("name:") {
            name_found = Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    if let Some(url) = url_found {
        return Ok((url, name_found.unwrap_or_else(|| "Database".to_string())));
    }

    // Treat entire content as a raw URL
    if content.contains("://") {
        return Ok((content.to_string(), "Database".to_string()));
    }

    Err(format!(
        "Could not parse '{}' as a connection config. \
         Expected JSON {{\"url\": \"...\"}}, YAML with 'url:' field, or a bare connection URL.",
        path.display()
    ))
}
