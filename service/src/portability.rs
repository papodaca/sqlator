//! Connection import/export payload builders.
//!
//! The service returns JSON **payload** only; each frontend chooses persistence
//! (Downloads write, temp-file + HTTP download, GTK save dialog, …).

use crate::error::ServiceError;
use serde::{Deserialize, Serialize};
use sqlator_core::models::{
    ConnectionGroup, ConnectionType, SavedConnection, SshJumpHost, SshProfile,
};
use std::collections::{HashMap, HashSet};

/// Portable representation of a connection (no secrets).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedConnection {
    pub name: String,
    pub color_id: String,
    pub db_type: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    /// Name of the linked SSH profile (resolved by name on import).
    pub ssh_profile_name: Option<String>,
    /// Name of the group (resolved by name on import).
    pub group_name: Option<String>,
}

/// Portable SSH profile — no password or passphrase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedSshProfile {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub key_path: Option<String>,
    pub proxy_jump: Vec<ExportedJumpHost>,
    pub local_port_binding: Option<u16>,
    pub keepalive_interval: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedJumpHost {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub key_path: Option<String>,
}

/// Portable group (parent resolved by name).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedGroup {
    pub name: String,
    pub color: Option<String>,
    pub parent_group_name: Option<String>,
    pub order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportFile {
    pub version: String,
    pub exported_at: String,
    pub connections: Vec<ExportedConnection>,
    pub ssh_profiles: Vec<ExportedSshProfile>,
    pub groups: Vec<ExportedGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub groups_added: usize,
    pub profiles_added: usize,
    pub connections_added: usize,
    pub connections_skipped: usize,
}

/// Build the export JSON string from already-loaded config slices (no I/O).
pub fn build_export_json(
    connections: &[SavedConnection],
    profiles: &[SshProfile],
    groups: &[ConnectionGroup],
) -> Result<String, ServiceError> {
    let profile_names: HashMap<String, String> = profiles
        .iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();
    let group_names: HashMap<String, String> = groups
        .iter()
        .map(|g| (g.id.clone(), g.name.clone()))
        .collect();

    let exported_connections: Vec<ExportedConnection> = connections
        .iter()
        .map(|c| ExportedConnection {
            name: c.name.clone(),
            color_id: c.color_id.clone(),
            db_type: c.db_type.clone(),
            host: c.host.clone(),
            port: c.port,
            database: c.database.clone(),
            username: c.username.clone(),
            ssh_profile_name: c
                .ssh_profile_id
                .as_ref()
                .and_then(|id| profile_names.get(id))
                .cloned(),
            group_name: c
                .group_id
                .as_ref()
                .and_then(|id| group_names.get(id))
                .cloned(),
        })
        .collect();

    let exported_profiles: Vec<ExportedSshProfile> = profiles
        .iter()
        .map(|p| ExportedSshProfile {
            name: p.name.clone(),
            host: p.host.clone(),
            port: p.port,
            username: p.username.clone(),
            auth_method: format!("{:?}", p.auth_method).to_lowercase(),
            key_path: p.key_path.clone(),
            proxy_jump: p
                .proxy_jump
                .iter()
                .map(|j| ExportedJumpHost {
                    host: j.host.clone(),
                    port: j.port,
                    username: j.username.clone(),
                    auth_method: format!("{:?}", j.auth_method).to_lowercase(),
                    key_path: j.key_path.clone(),
                })
                .collect(),
            local_port_binding: p.local_port_binding,
            keepalive_interval: p.keepalive_interval,
        })
        .collect();

    let exported_groups: Vec<ExportedGroup> = groups
        .iter()
        .map(|g| ExportedGroup {
            name: g.name.clone(),
            color: g.color.clone(),
            parent_group_name: g
                .parent_group_id
                .as_ref()
                .and_then(|id| group_names.get(id))
                .cloned(),
            order: g.order,
        })
        .collect();

    let export_file = ExportFile {
        version: "1.0".to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        connections: exported_connections,
        ssh_profiles: exported_profiles,
        groups: exported_groups,
    };

    serde_json::to_string_pretty(&export_file)
        .map_err(|e| ServiceError::app("EXPORT_SERIALIZE", e.to_string()))
}

/// Parse an export payload into [`ExportFile`].
pub fn parse_export_json(json: &str) -> Result<ExportFile, ServiceError> {
    serde_json::from_str(json).map_err(|e| ServiceError::app("EXPORT_PARSE", e.to_string()))
}

// ── AppService import / export ────────────────────────────────────────────────

use crate::connections::{build_url_no_password, unique_name};
use crate::service::{run_blocking, AppService};
use crate::ssh::parse_auth_method;
use std::sync::Arc;

impl AppService {
    /// Build export JSON payload (frontend chooses where to persist the bytes).
    pub async fn export_connections_json(&self) -> Result<String, ServiceError> {
        let config = Arc::clone(&self.config);
        run_blocking(move || {
            let connections = config.get_connections()?;
            let profiles = config.get_ssh_profiles()?;
            let groups = config.get_groups()?;
            build_export_json(&connections, &profiles, &groups)
        })
        .await
    }

    /// Apply an export payload (`duplicate_mode`: `"skip"` or `"rename"`).
    pub async fn import_connections(
        &self,
        json: String,
        duplicate_mode: String,
    ) -> Result<ImportResult, ServiceError> {
        self.require_multi_db()?;
        let config = Arc::clone(&self.config);
        run_blocking(move || apply_import(&config, &json, &duplicate_mode)).await
    }
}

