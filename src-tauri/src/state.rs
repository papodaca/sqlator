use dashmap::DashMap;
use sqlator_core::config::ConfigManager;
use sqlator_core::db::DbManager;
use sqlator_core::ssh::TunnelHandle;

pub struct AppState {
    pub config: ConfigManager,
    pub db: DbManager,
    pub tunnels: DashMap<String, TunnelHandle>,
}

impl AppState {
    pub fn new() -> Result<Self, sqlator_core::error::CoreError> {
        Ok(Self {
            config: ConfigManager::new("sqlator")?,
            db: DbManager::new(),
            tunnels: DashMap::new(),
        })
    }
}
