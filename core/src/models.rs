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

// ── Schema Browser ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaInfo {
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub name: String,
    pub schema: Option<String>,
    pub table_type: String, // "table" | "view"
    pub full_name: String,  // "schema.table" or just "table"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub is_primary_key: bool,
    pub is_foreign_key: bool,
    pub foreign_table: Option<String>,
    pub foreign_column: Option<String>,
    pub ordinal_position: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortSpec {
    pub column: String,
    pub desc: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterSpec {
    pub column: String,
    pub operator: String, // "contains"|"equals"|"startsWith"|"endsWith"|"gt"|"gte"|"lt"|"lte"|"isNull"|"isNotNull"
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableQueryParams {
    pub connection_id: String,
    pub table_name: String,
    pub schema: Option<String>,
    pub sort: Vec<SortSpec>,
    pub filters: Vec<FilterSpec>,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableQueryResult {
    pub columns: Vec<String>,
    pub column_types: Vec<String>,
    pub rows: Vec<serde_json::Value>, // Vec of objects {col -> value}
    pub has_more: bool,
    pub total_returned: usize,
}

// ── Schema Metadata ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMeta {
    pub name: String,
    pub column_type: String,
    pub nullable: bool,
    pub is_auto_increment: bool,
    pub is_generated: bool,
    pub is_updatable: bool,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrimaryKeyMeta {
    pub columns: Vec<String>,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableMeta {
    pub table_name: String,
    pub schema: Option<String>,
    pub columns: Vec<ColumnMeta>,
    pub primary_key: PrimaryKeyMeta,
    pub is_editable: bool,
    pub editability_reason: Option<String>,
}

// ── Batch Execution ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterizedStatement {
    pub sql: String,
    pub params: Vec<serde_json::Value>,
    pub temp_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlBatch {
    pub statements: Vec<ParameterizedStatement>,
    pub use_transaction: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchError {
    pub statement_index: usize,
    pub message: String,
    pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchResult {
    pub success: bool,
    pub executed_count: usize,
    pub total_statements: usize,
    pub error: Option<BatchError>,
    pub inserted_ids: std::collections::HashMap<String, serde_json::Value>,
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
