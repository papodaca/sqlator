use crate::state::AppState;
use sqlator_core::credentials;
use sqlator_core::models::{ConnectionConfig, ConnectionInfo, QueryEvent, SavedConnection, SshProfile};
use sqlator_core::ssh::{config_parser, HostEntry, SshAuthConfig, SshHostConfig, SshTunnel};
use sqlator_core::DatabaseType;
use tauri::ipc::Channel;
use tauri::State;

type CmdResult<T> = Result<T, String>;

fn map_err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// --- Connection CRUD ---

#[tauri::command]
pub async fn get_connections(state: State<'_, AppState>) -> CmdResult<Vec<ConnectionInfo>> {
    let connections = state.config.get_connections().map_err(map_err)?;
    Ok(connections.iter().map(ConnectionInfo::from).collect())
}

#[tauri::command]
pub async fn save_connection(state: State<'_, AppState>, config: ConnectionConfig) -> CmdResult<ConnectionInfo> {
    let parsed = url::Url::parse(&config.url).map_err(map_err)?;

    let db_type = match sqlator_core::detect_database_type(&config.url) {
        Some(DatabaseType::Postgres) => "postgres",
        Some(DatabaseType::MySql) => {
            if parsed.scheme() == "mariadb" { "mariadb" } else { "mysql" }
        }
        Some(DatabaseType::Sqlite) => "sqlite",
        None => return Err(format!("Unsupported database scheme: {}", parsed.scheme())),
    };

    let conn = SavedConnection {
        id: uuid::Uuid::new_v4().to_string(),
        name: config.name,
        color_id: config.color_id,
        db_type: db_type.to_string(),
        host: parsed.host_str().unwrap_or("localhost").to_string(),
        port: parsed.port().unwrap_or(match db_type {
            "postgres" => 5432,
            "mysql" | "mariadb" => 3306,
            _ => 0,
        }),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        url: config.url,
        ssh_profile_id: None,
    };

    state.config.save_connection(conn.clone()).map_err(map_err)?;
    Ok(ConnectionInfo::from(&conn))
}

#[tauri::command]
pub async fn update_connection(
    state: State<'_, AppState>,
    id: String,
    config: ConnectionConfig,
) -> CmdResult<ConnectionInfo> {
    let parsed = url::Url::parse(&config.url).map_err(map_err)?;

    let db_type = match sqlator_core::detect_database_type(&config.url) {
        Some(DatabaseType::Postgres) => "postgres",
        Some(DatabaseType::MySql) => {
            if parsed.scheme() == "mariadb" { "mariadb" } else { "mysql" }
        }
        Some(DatabaseType::Sqlite) => "sqlite",
        None => return Err(format!("Unsupported database scheme: {}", parsed.scheme())),
    };

    let conn = SavedConnection {
        id,
        name: config.name,
        color_id: config.color_id,
        db_type: db_type.to_string(),
        host: parsed.host_str().unwrap_or("localhost").to_string(),
        port: parsed.port().unwrap_or(match db_type {
            "postgres" => 5432,
            "mysql" | "mariadb" => 3306,
            _ => 0,
        }),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        url: config.url,
        ssh_profile_id: None,
    };

    state.config.update_connection(conn.clone()).map_err(map_err)?;
    Ok(ConnectionInfo::from(&conn))
}

#[tauri::command]
pub async fn delete_connection(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.db.disconnect(&id).await;
    state.config.delete_connection(&id).map_err(map_err)
}

#[tauri::command]
pub async fn test_connection(url: String) -> CmdResult<String> {
    sqlator_core::db::DbManager::test_connection(&url)
        .await
        .map_err(map_err)
}

// --- Active connection ---

#[tauri::command]
pub async fn connect_database(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    // Look up the connection URL from config
    let connections = state.config.get_connections().map_err(map_err)?;
    let conn = connections
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("Connection '{}' not found", id))?;

    state.db.connect(&id, &conn.url).await.map_err(map_err)
}

#[tauri::command]
pub async fn disconnect_database(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.db.disconnect(&id).await;
    Ok(())
}

// --- Query execution ---

