//! Connection helpers: URL ↔ [`SavedConnection`], naming, and type inference.

use crate::error::ServiceError;
use serde::{Deserialize, Serialize};
use sqlator_core::db::DatabaseType;
use sqlator_core::models::{ConnectionConfig, ConnectionType, SavedConnection};
use std::collections::HashSet;

/// Default TCP port for a stored `db_type` label (`"postgres"`, `"mariadb"`, …).
pub fn default_port_for_db_type(db_type: &str) -> u16 {
    match db_type {
        "postgres" => 5432,
        "mysql" | "mariadb" => 3306,
        "mssql" => 1433,
        "oracle" => 1521,
        "clickhouse" => 8123,
        _ => 0,
    }
}

/// Always returns `"base (N)"` — never `base` itself, even when free.
pub fn unique_name(base: &str, existing: &HashSet<String>) -> String {
    let mut i = 1u32;
    loop {
        let candidate = format!("{base} ({i})");
        if !existing.contains(&candidate) {
            return candidate;
        }
        i += 1;
    }
}

/// Build a connection URL without embedding a password.
pub fn build_url_no_password(
    db_type: &str,
    host: &str,
    port: u16,
    database: &str,
    username: &str,
) -> String {
    match db_type {
        "sqlite" => format!("sqlite://{database}"),
        _ => {
            let user = if !username.is_empty() {
                format!("{username}@")
            } else {
                String::new()
            };
            format!("{db_type}://{user}{host}:{port}/{database}")
        }
    }
}

/// Prefer an explicit `connection_type`; otherwise infer from SSH / container fields.
pub fn resolve_connection_type(config: &ConnectionConfig) -> ConnectionType {
    match config.connection_type {
        Some(ref ct) => ct.clone(),
        None => {
            if config.container_name.is_some() && config.ssh_profile_id.is_some() {
                ConnectionType::DockerContainer
            } else if config.ssh_profile_id.is_some() {
                ConnectionType::SshTunnel
            } else {
                ConnectionType::Direct
            }
        }
    }
}

/// Map a URL to the stored `db_type` label (keeps `mariadb` distinct from `mysql`).
pub fn db_type_from_url(url: &str) -> Result<&'static str, ServiceError> {
    let parsed = url::Url::parse(url)
        .map_err(|e| ServiceError::app("INVALID_URL", format!("Invalid URL: {e}")))?;

    match sqlator_core::detect_database_type(url) {
        Some(DatabaseType::Postgres) => Ok("postgres"),
        Some(DatabaseType::MySql) => {
            if parsed.scheme() == "mariadb" {
                Ok("mariadb")
            } else {
                Ok("mysql")
            }
        }
        Some(DatabaseType::Sqlite) => Ok("sqlite"),
        Some(DatabaseType::Mssql) => Ok("mssql"),
        Some(DatabaseType::Oracle) => Ok("oracle"),
        Some(DatabaseType::ClickHouse) => Ok("clickhouse"),
        None => Err(ServiceError::app(
            "UNSUPPORTED_SCHEME",
            format!("Unsupported database scheme: {}", parsed.scheme()),
        )),
    }
}

/// Parsed connection URL fields for the connection editor / save path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParsedConnectionUrl {
    pub db_type: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: Option<String>,
}

/// Parse a URL into connection-editor fields (scheme → db_type + default port).
pub fn parse_connection_url(url: &str) -> Result<ParsedConnectionUrl, ServiceError> {
    let parsed = url::Url::parse(url)
        .map_err(|e| ServiceError::app("INVALID_URL", format!("Invalid URL: {e}")))?;
    let db_type = db_type_from_url(url)?;
    Ok(ParsedConnectionUrl {
        db_type: db_type.to_string(),
        host: parsed.host_str().unwrap_or("localhost").to_string(),
        port: parsed
            .port()
            .unwrap_or_else(|| default_port_for_db_type(db_type)),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        password: parsed.password().map(|p| p.to_string()),
    })
}

