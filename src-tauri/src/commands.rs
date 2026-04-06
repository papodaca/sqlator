use crate::state::AppState;
use sqlator_core::credentials::{CredentialStore, StorageMode, VaultSettings};
use sqlator_core::models::{ConnectionConfig, ConnectionGroup, ConnectionInfo, QueryEvent, SavedConnection, SshProfile};
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
        ssh_profile_id: config.ssh_profile_id,
        group_id: config.group_id,
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
        ssh_profile_id: config.ssh_profile_id,
        group_id: config.group_id,
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

    // Store secrets in credential store if provided
    if let Some(pw) = &config.password {
        if !pw.is_empty() {
            state.credentials.store_credential(&id, "password", pw).map_err(map_err)?;
        }
    }
    if let Some(pp) = &config.key_passphrase {
        if !pp.is_empty() {
            state.credentials.store_credential(&id, "passphrase", pp).map_err(map_err)?;
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
            state.credentials.store_credential(&id, "password", pw).map_err(map_err)?;
        }
    }
    if let Some(pp) = &config.key_passphrase {
        if !pp.is_empty() {
            state.credentials.store_credential(&id, "passphrase", pp).map_err(map_err)?;
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
    // Clean up credential entries
    state.credentials.delete_all_credentials(&id).map_err(map_err)?;
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

// --- Connection URL parsing ---

#[derive(Debug, serde::Serialize)]
pub struct ParsedConnectionUrl {
    pub db_type: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: Option<String>,
}

#[tauri::command]
pub async fn parse_connection_url(url: String) -> CmdResult<ParsedConnectionUrl> {
    let parsed = url::Url::parse(&url).map_err(map_err)?;

    let (db_type, default_port) = match parsed.scheme() {
        "postgres" | "postgresql" => ("postgres", 5432u16),
        "mysql" => ("mysql", 3306),
        "mariadb" => ("mariadb", 3306),
        "sqlite" => ("sqlite", 0),
        s => return Err(format!("Unsupported scheme: {s}")),
    };

    Ok(ParsedConnectionUrl {
        db_type: db_type.to_string(),
        host: parsed.host_str().unwrap_or("localhost").to_string(),
        port: parsed.port().unwrap_or(default_port),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        password: parsed.password().map(|p| p.to_string()),
    })
}

// --- SSH-tunneled connection test ---

#[tauri::command]
pub async fn test_connection_with_ssh(
    state: State<'_, AppState>,
    url: String,
    ssh_profile_id: String,
) -> CmdResult<String> {
    // Look up SSH profile
    let profile = state
        .config
        .get_ssh_profile(&ssh_profile_id)
        .map_err(map_err)?
        .ok_or_else(|| format!("SSH profile '{ssh_profile_id}' not found"))?;

    // Parse DB URL to get target host/port
    let parsed_url = url::Url::parse(&url).map_err(map_err)?;
    let target_host = parsed_url.host_str().unwrap_or("localhost").to_string();
    let default_port = match parsed_url.scheme() {
        "postgres" | "postgresql" => 5432u16,
        "mysql" | "mariadb" => 3306,
        _ => 0,
    };
    let target_port = parsed_url.port().unwrap_or(default_port);

    // Build auth config from profile + credential store
    let auth_config = build_auth_config_for_profile(&profile, &state.credentials)?;
    let ssh_config = SshHostConfig::new(&profile.host, profile.port, auth_config.clone());

    // Create ephemeral tunnel (no jump hosts for now — TODO: wire up proxy_jump)
    let tunnel_id = format!("test-{}", uuid::Uuid::new_v4());
    let tunnel = SshTunnel::create(
        tunnel_id,
        &ssh_config,
        auth_config,
        target_host,
        target_port,
        vec![],
    )
    .await
    .map_err(map_err)?;

    SshTunnel::start_forwarding(&tunnel).await.map_err(map_err)?;
    let local_port = tunnel.local_port;

    // Rewrite DB URL to point at the local tunnel endpoint
    let mut test_url = parsed_url.clone();
    let _ = test_url.set_host(Some("127.0.0.1"));
    let _ = test_url.set_port(Some(local_port));
    let test_url_str = test_url.to_string();

    // Test the connection through the tunnel
    let result = sqlator_core::db::DbManager::test_connection(&test_url_str).await;

    // Always close the ephemeral tunnel, even on error
    SshTunnel::close(tunnel).await.ok();

    result.map_err(map_err)
}

fn build_auth_config_for_profile(
    profile: &sqlator_core::models::SshProfile,
    credentials: &CredentialStore,
) -> CmdResult<SshAuthConfig> {
    use sqlator_core::models::SshAuthMethod;
    match profile.auth_method {
        SshAuthMethod::Key => {
            let key_path = profile.key_path.as_deref().unwrap_or_default();
            let passphrase = credentials.get_credential(&profile.id, "passphrase").map_err(map_err)?;
            if let Some(pp) = passphrase {
                Ok(SshAuthConfig::with_key_and_passphrase(&profile.username, key_path, pp))
            } else {
                Ok(SshAuthConfig::with_key(&profile.username, key_path))
            }
        }
        SshAuthMethod::Password => {
            let password = credentials
                .get_credential(&profile.id, "password")
                .map_err(map_err)?
                .unwrap_or_default();
            Ok(SshAuthConfig::with_password(&profile.username, password))
        }
        SshAuthMethod::Agent => Ok(SshAuthConfig::with_agent(&profile.username)),
    }
}

// ── Credential storage settings ───────────────────────────────────────────────

#[tauri::command]
pub async fn check_keyring_available() -> bool {
    CredentialStore::keyring_available()
}

#[tauri::command]
pub async fn get_storage_mode(state: State<'_, AppState>) -> CmdResult<String> {
    Ok(state.credentials.mode().to_string())
}

#[tauri::command]
pub async fn set_storage_mode(
    state: State<'_, AppState>,
    mode: String,
    migrate: bool,
) -> CmdResult<()> {
    let new_mode: StorageMode = mode.parse().map_err(map_err)?;

    if migrate {
        let profiles = state.config.get_ssh_profiles().map_err(map_err)?;
        let ids: Vec<String> = profiles.iter().map(|p| p.id.clone()).collect();
        state.credentials.migrate_to(&new_mode, &ids).map_err(map_err)?;
    }

    state.credentials.set_mode(new_mode);
    state.config.save_storage_mode(&mode).map_err(map_err)?;
    Ok(())
}

// ── Vault commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn vault_exists(state: State<'_, AppState>) -> CmdResult<bool> {
    Ok(state.credentials.vault.is_initialized())
}

#[tauri::command]
pub async fn is_vault_locked(state: State<'_, AppState>) -> CmdResult<bool> {
    Ok(state.credentials.vault.is_locked())
}

#[tauri::command]
pub async fn create_vault(state: State<'_, AppState>, password: String) -> CmdResult<()> {
    state.credentials.vault.create(&password).map_err(map_err)?;
    // Auto-switch mode to vault after creation
    state.credentials.set_mode(StorageMode::Vault);
    state.config.save_storage_mode("vault").map_err(map_err)?;
    Ok(())
}

#[tauri::command]
pub async fn unlock_vault(state: State<'_, AppState>, password: String) -> CmdResult<()> {
    state.credentials.vault.unlock(&password).map_err(map_err)
}

#[tauri::command]
pub async fn lock_vault(state: State<'_, AppState>) -> CmdResult<()> {
    state.credentials.vault.lock();
    Ok(())
}

#[tauri::command]
pub async fn get_vault_settings(state: State<'_, AppState>) -> CmdResult<VaultSettings> {
    let timeout_secs = state.config.get_vault_timeout_secs().map_err(map_err)?;
    Ok(VaultSettings { timeout_secs })
}

#[tauri::command]
pub async fn save_vault_settings(
    state: State<'_, AppState>,
    settings: VaultSettings,
) -> CmdResult<()> {
    state.credentials.vault.set_timeout(settings.timeout_secs);
    state.config.save_vault_timeout_secs(settings.timeout_secs).map_err(map_err)?;
    Ok(())
}

// ── Connection Groups ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_groups(state: State<'_, AppState>) -> CmdResult<Vec<ConnectionGroup>> {
    state.config.get_groups().map_err(map_err)
}