#[tauri::command]
pub async fn execute_query(
    state: State<'_, AppState>,
    connection_id: String,
    sql: String,
    on_event: Channel<QueryEvent>,
) -> CmdResult<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<QueryEvent>(256);

    // Spawn the core query execution
    let db = &state.db;
    let exec_result = {
        let connection_id = connection_id.clone();
        let sql = sql.clone();

        // Bridge: core mpsc → tauri Channel
        let bridge_handle = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let _ = on_event.send(event);
            }
        });

        let result = db.execute_query(&connection_id, &sql, tx).await;
        // Wait for bridge to flush
        let _ = bridge_handle.await;
        result
    };

    exec_result.map_err(map_err)
}

// --- Query persistence ---

#[tauri::command]
pub async fn get_query(state: State<'_, AppState>, connection_id: String) -> CmdResult<Option<String>> {
    state.config.get_query(&connection_id).map_err(map_err)
}

#[tauri::command]
pub async fn save_query(state: State<'_, AppState>, connection_id: String, query: String) -> CmdResult<()> {
    state.config.save_query(&connection_id, &query).map_err(map_err)
}

// --- Theme ---

#[tauri::command]
pub async fn get_theme(state: State<'_, AppState>) -> CmdResult<String> {
    state.config.get_theme().map_err(map_err)
}

#[tauri::command]
pub async fn save_theme(state: State<'_, AppState>, theme: String) -> CmdResult<()> {
    state.config.save_theme(&theme).map_err(map_err)
}

// --- SSH Config ---

#[tauri::command]
pub async fn list_ssh_hosts() -> CmdResult<Vec<HostEntry>> {
    config_parser::load_ssh_config().map_err(map_err)
}

// --- SSH Tunnels ---

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SshTunnelRequest {
    pub profile_id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub key_path: Option<String>,
    pub key_passphrase: Option<String>,
    pub password: Option<String>,
    pub target_host: String,
    pub target_port: u16,
}

#[derive(Debug, serde::Serialize)]
pub struct SshTunnelInfo {
    pub profile_id: String,
    pub local_port: u16,
    pub target_host: String,
    pub target_port: u16,
}

#[tauri::command]
pub async fn create_ssh_tunnel(
    state: State<'_, AppState>,
    request: SshTunnelRequest,
) -> CmdResult<SshTunnelInfo> {
    let auth_config = match request.auth_method.as_str() {
        "key" => {
            let key_path = request.key_path.clone().unwrap_or_default();
            if let Some(passphrase) = &request.key_passphrase {
                SshAuthConfig::with_key_and_passphrase(&request.username, key_path, passphrase)
            } else {
                SshAuthConfig::with_key(&request.username, key_path)
            }
        }
        "password" => {
            let password = request.password.clone().unwrap_or_default();
            SshAuthConfig::with_password(&request.username, password)
        }
        "agent" => SshAuthConfig::with_agent(&request.username),
        _ => {
            return Err(format!(
                "Unsupported auth method: {}",
                request.auth_method
            ))
        }
    };

    let ssh_config = SshHostConfig::new(&request.host, request.port, auth_config.clone());

    let tunnel = SshTunnel::create(
        request.profile_id.clone(),
        &ssh_config,
        auth_config,
        request.target_host.clone(),
        request.target_port,
        vec![],
    )
    .await
    .map_err(map_err)?;

    SshTunnel::start_forwarding(&tunnel)
        .await
        .map_err(map_err)?;

    let info = SshTunnelInfo {
        profile_id: tunnel.profile_id.clone(),
        local_port: tunnel.local_port,
        target_host: tunnel.target_host.clone(),
        target_port: tunnel.target_port,
    };

    state.tunnels.insert(request.profile_id, tunnel);

    Ok(info)
}

#[tauri::command]
pub async fn close_ssh_tunnel(state: State<'_, AppState>, profile_id: String) -> CmdResult<()> {
    let (_, tunnel) = state
        .tunnels
        .remove(&profile_id)
        .ok_or_else(|| format!("Tunnel '{}' not found", profile_id))?;

    SshTunnel::close(tunnel).await.map_err(map_err)?;

    Ok(())
}

#[tauri::command]
pub async fn get_active_tunnels(state: State<'_, AppState>) -> CmdResult<Vec<SshTunnelInfo>> {
    let tunnels: Vec<SshTunnelInfo> = state
        .tunnels
        .iter()
        .map(|entry| SshTunnelInfo {
            profile_id: entry.profile_id.clone(),
            local_port: entry.local_port,
            target_host: entry.target_host.clone(),
            target_port: entry.target_port,
        })
        .collect();

    Ok(tunnels)
}

