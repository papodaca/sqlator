use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedConnection {
    pub id: String,
    pub name: String,
    pub color_id: String,
    pub db_type: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub url: String, // MVP: stored as-is; TODO: migrate passwords to OS keychain
}

impl SavedConnection {
    pub fn masked_url(&self) -> String {
        if let Ok(mut parsed) = url::Url::parse(&self.url) {
            if parsed.password().is_some() {
                let _ = parsed.set_password(Some("***"));
            }
            parsed.to_string()
        } else {
            self.url.clone()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub name: String,
    pub color_id: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum QueryEvent {
    Columns { names: Vec<String> },
    Row { values: Vec<serde_json::Value> },
    Done { row_count: usize, duration_ms: u64 },
    RowsAffected { count: u64, duration_ms: u64 },
    Error { message: String },
}

/// Data sent to the frontend representing a saved connection (with masked password)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub id: String,
    pub name: String,
    pub color_id: String,
    pub db_type: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub masked_url: String,
}

impl From<&SavedConnection> for ConnectionInfo {
    fn from(conn: &SavedConnection) -> Self {
        ConnectionInfo {
            id: conn.id.clone(),
            name: conn.name.clone(),
            color_id: conn.color_id.clone(),
            db_type: conn.db_type.clone(),
            host: conn.host.clone(),
            port: conn.port,
            database: conn.database.clone(),
            username: conn.username.clone(),
            masked_url: conn.masked_url(),
        }
    }
}