fn apply_import(
    config: &sqlator_core::config::ConfigManager,
    json: &str,
    duplicate_mode: &str,
) -> Result<ImportResult, ServiceError> {
    let file = parse_export_json(json)?;
    let rename = duplicate_mode == "rename";

    // ── 1. Import groups (roots first, then children) ─────────────────────────
    let existing_groups = config.get_groups()?;
    let existing_group_names: HashSet<String> =
        existing_groups.iter().map(|g| g.name.clone()).collect();

    let mut group_id_map: HashMap<String, String> = existing_groups
        .iter()
        .map(|g| (g.name.clone(), g.id.clone()))
        .collect();

    let mut groups_added = 0usize;

    let mut remaining: Vec<&ExportedGroup> = file.groups.iter().collect();
    for _ in 0..3 {
        let mut next_remaining = Vec::new();
        for eg in remaining {
            if let Some(ref parent_name) = eg.parent_group_name {
                if !group_id_map.contains_key(parent_name.as_str()) {
                    next_remaining.push(eg);
                    continue;
                }
            }
            if existing_group_names.contains(&eg.name) {
                continue;
            }
            let new_id = uuid::Uuid::new_v4().to_string();
            let group = ConnectionGroup {
                id: new_id.clone(),
                name: eg.name.clone(),
                color: eg.color.clone(),
                parent_group_id: eg
                    .parent_group_name
                    .as_ref()
                    .and_then(|n| group_id_map.get(n))
                    .cloned(),
                order: eg.order,
                collapsed: false,
            };
            config.save_group(group)?;
            group_id_map.insert(eg.name.clone(), new_id);
            groups_added += 1;
        }
        remaining = next_remaining;
        if remaining.is_empty() {
            break;
        }
    }

    // ── 2. Import SSH profiles ────────────────────────────────────────────────
    let existing_profiles = config.get_ssh_profiles()?;
    let existing_profile_names: HashSet<String> =
        existing_profiles.iter().map(|p| p.name.clone()).collect();

    let mut profile_id_map: HashMap<String, String> = existing_profiles
        .iter()
        .map(|p| (p.name.clone(), p.id.clone()))
        .collect();

    let mut profiles_added = 0usize;

    for ep in &file.ssh_profiles {
        let final_name = if existing_profile_names.contains(&ep.name) {
            if !rename {
                continue;
            }
            unique_name(&ep.name, &profile_id_map.keys().cloned().collect())
        } else {
            ep.name.clone()
        };

        let new_id = uuid::Uuid::new_v4().to_string();
        let auth_method = parse_auth_method(&ep.auth_method)?;
        let profile = SshProfile {
            id: new_id.clone(),
            name: final_name.clone(),
            host: ep.host.clone(),
            port: ep.port,
            username: ep.username.clone(),
            auth_method,
            key_path: ep.key_path.clone(),
            proxy_jump: ep
                .proxy_jump
                .iter()
                .map(|j| SshJumpHost {
                    host: j.host.clone(),
                    port: j.port,
                    username: j.username.clone(),
                    auth_method: parse_auth_method(&j.auth_method)
                        .unwrap_or(sqlator_core::models::SshAuthMethod::Key),
                    key_path: j.key_path.clone(),
                })
                .collect(),
            local_port_binding: ep.local_port_binding,
            keepalive_interval: ep.keepalive_interval,
        };
        config.save_ssh_profile(profile)?;
        profile_id_map.insert(ep.name.clone(), new_id);
        profiles_added += 1;
    }

    // ── 3. Import connections ─────────────────────────────────────────────────
    let existing_connections = config.get_connections()?;
    let existing_conn_names: HashSet<String> = existing_connections
        .iter()
        .map(|c| c.name.clone())
        .collect();

    let mut all_conn_names: HashSet<String> = existing_conn_names.clone();
    let mut connections_added = 0usize;
    let mut connections_skipped = 0usize;

    for ec in &file.connections {
        let final_name = if existing_conn_names.contains(&ec.name) {
            if !rename {
                connections_skipped += 1;
                continue;
            }
            unique_name(&ec.name, &all_conn_names)
        } else {
            ec.name.clone()
        };

        let url = build_url_no_password(&ec.db_type, &ec.host, ec.port, &ec.database, &ec.username);

        let conn = SavedConnection {
            id: uuid::Uuid::new_v4().to_string(),
            name: final_name.clone(),
            color_id: ec.color_id.clone(),
            db_type: ec.db_type.clone(),
            host: ec.host.clone(),
            port: ec.port,
            database: ec.database.clone(),
            username: ec.username.clone(),
            url,
            ssh_profile_id: ec
                .ssh_profile_name
                .as_ref()
                .and_then(|n| profile_id_map.get(n))
                .cloned(),
            group_id: ec
                .group_name
                .as_ref()
                .and_then(|n| group_id_map.get(n))
                .cloned(),
            connection_type: ConnectionType::default(),
            container_name: None,
            container_port: None,
        };
        config.save_connection(conn)?;
        all_conn_names.insert(final_name);
        connections_added += 1;
    }

    Ok(ImportResult {
        groups_added,
        profiles_added,
        connections_added,
        connections_skipped,
    })
}