// --- SSH Profiles ---

/// Frontend-facing SSH profile (no secrets).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SshProfileConfig {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub key_path: Option<String>,
    /// Set to store/update a password in the keyring (not returned on read)
    pub password: Option<String>,
    /// Set to store/update a key passphrase in the keyring (not returned on read)
    pub key_passphrase: Option<String>,
    pub proxy_jump: Vec<sqlator_core::models::SshJumpHost>,
    pub local_port_binding: Option<u16>,
    pub keepalive_interval: Option<u32>,
}

#[tauri::command]
pub async fn get_ssh_profiles(state: State<'_, AppState>) -> CmdResult<Vec<SshProfile>> {
    state.config.get_ssh_profiles().map_err(map_err)
}

#[tauri::command]
pub async fn save_ssh_profile(
    state: State<'_, AppState>,
    config: SshProfileConfig,
) -> CmdResult<SshProfile> {
    let id = uuid::Uuid::new_v4().to_string();

    let auth_method = parse_auth_method(&config.auth_method)?;

    let profile = SshProfile {
        id: id.clone(),
        name: config.name,
        host: config.host,
        port: config.port,
        username: config.username,
        auth_method,
        key_path: config.key_path,
        proxy_jump: config.proxy_jump,
        local_port_binding: config.local_port_binding,
        keepalive_interval: config.keepalive_interval,
    };

    state.config.save_ssh_profile(profile.clone()).map_err(map_err)?;

    // Store secrets in keyring if provided
    if let Some(pw) = &config.password {
        if !pw.is_empty() {
            credentials::store_credential(&id, "password", pw).map_err(map_err)?;
        }
    }
    if let Some(pp) = &config.key_passphrase {
        if !pp.is_empty() {
            credentials::store_credential(&id, "passphrase", pp).map_err(map_err)?;
        }
    }

    Ok(profile)
}

#[tauri::command]
pub async fn update_ssh_profile(
    state: State<'_, AppState>,
    id: String,
    config: SshProfileConfig,
) -> CmdResult<SshProfile> {
    // Check profile exists
    state
        .config
        .get_ssh_profile(&id)
        .map_err(map_err)?
        .ok_or_else(|| format!("SSH profile '{id}' not found"))?;

    let auth_method = parse_auth_method(&config.auth_method)?;

    let profile = SshProfile {
        id: id.clone(),
        name: config.name,
        host: config.host,
        port: config.port,
        username: config.username,
        auth_method,
        key_path: config.key_path,
        proxy_jump: config.proxy_jump,
        local_port_binding: config.local_port_binding,
        keepalive_interval: config.keepalive_interval,
    };

    state.config.update_ssh_profile(profile.clone()).map_err(map_err)?;

    // Update secrets if provided (empty string = leave unchanged)
    if let Some(pw) = &config.password {
        if !pw.is_empty() {
            credentials::store_credential(&id, "password", pw).map_err(map_err)?;
        }
    }
    if let Some(pp) = &config.key_passphrase {
        if !pp.is_empty() {
            credentials::store_credential(&id, "passphrase", pp).map_err(map_err)?;
        }
    }

    Ok(profile)
}

#[tauri::command]
pub async fn delete_ssh_profile(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<()> {
    // This returns an error (PROFILE_IN_USE) if connections reference it
    state.config.delete_ssh_profile(&id).map_err(map_err)?;
    // Clean up keyring entries
    credentials::delete_all_credentials(&id).map_err(map_err)?;
    Ok(())
}

/// Returns connection IDs that use a given profile — used to warn before delete.
#[tauri::command]
pub async fn connections_using_ssh_profile(
    state: State<'_, AppState>,
    profile_id: String,
) -> CmdResult<Vec<String>> {
    state
        .config
        .connections_using_profile(&profile_id)
        .map_err(map_err)
}

fn parse_auth_method(s: &str) -> CmdResult<sqlator_core::models::SshAuthMethod> {
    match s {
        "key" => Ok(sqlator_core::models::SshAuthMethod::Key),
        "password" => Ok(sqlator_core::models::SshAuthMethod::Password),
        "agent" => Ok(sqlator_core::models::SshAuthMethod::Agent),
        other => Err(format!("Unknown auth method: {other}")),
    }
}
