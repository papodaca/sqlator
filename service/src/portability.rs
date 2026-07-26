//! Connection import/export payload builders.
//!
//! The service returns JSON **payload** only; each frontend chooses persistence
//! (Downloads write, temp-file + HTTP download, GTK save dialog, …).

use crate::error::ServiceError;
use serde::{Deserialize, Serialize};
use sqlator_core::models::{ConnectionGroup, SavedConnection, SshProfile};
use std::collections::HashMap;

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
