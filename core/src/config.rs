use crate::error::CoreError;
use crate::models::{ConnectionGroup, SavedConnection, SshProfile};
use std::collections::HashMap;
use std::path::PathBuf;

/// File-based configuration manager.
/// Stores connection metadata as JSON on disk.
/// Framework-agnostic — works for both Tauri and TUI.
pub struct ConfigManager {
    config_path: PathBuf,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct ConfigData {
    connections: HashMap<String, SavedConnection>,
    /// Per-connection last query text
    queries: HashMap<String, String>,
    /// Theme preference: "light", "dark", or "system"
    theme: Option<String>,
    /// SSH profiles (credentials stored separately in keyring)
    #[serde(default)]
    ssh_profiles: HashMap<String, SshProfile>,
    /// Credential storage mode: "keyring" or "vault"
    #[serde(default)]
    storage_mode: Option<String>,
    /// Vault idle timeout in seconds (0 = never)
    #[serde(default)]
    vault_timeout_secs: Option<u64>,
    /// Connection groups
    #[serde(default)]
    groups: HashMap<String, ConnectionGroup>,
}

impl ConfigManager {
    pub fn new(app_name: &str) -> Result<Self, CoreError> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| CoreError {
                message: "Could not determine config directory".into(),
                code: "CONFIG_ERROR".into(),
            })?
            .join(app_name);

        std::fs::create_dir_all(&config_dir)?;

