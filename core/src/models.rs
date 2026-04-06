use serde::{Deserialize, Serialize};

// ── Connection Groups ─────────────────────────────────────────────────────────

/// A folder for organizing connections. Max nesting depth: 3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionGroup {
    pub id: String,
    pub name: String,
    /// Optional hex color string (e.g. "#ef4444")
    pub color: Option<String>,
    /// Parent group id; None = root level
    #[serde(default)]
    pub parent_group_id: Option<String>,
    /// Sort order within parent
    pub order: u32,
    /// Whether this group is collapsed in the sidebar
    pub collapsed: bool,
}

// ── SSH Profiles ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SshAuthMethod {
    Key,
    Password,
    Agent,
}

/// A jump-host hop in a ProxyJump chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshJumpHost {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: SshAuthMethod,
    /// Path to identity file (if auth_method = Key)
    pub key_path: Option<String>,
}

/// Stored SSH profile — credentials (passwords/passphrases) live in the OS
/// keyring keyed by profile id; only non-secret fields live here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: SshAuthMethod,
    /// Path to identity file (if auth_method = Key)
    pub key_path: Option<String>,
    /// Jump host chain (empty = direct connection)
    pub proxy_jump: Vec<SshJumpHost>,
    /// Preferred local port for the tunnel (None = auto-assign)
    pub local_port_binding: Option<u16>,
    /// Keep-alive interval in seconds (None = disabled)
    pub keepalive_interval: Option<u32>,
}

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
    /// Optional link to an SshProfile for tunnelled connections
    #[serde(default)]
    pub ssh_profile_id: Option<String>,
    /// Optional connection group id
    #[serde(default)]
    pub group_id: Option<String>,
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
    #[serde(default)]
    pub ssh_profile_id: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
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
    #[serde(default)]
    pub ssh_profile_id: Option<String>,
    /// Optional connection group id
    #[serde(default)]
    pub group_id: Option<String>,
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
            ssh_profile_id: conn.ssh_profile_id.clone(),
            group_id: conn.group_id.clone(),
        }
    }
}
