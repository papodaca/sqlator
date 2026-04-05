use crate::error::CoreError;
use crate::models::SavedConnection;
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
}