        Ok(Self {
            config_path: config_dir.join("connections.json"),
        })
    }

    fn load(&self) -> Result<ConfigData, CoreError> {
        if !self.config_path.exists() {
            return Ok(ConfigData::default());
        }
        let data = std::fs::read_to_string(&self.config_path)?;
        let config: ConfigData = serde_json::from_str(&data)?;
        Ok(config)
    }

    fn save(&self, config: &ConfigData) -> Result<(), CoreError> {
        let data = serde_json::to_string_pretty(config)?;
        std::fs::write(&self.config_path, data)?;
        Ok(())
    }

    pub fn get_connections(&self) -> Result<Vec<SavedConnection>, CoreError> {
        let config = self.load()?;
        Ok(config.connections.values().cloned().collect())
    }

    pub fn save_connection(&self, conn: SavedConnection) -> Result<(), CoreError> {
        let mut config = self.load()?;
        config.connections.insert(conn.id.clone(), conn);
        self.save(&config)
    }

    pub fn update_connection(&self, conn: SavedConnection) -> Result<(), CoreError> {
        let mut config = self.load()?;
        if !config.connections.contains_key(&conn.id) {
            return Err(CoreError {
                message: format!("Connection '{}' not found", conn.id),
                code: "NOT_FOUND".into(),
            });
        }
        config.connections.insert(conn.id.clone(), conn);
        self.save(&config)
    }

    pub fn delete_connection(&self, id: &str) -> Result<(), CoreError> {
        let mut config = self.load()?;
        config.connections.remove(id);
        config.queries.remove(id);
        self.save(&config)
    }

    pub fn get_query(&self, connection_id: &str) -> Result<Option<String>, CoreError> {
        let config = self.load()?;
        Ok(config.queries.get(connection_id).cloned())
    }

    pub fn save_query(&self, connection_id: &str, query: &str) -> Result<(), CoreError> {
        let mut config = self.load()?;
        config
            .queries
            .insert(connection_id.to_string(), query.to_string());
        self.save(&config)
    }

    pub fn get_theme(&self) -> Result<String, CoreError> {
        let config = self.load()?;
        Ok(config.theme.unwrap_or_else(|| "system".to_string()))
    }

    pub fn save_theme(&self, theme: &str) -> Result<(), CoreError> {
        let mut config = self.load()?;
        config.theme = Some(theme.to_string());
        self.save(&config)
    }

    // ── SSH Profiles ──────────────────────────────────────────────────────────

    pub fn get_ssh_profiles(&self) -> Result<Vec<SshProfile>, CoreError> {
        let config = self.load()?;
        let mut profiles: Vec<SshProfile> = config.ssh_profiles.values().cloned().collect();
        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(profiles)
    }

    pub fn get_ssh_profile(&self, id: &str) -> Result<Option<SshProfile>, CoreError> {
        let config = self.load()?;
        Ok(config.ssh_profiles.get(id).cloned())
    }

    pub fn save_ssh_profile(&self, profile: SshProfile) -> Result<(), CoreError> {
        let mut config = self.load()?;
        config.ssh_profiles.insert(profile.id.clone(), profile);
        self.save(&config)
    }

    pub fn update_ssh_profile(&self, profile: SshProfile) -> Result<(), CoreError> {
        let mut config = self.load()?;
        if !config.ssh_profiles.contains_key(&profile.id) {
            return Err(CoreError {
                message: format!("SSH profile '{}' not found", profile.id),
                code: "NOT_FOUND".into(),
            });
        }
        config.ssh_profiles.insert(profile.id.clone(), profile);
        self.save(&config)
    }

    /// Delete a profile. Returns an error if any connection still references it
    /// (callers must pass `in_use = true` when that is the case).
    pub fn delete_ssh_profile(&self, id: &str) -> Result<(), CoreError> {
        let mut config = self.load()?;

        // Warn if any connection references this profile
        let in_use = config
            .connections
            .values()
            .any(|c| c.ssh_profile_id.as_deref() == Some(id));

        if in_use {
            return Err(CoreError {
                message: "Cannot delete SSH profile: one or more connections are using it".into(),
                code: "PROFILE_IN_USE".into(),
            });
        }

        config.ssh_profiles.remove(id);
        self.save(&config)
    }

    // ── Credential storage settings ───────────────────────────────────────────

    pub fn get_storage_mode(&self) -> Result<Option<String>, CoreError> {
        let config = self.load()?;
        Ok(config.storage_mode)
    }

    pub fn save_storage_mode(&self, mode: &str) -> Result<(), CoreError> {
        let mut config = self.load()?;
        config.storage_mode = Some(mode.to_string());
        self.save(&config)
    }

    pub fn get_vault_timeout_secs(&self) -> Result<u64, CoreError> {
        let config = self.load()?;
        Ok(config.vault_timeout_secs.unwrap_or(15 * 60))
    }

    pub fn save_vault_timeout_secs(&self, secs: u64) -> Result<(), CoreError> {
        let mut config = self.load()?;
        config.vault_timeout_secs = Some(secs);
        self.save(&config)
    }

    /// Returns the IDs of all connections that reference the given SSH profile.
    pub fn connections_using_profile(&self, profile_id: &str) -> Result<Vec<String>, CoreError> {
        let config = self.load()?;
        Ok(config
            .connections
            .values()
            .filter(|c| c.ssh_profile_id.as_deref() == Some(profile_id))
            .map(|c| c.id.clone())
            .collect())
    }

    // ── Connection Groups ──────────────────────────────────────────────────────

    pub fn get_groups(&self) -> Result<Vec<ConnectionGroup>, CoreError> {
        let config = self.load()?;
        let mut groups: Vec<ConnectionGroup> = config.groups.values().cloned().collect();
        groups.sort_by(|a, b| a.order.cmp(&b.order).then(a.name.cmp(&b.name)));
        Ok(groups)
    }

    pub fn save_group(&self, group: ConnectionGroup) -> Result<(), CoreError> {
        let mut config = self.load()?;
        config.groups.insert(group.id.clone(), group);
        self.save(&config)
    }

    pub fn update_group(&self, group: ConnectionGroup) -> Result<(), CoreError> {
        let mut config = self.load()?;
        if !config.groups.contains_key(&group.id) {
            return Err(CoreError {
                message: format!("Group '{}' not found", group.id),
                code: "NOT_FOUND".into(),
            });
        }
        config.groups.insert(group.id.clone(), group);
        self.save(&config)
    }

    /// Delete a group, reassigning its children to the group's parent (or root).
    /// Child sub-groups are also re-parented to the deleted group's parent.
    pub fn delete_group(&self, id: &str) -> Result<(), CoreError> {
        let mut config = self.load()?;

        let parent_id = config
            .groups
            .get(id)
            .and_then(|g| g.parent_group_id.clone());

        // Re-parent child groups
        for group in config.groups.values_mut() {
            if group.parent_group_id.as_deref() == Some(id) {
                group.parent_group_id = parent_id.clone();
            }
        }

        // Re-parent connections
        for conn in config.connections.values_mut() {
            if conn.group_id.as_deref() == Some(id) {
                conn.group_id = parent_id.clone();
            }
        }

        config.groups.remove(id);
        self.save(&config)
    }

    /// Move a connection to a different group (or remove from any group if group_id is None).
    pub fn move_connection_to_group(
        &self,
        connection_id: &str,
        group_id: Option<&str>,
    ) -> Result<(), CoreError> {
        let mut config = self.load()?;
        let conn = config.connections.get_mut(connection_id).ok_or_else(|| CoreError {
            message: format!("Connection '{}' not found", connection_id),
            code: "NOT_FOUND".into(),
        })?;
        conn.group_id = group_id.map(|s| s.to_string());
        self.save(&config)
    }
}
