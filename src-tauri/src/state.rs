use crate::terminal::PtyHandle;
use dashmap::DashMap;
use sqlator_service::AppService;
use std::sync::Arc;

pub struct AppState {
    pub service: Arc<AppService>,
    /// Frontend-only PTY handles (Tauri terminal feature).
    pub terminals: DashMap<String, PtyHandle>,
}

impl AppState {
    pub fn new() -> Result<Self, sqlator_service::ServiceError> {
        Ok(Self {
            service: Arc::new(AppService::new()?),
            terminals: DashMap::new(),
        })
    }
}
