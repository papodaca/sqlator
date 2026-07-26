//! Connection-set abstraction (multi-db store vs fixed singleton).
//!
//! Resolves web `single_db` / future GTK `--config` without baking policy into
//! each frontend.

use crate::error::ServiceError;
use sqlator_core::config::ConfigManager;
use sqlator_core::models::{ConnectionType, SavedConnection};
use std::sync::Arc;

/// Fixed connection ID used in single-db / `--config` mode.
pub const SINGLE_DB_CONN_ID: &str = "__single_db__";

/// Metadata exposed to frontends for single-db server-info / UI chrome.
#[derive(Debug, Clone)]
pub struct SingleDbInfo {
    pub connection_id: String,
    pub connection_name: String,
    pub url: String,
}

/// Source of truth for which connections the service can list and mutate.
pub trait ConnectionSource: Send + Sync {
    /// All connections visible in the current mode.
    fn list_connections(&self) -> Result<Vec<SavedConnection>, ServiceError>;

    /// Look up one connection by id.
    fn get_connection(&self, id: &str) -> Result<Option<SavedConnection>, ServiceError>;

    /// Whether connection CRUD (add/remove/import) is allowed.
    ///
    /// Single-db / fixed-config mode returns `false`.
    fn allows_mutation(&self) -> bool;

    /// Guard used by multi-db-only operations (`require_multi_db`).
    fn require_multi_db(&self) -> Result<(), ServiceError> {
        if self.allows_mutation() {
            Ok(())
        } else {
            Err(ServiceError::app(
                "MULTI_DB_REQUIRED",
                "This operation is not available in single-database mode",
            ))
        }
    }

    /// Optional single-db chrome metadata.
    fn single_db_info(&self) -> Option<&SingleDbInfo> {
        None
    }
}

/// Normal multi-db mode backed by on-disk `ConfigManager`.
pub struct ConfigConnectionSource {
    config: Arc<ConfigManager>,
}

impl ConfigConnectionSource {
    pub fn new(config: Arc<ConfigManager>) -> Self {
        Self { config }
    }
}

impl ConnectionSource for ConfigConnectionSource {
    fn list_connections(&self) -> Result<Vec<SavedConnection>, ServiceError> {
        Ok(self.config.get_connections()?)
    }

    fn get_connection(&self, id: &str) -> Result<Option<SavedConnection>, ServiceError> {
        Ok(self
            .config
            .get_connections()?
            .into_iter()
            .find(|c| c.id == id))
    }

    fn allows_mutation(&self) -> bool {
        true
    }
}

/// Fixed synthetic singleton (web `-c` / future GTK `--config`).
pub struct SingleDbConnectionSource {
    info: SingleDbInfo,
    connection: SavedConnection,
}

impl SingleDbConnectionSource {
    pub fn new(url: String, name: String) -> Result<Self, ServiceError> {
        let connection = synthetic_saved_connection(SINGLE_DB_CONN_ID, &name, &url)?;
        Ok(Self {
            info: SingleDbInfo {
                connection_id: SINGLE_DB_CONN_ID.to_string(),
                connection_name: name,
                url,
            },
            connection,
        })
    }
}

impl ConnectionSource for SingleDbConnectionSource {
    fn list_connections(&self) -> Result<Vec<SavedConnection>, ServiceError> {
        Ok(vec![self.connection.clone()])
    }

    fn get_connection(&self, id: &str) -> Result<Option<SavedConnection>, ServiceError> {
        if id == self.connection.id {
            Ok(Some(self.connection.clone()))
        } else {
            Ok(None)
        }
    }

    fn allows_mutation(&self) -> bool {
        false
    }

    fn single_db_info(&self) -> Option<&SingleDbInfo> {
        Some(&self.info)
    }
}

/// Build a [`SavedConnection`] from a raw URL for single-db mode.
pub fn synthetic_saved_connection(
    id: &str,
    name: &str,
    url: &str,
) -> Result<SavedConnection, ServiceError> {
    let parsed = url::Url::parse(url)
        .map_err(|e| ServiceError::app("INVALID_URL", format!("Invalid connection URL: {e}")))?;
    let db_type = match parsed.scheme() {
        "postgres" | "postgresql" => "postgres",
        "mysql" => "mysql",
        "mariadb" => "mariadb",
        "sqlite" => "sqlite",
        "mssql" | "sqlserver" | "tds" => "mssql",
        "oracle" => "oracle",
        "clickhouse" => "clickhouse",
        other => other,
    };
    Ok(SavedConnection {
        id: id.to_string(),
        name: name.to_string(),
        color_id: "blue".to_string(),
        db_type: db_type.to_string(),
        host: parsed.host_str().unwrap_or("localhost").to_string(),
        port: parsed.port().unwrap_or(0),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        url: url.to_string(),
        ssh_profile_id: None,
        group_id: None,
        connection_type: ConnectionType::Direct,
        container_name: None,
        container_port: None,
    })
}
