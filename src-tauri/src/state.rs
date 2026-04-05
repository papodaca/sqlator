use sqlator_core::config::ConfigManager;
use sqlator_core::db::DbManager;

pub struct AppState {
    pub config: ConfigManager,
    pub db: DbManager,
}

impl AppState {
    pub fn new() -> Result<Self, sqlator_core::error::CoreError> {
        Ok(Self {
            config: ConfigManager::new("sqlator")?,
            db: DbManager::new(),
        })
    }
}
