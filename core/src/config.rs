use crate::error::CoreError;
use crate::models::{ConnectionGroup, SavedConnection, SshProfile};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// File-based configuration manager.
/// Stores connection metadata as JSON on disk.
/// Framework-agnostic — works for both Tauri and TUI.
///
/// Internally synchronized (process-local) and writes atomically via temp+rename
/// so concurrent desktop+web / multi-threaded access cannot tear the file.
pub struct ConfigManager {
    config_path: PathBuf,
    lock: Mutex<()>,
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
    /// Persisted tab layout (open connections + query tabs + SQL text)
    #[serde(default)]
    tab_state: Option<serde_json::Value>,
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
            lock: Mutex::new(()),
        })
    }

    /// Directory that holds `connections.json` (and the sibling vault file).
    pub fn config_dir(&self) -> PathBuf {
        self.config_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Default vault path beside `connections.json`.
    pub fn vault_path(&self) -> PathBuf {
        self.config_dir().join("vault.enc")
    }

    fn with_lock<T>(&self, f: impl FnOnce() -> Result<T, CoreError>) -> Result<T, CoreError> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }

    fn load_unlocked(&self) -> Result<ConfigData, CoreError> {
        if !self.config_path.exists() {
            return Ok(ConfigData::default());
        }
        let data = std::fs::read_to_string(&self.config_path)?;
        let config: ConfigData = serde_json::from_str(&data)?;
        Ok(config)
    }

    fn save_unlocked(&self, config: &ConfigData) -> Result<(), CoreError> {
        let data = serde_json::to_string_pretty(config)?;
        // Atomic write: temp file → rename (same pattern as vault).
        let tmp = self.config_path.with_extension("tmp");
        std::fs::write(&tmp, &data)?;
        std::fs::rename(&tmp, &self.config_path)?;
        Ok(())
    }

    pub fn get_connections(&self) -> Result<Vec<SavedConnection>, CoreError> {
        self.with_lock(|| {
            let config = self.load_unlocked()?;
            Ok(config.connections.values().cloned().collect())
        })
    }

    pub fn save_connection(&self, conn: SavedConnection) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;
            config.connections.insert(conn.id.clone(), conn);
            self.save_unlocked(&config)
        })
    }

    pub fn update_connection(&self, conn: SavedConnection) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;
            if !config.connections.contains_key(&conn.id) {
                return Err(CoreError {
                    message: format!("Connection '{}' not found", conn.id),
                    code: "NOT_FOUND".into(),
                });
            }
            config.connections.insert(conn.id.clone(), conn);
            self.save_unlocked(&config)
        })
    }

    pub fn delete_connection(&self, id: &str) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;
            config.connections.remove(id);
            config.queries.remove(id);
            self.save_unlocked(&config)
        })
    }

    pub fn get_query(&self, connection_id: &str) -> Result<Option<String>, CoreError> {
        self.with_lock(|| {
            let config = self.load_unlocked()?;
            Ok(config.queries.get(connection_id).cloned())
        })
    }

    pub fn save_query(&self, connection_id: &str, query: &str) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;
            config
                .queries
                .insert(connection_id.to_string(), query.to_string());
            self.save_unlocked(&config)
        })
    }

    pub fn get_theme(&self) -> Result<String, CoreError> {
        self.with_lock(|| {
            let config = self.load_unlocked()?;
            Ok(config.theme.unwrap_or_else(|| "system".to_string()))
        })
    }

    pub fn save_theme(&self, theme: &str) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;
            config.theme = Some(theme.to_string());
            self.save_unlocked(&config)
        })
    }

    // ── SSH Profiles ──────────────────────────────────────────────────────────

    pub fn get_ssh_profiles(&self) -> Result<Vec<SshProfile>, CoreError> {
        self.with_lock(|| {
            let config = self.load_unlocked()?;
            let mut profiles: Vec<SshProfile> = config.ssh_profiles.values().cloned().collect();
            profiles.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(profiles)
        })
    }

    pub fn get_ssh_profile(&self, id: &str) -> Result<Option<SshProfile>, CoreError> {
        self.with_lock(|| {
            let config = self.load_unlocked()?;
            Ok(config.ssh_profiles.get(id).cloned())
        })
    }

    pub fn save_ssh_profile(&self, profile: SshProfile) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;
            config.ssh_profiles.insert(profile.id.clone(), profile);
            self.save_unlocked(&config)
        })
    }

    pub fn update_ssh_profile(&self, profile: SshProfile) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;
            if !config.ssh_profiles.contains_key(&profile.id) {
                return Err(CoreError {
                    message: format!("SSH profile '{}' not found", profile.id),
                    code: "NOT_FOUND".into(),
                });
            }
            config.ssh_profiles.insert(profile.id.clone(), profile);
            self.save_unlocked(&config)
        })
    }

    /// Delete a profile. Returns an error if any connection still references it
    /// (callers must pass `in_use = true` when that is the case).
    pub fn delete_ssh_profile(&self, id: &str) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;

            // Warn if any connection references this profile
            let in_use = config
                .connections
                .values()
                .any(|c| c.ssh_profile_id.as_deref() == Some(id));

            if in_use {
                return Err(CoreError {
                    message: "Cannot delete SSH profile: one or more connections are using it"
                        .into(),
                    code: "PROFILE_IN_USE".into(),
                });
            }

            config.ssh_profiles.remove(id);
            self.save_unlocked(&config)
        })
    }

    // ── Credential storage settings ───────────────────────────────────────────

    pub fn get_storage_mode(&self) -> Result<Option<String>, CoreError> {
        self.with_lock(|| {
            let config = self.load_unlocked()?;
            Ok(config.storage_mode)
        })
    }

    pub fn save_storage_mode(&self, mode: &str) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;
            config.storage_mode = Some(mode.to_string());
            self.save_unlocked(&config)
        })
    }

    pub fn get_vault_timeout_secs(&self) -> Result<u64, CoreError> {
        self.with_lock(|| {
            let config = self.load_unlocked()?;
            Ok(config.vault_timeout_secs.unwrap_or(15 * 60))
        })
    }

    pub fn save_vault_timeout_secs(&self, secs: u64) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;
            config.vault_timeout_secs = Some(secs);
            self.save_unlocked(&config)
        })
    }

    /// Returns the IDs of all connections that reference the given SSH profile.
    pub fn connections_using_profile(&self, profile_id: &str) -> Result<Vec<String>, CoreError> {
        self.with_lock(|| {
            let config = self.load_unlocked()?;
            Ok(config
                .connections
                .values()
                .filter(|c| c.ssh_profile_id.as_deref() == Some(profile_id))
                .map(|c| c.id.clone())
                .collect())
        })
    }

    // ── Connection Groups ──────────────────────────────────────────────────────

    pub fn get_groups(&self) -> Result<Vec<ConnectionGroup>, CoreError> {
        self.with_lock(|| {
            let config = self.load_unlocked()?;
            let mut groups: Vec<ConnectionGroup> = config.groups.values().cloned().collect();
            groups.sort_by(|a, b| a.order.cmp(&b.order).then(a.name.cmp(&b.name)));
            Ok(groups)
        })
    }

    pub fn save_group(&self, group: ConnectionGroup) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;
            config.groups.insert(group.id.clone(), group);
            self.save_unlocked(&config)
        })
    }

    pub fn update_group(&self, group: ConnectionGroup) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;
            if !config.groups.contains_key(&group.id) {
                return Err(CoreError {
                    message: format!("Group '{}' not found", group.id),
                    code: "NOT_FOUND".into(),
                });
            }
            config.groups.insert(group.id.clone(), group);
            self.save_unlocked(&config)
        })
    }

    /// Delete a group, reassigning its children to the group's parent (or root).
    /// Child sub-groups are also re-parented to the deleted group's parent.
    pub fn delete_group(&self, id: &str) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;

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
            self.save_unlocked(&config)
        })
    }

    // ── Tab state ─────────────────────────────────────────────────────────────

    pub fn get_tab_state(&self) -> Result<Option<serde_json::Value>, CoreError> {
        self.with_lock(|| {
            let config = self.load_unlocked()?;
            Ok(config.tab_state)
        })
    }

    pub fn save_tab_state(&self, state: serde_json::Value) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;
            config.tab_state = Some(state);
            self.save_unlocked(&config)
        })
    }

    /// Move a connection to a different group (or remove from any group if group_id is None).
    pub fn move_connection_to_group(
        &self,
        connection_id: &str,
        group_id: Option<&str>,
    ) -> Result<(), CoreError> {
        self.with_lock(|| {
            let mut config = self.load_unlocked()?;
            let conn = config
                .connections
                .get_mut(connection_id)
                .ok_or_else(|| CoreError {
                    message: format!("Connection '{connection_id}' not found"),
                    code: "NOT_FOUND".into(),
                })?;
            conn.group_id = group_id.map(|s| s.to_string());
            self.save_unlocked(&config)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn save_writes_atomically_without_leaving_tmp() {
        let id = uuid::Uuid::new_v4();
        let app_name = format!("sqlator-config-atomic-{id}");
        let mgr = ConfigManager::new(&app_name).expect("config manager");
        let dir = dirs::config_dir().unwrap().join(&app_name);
        let path = dir.join("connections.json");
        let tmp = dir.join("connections.tmp");

        mgr.save_theme("dark").expect("save");
        assert!(path.exists());
        assert!(!tmp.exists(), "temp file must be renamed away");

        let _ = fs::remove_dir_all(&dir);
    }
}