/// Build a [`SavedConnection`] from editor config (shared Tauri/web save+update path).
pub fn build_saved_connection(
    id: String,
    config: ConnectionConfig,
) -> Result<SavedConnection, ServiceError> {
    let parsed = url::Url::parse(&config.url)
        .map_err(|e| ServiceError::app("INVALID_URL", format!("Invalid URL: {e}")))?;
    let db_type = db_type_from_url(&config.url)?;
    let connection_type = resolve_connection_type(&config);

    Ok(SavedConnection {
        id,
        name: config.name,
        color_id: config.color_id,
        db_type: db_type.to_string(),
        host: parsed.host_str().unwrap_or("localhost").to_string(),
        port: parsed
            .port()
            .unwrap_or_else(|| default_port_for_db_type(db_type)),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        url: config.url,
        ssh_profile_id: config.ssh_profile_id,
        group_id: config.group_id,
        connection_type,
        container_name: config.container_name,
        container_port: config.container_port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_name_never_returns_base_even_when_free() {
        let existing = HashSet::new();
        assert_eq!(unique_name("alpha", &existing), "alpha (1)");
    }

    #[test]
    fn unique_name_skips_taken_suffixes() {
        let existing: HashSet<String> = ["alpha (1)".into(), "alpha (2)".into()].into();
        assert_eq!(unique_name("alpha", &existing), "alpha (3)");
    }

    #[test]
    fn unique_name_ignores_whether_base_itself_is_taken() {
        let existing: HashSet<String> = ["alpha".into()].into();
        assert_eq!(unique_name("alpha", &existing), "alpha (1)");
    }

    #[test]
    fn build_url_no_password_all_engines() {
        assert_eq!(
            build_url_no_password("postgres", "h", 5432, "db", "u"),
            "postgres://u@h:5432/db"
        );
        assert_eq!(
            build_url_no_password("mysql", "h", 3306, "db", "u"),
            "mysql://u@h:3306/db"
        );
        assert_eq!(
            build_url_no_password("mariadb", "h", 3306, "db", "u"),
            "mariadb://u@h:3306/db"
        );
        assert_eq!(
            build_url_no_password("mssql", "h", 1433, "db", "u"),
            "mssql://u@h:1433/db"
        );
        assert_eq!(
            build_url_no_password("oracle", "h", 1521, "db", "u"),
            "oracle://u@h:1521/db"
        );
        assert_eq!(
            build_url_no_password("clickhouse", "h", 8123, "db", "u"),
            "clickhouse://u@h:8123/db"
        );
        assert_eq!(
            build_url_no_password("sqlite", "ignored", 0, "/tmp/x.db", "u"),
            "sqlite:///tmp/x.db"
        );
    }

    #[test]
    fn build_url_no_password_empty_username_omits_at() {
        assert_eq!(
            build_url_no_password("postgres", "h", 5432, "db", ""),
            "postgres://h:5432/db"
        );
    }

    fn conn_cfg(
        connection_type: Option<ConnectionType>,
        ssh_profile_id: Option<&str>,
        container_name: Option<&str>,
    ) -> ConnectionConfig {
        ConnectionConfig {
            name: "n".into(),
            color_id: "c".into(),
            url: "postgres://h/db".into(),
            ssh_profile_id: ssh_profile_id.map(str::to_string),
            group_id: None,
            connection_type,
            container_name: container_name.map(str::to_string),
            container_port: None,
        }
    }

    #[test]
    fn resolve_connection_type_explicit_wins() {
        let cfg = conn_cfg(Some(ConnectionType::Direct), Some("ssh"), Some("ctr"));
        assert_eq!(resolve_connection_type(&cfg), ConnectionType::Direct);

        let cfg = conn_cfg(Some(ConnectionType::LocalDockerContainer), None, None);
        assert_eq!(
            resolve_connection_type(&cfg),
            ConnectionType::LocalDockerContainer
        );
    }

    #[test]
    fn resolve_connection_type_inference_paths() {
        let cfg = conn_cfg(None, Some("ssh"), Some("ctr"));
        assert_eq!(
            resolve_connection_type(&cfg),
            ConnectionType::DockerContainer
        );

        let cfg = conn_cfg(None, Some("ssh"), None);
        assert_eq!(resolve_connection_type(&cfg), ConnectionType::SshTunnel);

        let cfg = conn_cfg(None, None, None);
        assert_eq!(resolve_connection_type(&cfg), ConnectionType::Direct);

        // container alone does NOT infer LocalDockerContainer — falls through to Direct
        let cfg = conn_cfg(None, None, Some("ctr"));
        assert_eq!(resolve_connection_type(&cfg), ConnectionType::Direct);
    }

    #[test]
    fn default_port_for_db_type_known_and_unknown() {
        assert_eq!(default_port_for_db_type("postgres"), 5432);
        assert_eq!(default_port_for_db_type("mysql"), 3306);
        assert_eq!(default_port_for_db_type("mariadb"), 3306);
        assert_eq!(default_port_for_db_type("mssql"), 1433);
        assert_eq!(default_port_for_db_type("oracle"), 1521);
        assert_eq!(default_port_for_db_type("clickhouse"), 8123);
        assert_eq!(default_port_for_db_type("sqlite"), 0);
        assert_eq!(default_port_for_db_type("nope"), 0);
    }

    #[test]
    fn db_type_from_url_mariadb_vs_mysql() {
        assert_eq!(db_type_from_url("mysql://h/db").unwrap(), "mysql");
        assert_eq!(db_type_from_url("mariadb://h/db").unwrap(), "mariadb");
        assert_eq!(db_type_from_url("postgres://h/db").unwrap(), "postgres");
    }

    #[test]
    fn build_saved_connection_fills_defaults() {
        let cfg = ConnectionConfig {
            name: "n".into(),
            color_id: "c".into(),
            url: "postgres://alice@db.example/app".into(),
            ssh_profile_id: None,
            group_id: None,
            connection_type: None,
            container_name: None,
            container_port: None,
        };
        let saved = build_saved_connection("id-1".into(), cfg).unwrap();
        assert_eq!(saved.id, "id-1");
        assert_eq!(saved.db_type, "postgres");
        assert_eq!(saved.host, "db.example");
        assert_eq!(saved.port, 5432);
        assert_eq!(saved.database, "app");
        assert_eq!(saved.username, "alice");
        assert_eq!(saved.connection_type, ConnectionType::Direct);
    }
}