#[derive(Debug, serde::Deserialize)]
pub struct SaveGroupPayload {
    pub name: String,
    pub color: Option<String>,
    pub parent_group_id: Option<String>,
}

#[tauri::command]
pub async fn save_group(
    state: State<'_, AppState>,
    payload: SaveGroupPayload,
) -> CmdResult<ConnectionGroup> {
    let groups = state.config.get_groups().map_err(map_err)?;
    let order = groups.len() as u32;

    let group = ConnectionGroup {
        id: uuid::Uuid::new_v4().to_string(),
        name: payload.name,
        color: payload.color,
        parent_group_id: payload.parent_group_id,
        order,
        collapsed: false,
    };

    state.config.save_group(group.clone()).map_err(map_err)?;
    Ok(group)
}

#[tauri::command]
pub async fn update_group(
    state: State<'_, AppState>,
    group: ConnectionGroup,
) -> CmdResult<ConnectionGroup> {
    state.config.update_group(group.clone()).map_err(map_err)?;
    Ok(group)
}

#[tauri::command]
pub async fn delete_group(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.config.delete_group(&id).map_err(map_err)
}

#[tauri::command]
pub async fn move_connection_to_group(
    state: State<'_, AppState>,
    connection_id: String,
    group_id: Option<String>,
) -> CmdResult<ConnectionInfo> {
    state
        .config
        .move_connection_to_group(&connection_id, group_id.as_deref())
        .map_err(map_err)?;

    let connections = state.config.get_connections().map_err(map_err)?;
    let conn = connections
        .iter()
        .find(|c| c.id == connection_id)
        .ok_or_else(|| format!("Connection '{connection_id}' not found"))?;

    Ok(ConnectionInfo::from(conn))
}

fn parse_auth_method(s: &str) -> CmdResult<sqlator_core::models::SshAuthMethod> {
    match s {
        "key" => Ok(sqlator_core::models::SshAuthMethod::Key),
        "password" => Ok(sqlator_core::models::SshAuthMethod::Password),
        "agent" => Ok(sqlator_core::models::SshAuthMethod::Agent),
        other => Err(format!("Unknown auth method: {other}")),
    }
}
