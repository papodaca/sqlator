use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use sqlator_core::{
    credentials::{CredentialStore, StorageMode, VaultSettings},
    db::DbManager,
    docker::inspector::ContainerInspector,
    models::{
        ConnectionConfig, ConnectionGroup, ConnectionInfo, ConnectionType, SavedConnection, SchemaColumnInfo,
        SchemaInfo, SqlBatch, SshProfile, TableInfo, TableMeta, TableQueryParams, TableQueryResult,
        BatchResult,
    },
    ssh::{config_parser, SshAuthConfig, SshHostConfig, SshTunnel},
};
use std::sync::Arc;

// ── Error helpers ─────────────────────────────────────────────────────────────

// Inner handlers return plain Value; dispatch() wraps it in Json.
type HandlerResult = Result<Value, (StatusCode, String)>;

fn err(msg: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, msg.to_string())
}

fn server_err(msg: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, msg.to_string())
}

/// Extract a required field from the JSON args body.
fn get<T: serde::de::DeserializeOwned>(
    args: &Value,
    key: &str,
) -> Result<T, (StatusCode, String)> {
    serde_json::from_value(args[key].clone())
        .map_err(|e| err(format!("missing/invalid '{}': {}", key, e)))
}

/// Extract an optional field (returns None if missing or null).
fn get_opt<T: serde::de::DeserializeOwned>(
    args: &Value,
    key: &str,
) -> Result<Option<T>, (StatusCode, String)> {
    let v = &args[key];
    // serde_json returns Value::Null for both missing keys and explicit nulls
    if v.is_null() {
        return Ok(None);
    }
    serde_json::from_value(v.clone())
        .map(Some)
        .map_err(|e| err(format!("invalid '{}': {}", key, e)))
}

// ── Main dispatch handler ─────────────────────────────────────────────────────

pub async fn dispatch(
    Path(command): Path<String>,
    State(state): State<Arc<AppState>>,
    body: Option<Json<Value>>,
) -> impl IntoResponse {
    let args = body.map(|j| j.0).unwrap_or(Value::Null);

    match handle(&command, &state, &args).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err((status, msg)) => (status, msg).into_response(),
    }
}

async fn handle(command: &str, state: &Arc<AppState>, args: &Value) -> HandlerResult {
    match command {
        // ── Server info (mode detection) ──────────────────────────────────────
        "server-info" => server_info(state),

        // ── Connection CRUD ───────────────────────────────────────────────────
        "get-connections" => get_connections(state).await,
        "save-connection" => require_multi_db(state, || save_connection(state, args)).await,
        "update-connection" => require_multi_db(state, || update_connection(state, args)).await,
        "clone-connection" => require_multi_db(state, || clone_connection(state, args)).await,
        "delete-connection" => require_multi_db(state, || delete_connection(state, args)).await,
        "test-connection" => test_connection(args).await,
        "connect-database" => connect_database(state, args).await,
        "disconnect-database" => disconnect_database(state, args).await,

        // ── Query & tab state ─────────────────────────────────────────────────
        "get-query" => get_query(state, args).await,
        "save-query" => save_query(state, args).await,
        "get-tab-state" => get_tab_state(state).await,
        "save-tab-state" => save_tab_state(state, args).await,

        // ── Theme ─────────────────────────────────────────────────────────────
        "get-theme" => get_theme(state).await,
        "save-theme" => save_theme(state, args).await,

        // ── SSH config & profiles ─────────────────────────────────────────────
        "list-ssh-hosts" => list_ssh_hosts().await,
        "get-ssh-profiles" => get_ssh_profiles(state).await,
        "save-ssh-profile" => save_ssh_profile(state, args).await,
        "update-ssh-profile" => update_ssh_profile(state, args).await,
        "delete-ssh-profile" => delete_ssh_profile(state, args).await,
        "connections-using-ssh-profile" => connections_using_ssh_profile(state, args).await,

        // ── SSH tunnels ───────────────────────────────────────────────────────
        "create-ssh-tunnel" => create_ssh_tunnel(state, args).await,
        "close-ssh-tunnel" => close_ssh_tunnel(state, args).await,
        "get-active-tunnels" => get_active_tunnels(state).await,

        // ── Credential storage ────────────────────────────────────────────────
        "check-keyring-available" => Ok(json!(CredentialStore::keyring_available())),
        "get-storage-mode" => get_storage_mode(state).await,
        "set-storage-mode" => set_storage_mode(state, args).await,
        "vault-exists" => Ok(json!(state.credentials.vault.is_initialized())),
        "is-vault-locked" => Ok(json!(state.credentials.vault.is_locked())),
        "create-vault" => create_vault(state, args).await,
        "unlock-vault" => unlock_vault(state, args).await,
        "lock-vault" => {
            state.credentials.vault.lock();
            Ok(json!(null))
        }
        "get-vault-settings" => get_vault_settings(state).await,
        "save-vault-settings" => save_vault_settings(state, args).await,

        // ── Connection groups ─────────────────────────────────────────────────
        "get-groups" => get_groups(state).await,
        "save-group" => save_group(state, args).await,
        "update-group" => update_group(state, args).await,
        "delete-group" => delete_group(state, args).await,
        "move-connection-to-group" => move_connection_to_group(state, args).await,

        // ── Import / Export ───────────────────────────────────────────────────
        "export-connections" => export_connections(state).await,
        "import-connections" => import_connections(state, args).await,

        // ── URL parsing ───────────────────────────────────────────────────────
        "parse-connection-url" => parse_connection_url(args).await,
        "test-connection-with-ssh" => test_connection_with_ssh(state, args).await,

        // ── Schema & query ────────────────────────────────────────────────────
        "fetch-schema-metadata" => fetch_schema_metadata(state, args).await,
        "get-schemas" => get_schemas(state, args).await,
        "get-tables" => get_tables(state, args).await,
        "get-columns" => get_columns(state, args).await,
        "query-table" => query_table(state, args).await,
        "execute-batch" => execute_batch(state, args).await,
        "discover-container" => discover_container(state, args).await,
        "list-running-containers" => list_running_containers(state, args).await,
        "test-docker-connection" => test_docker_connection(state, args).await,

        other => Err(err(format!("unknown command: {}", other))),
    }
}

// ── Export file download endpoint ─────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct ExportFileQuery {
    path: String,
}

pub async fn export_file(Query(q): Query<ExportFileQuery>) -> impl IntoResponse {
    match std::fs::read_to_string(&q.path) {
        Ok(content) => (
            StatusCode::OK,
            [
                ("Content-Type", "application/json"),
                (
                    "Content-Disposition",
                    "attachment; filename=\"sqlator-export.json\"",
                ),
            ],
            content,
        )
            .into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e.to_string()).into_response(),
    }
}

// ── Server info ───────────────────────────────────────────────────────────────

fn server_info(state: &Arc<AppState>) -> HandlerResult {
    match &state.single_db {
        Some(cfg) => Ok(json!({
            "mode": "single-db",
            "connectionId": cfg.connection_id,
            "connectionName": cfg.connection_name,
        })),
        None => Ok(json!({ "mode": "multi-db" })),
    }
}

/// Returns an error if the server is running in single-db mode.
async fn require_multi_db<F, Fut>(state: &Arc<AppState>, f: F) -> HandlerResult
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = HandlerResult>,
{
    if state.single_db.is_some() {
        return Err(err("connection management is disabled in single-db mode"));
    }
    f().await
}

// ── Connection CRUD ───────────────────────────────────────────────────────────

async fn get_connections(state: &Arc<AppState>) -> HandlerResult {
    // In single-db mode return a synthetic locked connection.
    if let Some(cfg) = &state.single_db {
        let parsed = url::Url::parse(&cfg.url).map_err(err)?;
        let db_type = match parsed.scheme() {
            "postgres" | "postgresql" => "postgres",
            "mysql" => "mysql",
            "mariadb" => "mariadb",
            "sqlite" => "sqlite",
            "mssql" | "sqlserver" | "tds" => "mssql",
            "oracle" => "oracle",
            "clickhouse" => "clickhouse",
            _ => "unknown",
        };
        return Ok(json!([{
            "id": cfg.connection_id,
            "name": cfg.connection_name,
            "color_id": "blue",
            "db_type": db_type,
            "host": parsed.host_str().unwrap_or("localhost"),
            "port": parsed.port().unwrap_or(0),
            "database": parsed.path().trim_start_matches('/'),
            "username": parsed.username(),
            "masked_url": mask_url(&cfg.url),
            "ssh_profile_id": null,
            "group_id": null,
        }]));
    }
    let config = state.config.lock().await;
    let connections = config.get_connections().map_err(err)?;
    let infos: Vec<ConnectionInfo> = connections.iter().map(ConnectionInfo::from).collect();
    Ok(json!(infos))
}

fn mask_url(url: &str) -> String {
    if let Ok(mut parsed) = url::Url::parse(url) {
        if parsed.password().is_some() {
            let _ = parsed.set_password(Some("***"));
        }
        parsed.to_string()
    } else {
        url.to_string()
    }
}

fn detect_db_type(url: &str) -> Result<(&'static str, u16), (StatusCode, String)> {
    let parsed = url::Url::parse(url).map_err(err)?;
    match parsed.scheme() {
        "postgres" | "postgresql" => Ok(("postgres", 5432)),
        "mysql" => Ok(("mysql", 3306)),
        "mariadb" => Ok(("mariadb", 3306)),
        "sqlite" => Ok(("sqlite", 0)),
        "mssql" | "sqlserver" | "tds" => Ok(("mssql", 1433)),
        "oracle" => Ok(("oracle", 1521)),
        "clickhouse" => Ok(("clickhouse", 8123)),
        s => Err(err(format!("unsupported scheme: {}", s))),
    }
}

fn build_saved_connection(id: String, config: ConnectionConfig) -> Result<SavedConnection, (StatusCode, String)> {
    let parsed = url::Url::parse(&config.url).map_err(err)?;
    let (db_type, default_port) = detect_db_type(&config.url)?;
    let db_type = if db_type == "mysql" && parsed.scheme() == "mariadb" {
        "mariadb"
    } else {
        db_type
    };
    let connection_type = resolve_connection_type(&config);
    Ok(SavedConnection {
        id,
        name: config.name,
        color_id: config.color_id,
        db_type: db_type.to_string(),
        host: parsed.host_str().unwrap_or("localhost").to_string(),
        port: parsed.port().unwrap_or(default_port),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        url: config.url,
        ssh_profile_id: config.ssh_profile_id,
        group_id: config.group_id,
        connection_type,
        container_name: config.container_name,
        container_port: config.container_port,
    })
}

async fn save_connection(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let config: ConnectionConfig = get(args, "config")?;
    let conn = build_saved_connection(uuid::Uuid::new_v4().to_string(), config)?;
    state.config.lock().await.save_connection(conn.clone()).map_err(err)?;
    Ok(json!(ConnectionInfo::from(&conn)))
}

async fn update_connection(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let id: String = get(args, "id")?;
    let config: ConnectionConfig = get(args, "config")?;
    let conn = build_saved_connection(id, config)?;
    state.config.lock().await.update_connection(conn.clone()).map_err(err)?;
    Ok(json!(ConnectionInfo::from(&conn)))
}

async fn clone_connection(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let id: String = get(args, "id")?;
    let config = state.config.lock().await;
    let connections = config.get_connections().map_err(err)?;
    let original = connections
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| err(format!("connection '{}' not found", id)))?;
    let mut cloned = original.clone();
    cloned.id = uuid::Uuid::new_v4().to_string();
    cloned.name = format!("{} (Copy)", original.name);
    drop(config);
    state.config.lock().await.save_connection(cloned.clone()).map_err(err)?;
    Ok(json!(ConnectionInfo::from(&cloned)))
}

async fn delete_connection(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let id: String = get(args, "id")?;
    state.db.disconnect(&id).await;
    state.config.lock().await.delete_connection(&id).map_err(err)?;
    Ok(json!(null))
}

async fn test_connection(args: &Value) -> HandlerResult {
    let url: String = get(args, "url")?;
    let result = DbManager::test_connection(&url).await.map_err(err)?;
    Ok(json!(result))
}

async fn connect_database(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let id: String = get(args, "id")?;
    // In single-db mode the pool is pre-connected at startup — no-op.
    if let Some(cfg) = &state.single_db {
        if id == cfg.connection_id {
            return Ok(json!(null));
        }
        return Err(err("cannot connect to other databases in single-db mode"));
    }
    let config = state.config.lock().await;
    let connections = config.get_connections().map_err(err)?;
    let conn = connections
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| err(format!("connection '{}' not found", id)))?;
    let url = conn.url.clone();
    drop(config);
    state.db.connect(&id, &url).await.map_err(err)?;
    Ok(json!(null))
}

async fn disconnect_database(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let id: String = get(args, "id")?;
    state.db.disconnect(&id).await;
    Ok(json!(null))
}

// ── Query & tab state ─────────────────────────────────────────────────────────

async fn get_query(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let connection_id: String = get(args, "connectionId")?;
    let q = state.config.lock().await.get_query(&connection_id).map_err(err)?;
    Ok(json!(q))
}

async fn save_query(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let connection_id: String = get(args, "connectionId")?;
    let query: String = get(args, "query")?;
    state.config.lock().await.save_query(&connection_id, &query).map_err(err)?;
    Ok(json!(null))
}

async fn get_tab_state(state: &Arc<AppState>) -> HandlerResult {
    let ts = state.config.lock().await.get_tab_state().map_err(err)?;
    Ok(json!(ts))
}

async fn save_tab_state(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let tab_state = args["tabState"].clone();
    state.config.lock().await.save_tab_state(tab_state).map_err(err)?;
    Ok(json!(null))
}

// ── Theme ─────────────────────────────────────────────────────────────────────

async fn get_theme(state: &Arc<AppState>) -> HandlerResult {
    let theme = state.config.lock().await.get_theme().map_err(err)?;
    Ok(json!(theme))
}

async fn save_theme(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let theme: String = get(args, "theme")?;
    state.config.lock().await.save_theme(&theme).map_err(err)?;
    Ok(json!(null))
}

// ── SSH config ────────────────────────────────────────────────────────────────

async fn list_ssh_hosts() -> HandlerResult {
    let hosts = config_parser::load_ssh_config().map_err(err)?;
    Ok(json!(hosts))
}

// ── SSH profiles ──────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct SshProfileConfig {
    name: String,
    host: String,
    port: u16,
    username: String,
    auth_method: String,
    key_path: Option<String>,
    password: Option<String>,
    key_passphrase: Option<String>,
    proxy_jump: Vec<sqlator_core::models::SshJumpHost>,
    local_port_binding: Option<u16>,
    keepalive_interval: Option<u32>,
}

fn parse_auth_method(s: &str) -> Result<sqlator_core::models::SshAuthMethod, (StatusCode, String)> {
    match s {
        "key" => Ok(sqlator_core::models::SshAuthMethod::Key),
        "password" => Ok(sqlator_core::models::SshAuthMethod::Password),
        "agent" => Ok(sqlator_core::models::SshAuthMethod::Agent),
        other => Err(err(format!("unknown auth method: {}", other))),
    }
}

async fn get_ssh_profiles(state: &Arc<AppState>) -> HandlerResult {
    let profiles = state.config.lock().await.get_ssh_profiles().map_err(err)?;
    Ok(json!(profiles))
}

async fn save_ssh_profile(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let c: SshProfileConfig = get(args, "config")?;
    let id = uuid::Uuid::new_v4().to_string();
    let auth_method = parse_auth_method(&c.auth_method)?;
    let profile = SshProfile {
        id: id.clone(),
        name: c.name,
        host: c.host,
        port: c.port,
        username: c.username,
        auth_method,
        key_path: c.key_path,
        proxy_jump: c.proxy_jump,
        local_port_binding: c.local_port_binding,
        keepalive_interval: c.keepalive_interval,
    };
    state.config.lock().await.save_ssh_profile(profile.clone()).map_err(err)?;
    store_ssh_credentials(&state, &id, c.password.as_deref(), c.key_passphrase.as_deref())?;
    Ok(json!(profile))
}

async fn update_ssh_profile(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let id: String = get(args, "id")?;
    let c: SshProfileConfig = get(args, "config")?;
    {
        let config = state.config.lock().await;
        config
            .get_ssh_profile(&id)
            .map_err(err)?
            .ok_or_else(|| err(format!("SSH profile '{}' not found", id)))?;
    }
    let auth_method = parse_auth_method(&c.auth_method)?;
    let profile = SshProfile {
        id: id.clone(),
        name: c.name,
        host: c.host,
        port: c.port,
        username: c.username,
        auth_method,
        key_path: c.key_path,
        proxy_jump: c.proxy_jump,
        local_port_binding: c.local_port_binding,
        keepalive_interval: c.keepalive_interval,
    };
    state.config.lock().await.update_ssh_profile(profile.clone()).map_err(err)?;
    store_ssh_credentials(&state, &id, c.password.as_deref(), c.key_passphrase.as_deref())?;
    Ok(json!(profile))
}

fn store_ssh_credentials(
    state: &Arc<AppState>,
    id: &str,
    password: Option<&str>,
    passphrase: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    if let Some(pw) = password {
        if !pw.is_empty() {
            state.credentials.store_credential(id, "password", pw).map_err(err)?;
        }
    }
    if let Some(pp) = passphrase {
        if !pp.is_empty() {
            state.credentials.store_credential(id, "passphrase", pp).map_err(err)?;
        }
    }
    Ok(())
}

async fn delete_ssh_profile(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let id: String = get(args, "id")?;
    state.config.lock().await.delete_ssh_profile(&id).map_err(err)?;
    state.credentials.delete_all_credentials(&id).map_err(err)?;
    Ok(json!(null))
}

async fn connections_using_ssh_profile(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let profile_id: String = get(args, "profileId")?;
    let ids = state
        .config
        .lock()
        .await
        .connections_using_profile(&profile_id)
        .map_err(err)?;
    Ok(json!(ids))
}

// ── SSH tunnels ───────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct SshTunnelRequest {
    profile_id: String,
    host: String,
    port: u16,
    username: String,
    auth_method: String,
    key_path: Option<String>,
    key_passphrase: Option<String>,
    password: Option<String>,
    target_host: String,
    target_port: u16,
}

#[derive(serde::Serialize)]
struct SshTunnelInfo {
    profile_id: String,
    local_port: u16,
    target_host: String,
    target_port: u16,
}

async fn create_ssh_tunnel(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let req: SshTunnelRequest = get(args, "request")?;
    let auth_config = match req.auth_method.as_str() {
        "key" => {
            let key_path = req.key_path.clone().unwrap_or_default();
            if let Some(passphrase) = &req.key_passphrase {
                SshAuthConfig::with_key_and_passphrase(&req.username, key_path, passphrase)
            } else {
                SshAuthConfig::with_key(&req.username, key_path)
            }
        }
        "password" => {
            let pw = req.password.clone().unwrap_or_default();
            SshAuthConfig::with_password(&req.username, pw)
        }
        "agent" => SshAuthConfig::with_agent(&req.username),
        m => return Err(err(format!("unsupported auth method: {}", m))),
    };

    let ssh_config = SshHostConfig::new(&req.host, req.port, auth_config.clone());
    let tunnel = SshTunnel::create(
        req.profile_id.clone(),
        &ssh_config,
        auth_config,
        req.target_host.clone(),
        req.target_port,
        vec![],
    )
    .await
    .map_err(err)?;

    SshTunnel::start_forwarding(&tunnel).await.map_err(err)?;

    let info = SshTunnelInfo {
        profile_id: tunnel.profile_id.clone(),
        local_port: tunnel.local_port,
        target_host: tunnel.target_host.clone(),
        target_port: tunnel.target_port,
    };
    state.tunnels.insert(req.profile_id, tunnel);
    Ok(json!(info))
}

async fn close_ssh_tunnel(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let profile_id: String = get(args, "profileId")?;
    let (_, tunnel) = state
        .tunnels
        .remove(&profile_id)
        .ok_or_else(|| err(format!("tunnel '{}' not found", profile_id)))?;
    SshTunnel::close(tunnel).await.map_err(err)?;
    Ok(json!(null))
}

async fn get_active_tunnels(state: &Arc<AppState>) -> HandlerResult {
    let tunnels: Vec<SshTunnelInfo> = state
        .tunnels
        .iter()
        .map(|e| SshTunnelInfo {
            profile_id: e.profile_id.clone(),
            local_port: e.local_port,
            target_host: e.target_host.clone(),
            target_port: e.target_port,
        })
        .collect();
    Ok(json!(tunnels))
}

// ── Credential storage ────────────────────────────────────────────────────────

async fn get_storage_mode(state: &Arc<AppState>) -> HandlerResult {
    Ok(json!(state.credentials.mode().to_string()))
}

async fn set_storage_mode(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let mode_str: String = get(args, "mode")?;
    let migrate: bool = get(args, "migrate")?;
    let new_mode: StorageMode = mode_str.parse().map_err(err)?;

    if migrate {
        let profiles = state.config.lock().await.get_ssh_profiles().map_err(err)?;
        let ids: Vec<String> = profiles.iter().map(|p| p.id.clone()).collect();
        state.credentials.migrate_to(&new_mode, &ids).map_err(err)?;
    }

    state.credentials.set_mode(new_mode);
    state.config.lock().await.save_storage_mode(&mode_str).map_err(err)?;
    Ok(json!(null))
}

async fn create_vault(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let password: String = get(args, "password")?;
    state.credentials.vault.create(&password).map_err(err)?;
    state.credentials.set_mode(StorageMode::Vault);
    state.config.lock().await.save_storage_mode("vault").map_err(err)?;
    Ok(json!(null))
}

async fn unlock_vault(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let password: String = get(args, "password")?;
    state.credentials.vault.unlock(&password).map_err(err)?;
    Ok(json!(null))
}

async fn get_vault_settings(state: &Arc<AppState>) -> HandlerResult {
    let timeout_secs = state.config.lock().await.get_vault_timeout_secs().map_err(err)?;
    Ok(json!(VaultSettings { timeout_secs }))
}

async fn save_vault_settings(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let settings: VaultSettings = get(args, "settings")?;
    state.credentials.vault.set_timeout(settings.timeout_secs);
    state.config.lock().await.save_vault_timeout_secs(settings.timeout_secs).map_err(err)?;
    Ok(json!(null))
}

// ── Connection groups ─────────────────────────────────────────────────────────

async fn get_groups(state: &Arc<AppState>) -> HandlerResult {
    let groups = state.config.lock().await.get_groups().map_err(err)?;
    Ok(json!(groups))
}

#[derive(serde::Deserialize)]
struct SaveGroupPayload {
    name: String,
    color: Option<String>,
    parent_group_id: Option<String>,
}

async fn save_group(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let payload: SaveGroupPayload = get(args, "payload")?;
    let config = state.config.lock().await;
    let groups = config.get_groups().map_err(err)?;
    let order = groups.len() as u32;
    drop(config);
    let group = ConnectionGroup {
        id: uuid::Uuid::new_v4().to_string(),
        name: payload.name,
        color: payload.color,
        parent_group_id: payload.parent_group_id,
        order,
        collapsed: false,
    };
    state.config.lock().await.save_group(group.clone()).map_err(err)?;
    Ok(json!(group))
}

async fn update_group(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let group: ConnectionGroup = get(args, "group")?;
    state.config.lock().await.update_group(group.clone()).map_err(err)?;
    Ok(json!(group))
}

async fn delete_group(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let id: String = get(args, "id")?;
    state.config.lock().await.delete_group(&id).map_err(err)?;
    Ok(json!(null))
}

async fn move_connection_to_group(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let connection_id: String = get(args, "connectionId")?;
    let group_id: Option<String> = get_opt(args, "groupId")?;
    let config = state.config.lock().await;
    config
        .move_connection_to_group(&connection_id, group_id.as_deref())
        .map_err(err)?;
    let connections = config.get_connections().map_err(err)?;
    let conn = connections
        .iter()
        .find(|c| c.id == connection_id)
        .ok_or_else(|| err(format!("connection '{}' not found", connection_id)))?;
    Ok(json!(ConnectionInfo::from(conn)))
}

// ── Import / Export ───────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct ExportedConnection {
    name: String,
    color_id: String,
    db_type: String,
    host: String,
    port: u16,
    database: String,
    username: String,
    ssh_profile_name: Option<String>,
    group_name: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ExportedSshProfile {
    name: String,
    host: String,
    port: u16,
    username: String,
    auth_method: String,
    key_path: Option<String>,
    proxy_jump: Vec<ExportedJumpHost>,
    local_port_binding: Option<u16>,
    keepalive_interval: Option<u32>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ExportedJumpHost {
    host: String,
    port: u16,
    username: String,
    auth_method: String,
    key_path: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ExportedGroup {
    name: String,
    color: Option<String>,
    parent_group_name: Option<String>,
    order: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ExportFile {
    version: String,
    exported_at: String,
    connections: Vec<ExportedConnection>,
    ssh_profiles: Vec<ExportedSshProfile>,
    groups: Vec<ExportedGroup>,
}

#[derive(serde::Serialize)]
struct ImportResult {
    groups_added: usize,
    profiles_added: usize,
    connections_added: usize,
    connections_skipped: usize,
}

fn build_export(state_config: &sqlator_core::config::ConfigManager) -> Result<String, (StatusCode, String)> {
    let connections = state_config.get_connections().map_err(err)?;
    let profiles = state_config.get_ssh_profiles().map_err(err)?;
    let groups = state_config.get_groups().map_err(err)?;

    let profile_names: std::collections::HashMap<String, String> =
        profiles.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
    let group_names: std::collections::HashMap<String, String> =
        groups.iter().map(|g| (g.id.clone(), g.name.clone())).collect();

    let exported_connections: Vec<ExportedConnection> = connections
        .iter()
        .map(|c| ExportedConnection {
            name: c.name.clone(),
            color_id: c.color_id.clone(),
            db_type: c.db_type.clone(),
            host: c.host.clone(),
            port: c.port,
            database: c.database.clone(),
            username: c.username.clone(),
            ssh_profile_name: c.ssh_profile_id.as_ref().and_then(|id| profile_names.get(id)).cloned(),
            group_name: c.group_id.as_ref().and_then(|id| group_names.get(id)).cloned(),
        })
        .collect();

    let exported_profiles: Vec<ExportedSshProfile> = profiles
        .iter()
        .map(|p| ExportedSshProfile {
            name: p.name.clone(),
            host: p.host.clone(),
            port: p.port,
            username: p.username.clone(),
            auth_method: format!("{:?}", p.auth_method).to_lowercase(),
            key_path: p.key_path.clone(),
            proxy_jump: p.proxy_jump.iter().map(|j| ExportedJumpHost {
                host: j.host.clone(),
                port: j.port,
                username: j.username.clone(),
                auth_method: format!("{:?}", j.auth_method).to_lowercase(),
                key_path: j.key_path.clone(),
            }).collect(),
            local_port_binding: p.local_port_binding,
            keepalive_interval: p.keepalive_interval,
        })
        .collect();

    let exported_groups: Vec<ExportedGroup> = groups
        .iter()
        .map(|g| ExportedGroup {
            name: g.name.clone(),
            color: g.color.clone(),
            parent_group_name: g.parent_group_id.as_ref().and_then(|id| group_names.get(id)).cloned(),
            order: g.order,
        })
        .collect();

    let export_file = ExportFile {
        version: "1.0".to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        connections: exported_connections,
        ssh_profiles: exported_profiles,
        groups: exported_groups,
    };

    serde_json::to_string_pretty(&export_file).map_err(server_err)
}

async fn export_connections(state: &Arc<AppState>) -> HandlerResult {
    let config = state.config.lock().await;
    let json = build_export(&config)?;
    drop(config);

    // Write to a temp file; the web-adapter's openPath will call GET /api/export-file?path=
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let filename = format!("sqlator-export-{}.json", date);
    let dir = dirs::download_dir()
        .or_else(dirs::home_dir)
        .or_else(|| std::env::temp_dir().into())
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    let path = dir.join(&filename);
    std::fs::write(&path, &json).map_err(server_err)?;
    Ok(json!(path.to_string_lossy()))
}

async fn import_connections(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let json_str: String = get(args, "json")?;
    let duplicate_mode: String = get(args, "duplicateMode")?;
    let rename = duplicate_mode == "rename";
    let file: ExportFile = serde_json::from_str(&json_str).map_err(err)?;

    let config = state.config.lock().await;
    let existing_groups = config.get_groups().map_err(err)?;
    let existing_group_names: std::collections::HashSet<String> =
        existing_groups.iter().map(|g| g.name.clone()).collect();
    let mut group_id_map: std::collections::HashMap<String, String> =
        existing_groups.iter().map(|g| (g.name.clone(), g.id.clone())).collect();
    let mut groups_added = 0usize;

    let mut remaining: Vec<&ExportedGroup> = file.groups.iter().collect();
    for _ in 0..3 {
        let mut next = Vec::new();
        for eg in remaining {
            if let Some(ref pn) = eg.parent_group_name {
                if !group_id_map.contains_key(pn.as_str()) {
                    next.push(eg);
                    continue;
                }
            }
            if existing_group_names.contains(&eg.name) {
                continue;
            }
            let new_id = uuid::Uuid::new_v4().to_string();
            let group = ConnectionGroup {
                id: new_id.clone(),
                name: eg.name.clone(),
                color: eg.color.clone(),
                parent_group_id: eg.parent_group_name.as_ref().and_then(|n| group_id_map.get(n)).cloned(),
                order: eg.order,
                collapsed: false,
            };
            config.save_group(group).map_err(err)?;
            group_id_map.insert(eg.name.clone(), new_id);
            groups_added += 1;
        }
        remaining = next;
        if remaining.is_empty() { break; }
    }

    let existing_profiles = config.get_ssh_profiles().map_err(err)?;
    let existing_profile_names: std::collections::HashSet<String> =
        existing_profiles.iter().map(|p| p.name.clone()).collect();
    let mut profile_id_map: std::collections::HashMap<String, String> =
        existing_profiles.iter().map(|p| (p.name.clone(), p.id.clone())).collect();
    let mut profiles_added = 0usize;

    for ep in &file.ssh_profiles {
        let final_name = if existing_profile_names.contains(&ep.name) {
            if !rename { continue; }
            unique_name(&ep.name, &profile_id_map.keys().cloned().collect())
        } else {
            ep.name.clone()
        };
        let new_id = uuid::Uuid::new_v4().to_string();
        let auth_method = parse_auth_method(&ep.auth_method)?;
        let profile = SshProfile {
            id: new_id.clone(),
            name: final_name.clone(),
            host: ep.host.clone(),
            port: ep.port,
            username: ep.username.clone(),
            auth_method,
            key_path: ep.key_path.clone(),
            proxy_jump: ep.proxy_jump.iter().map(|j| sqlator_core::models::SshJumpHost {
                host: j.host.clone(),
                port: j.port,
                username: j.username.clone(),
                auth_method: parse_auth_method(&j.auth_method).unwrap_or(sqlator_core::models::SshAuthMethod::Key),
                key_path: j.key_path.clone(),
            }).collect(),
            local_port_binding: None,
            keepalive_interval: None,
        };
        config.save_ssh_profile(profile).map_err(err)?;
        profile_id_map.insert(ep.name.clone(), new_id);
        profiles_added += 1;
    }

    let existing_conns = config.get_connections().map_err(err)?;
    let existing_conn_names: std::collections::HashSet<String> =
        existing_conns.iter().map(|c| c.name.clone()).collect();
    let mut all_conn_names = existing_conn_names.clone();
    let mut connections_added = 0usize;
    let mut connections_skipped = 0usize;

    for ec in &file.connections {
        let final_name = if existing_conn_names.contains(&ec.name) {
            if !rename { connections_skipped += 1; continue; }
            unique_name(&ec.name, &all_conn_names)
        } else {
            ec.name.clone()
        };
        let url = build_url_no_password(&ec.db_type, &ec.host, ec.port, &ec.database, &ec.username);
        let conn = SavedConnection {
            id: uuid::Uuid::new_v4().to_string(),
            name: final_name.clone(),
            color_id: ec.color_id.clone(),
            db_type: ec.db_type.clone(),
            host: ec.host.clone(),
            port: ec.port,
            database: ec.database.clone(),
            username: ec.username.clone(),
            url,
            ssh_profile_id: ec.ssh_profile_name.as_ref().and_then(|n| profile_id_map.get(n)).cloned(),
            group_id: ec.group_name.as_ref().and_then(|n| group_id_map.get(n)).cloned(),
            connection_type: sqlator_core::models::ConnectionType::default(),
            container_name: None,
            container_port: None,
        };
        config.save_connection(conn).map_err(err)?;
        all_conn_names.insert(final_name);
        connections_added += 1;
    }

    Ok(json!(ImportResult { groups_added, profiles_added, connections_added, connections_skipped }))
}

fn unique_name(base: &str, existing: &std::collections::HashSet<String>) -> String {
    let mut i = 1u32;
    loop {
        let candidate = format!("{} ({})", base, i);
        if !existing.contains(&candidate) { return candidate; }
        i += 1;
    }
}

fn build_url_no_password(db_type: &str, host: &str, port: u16, database: &str, username: &str) -> String {
    match db_type {
        "sqlite" => format!("sqlite://{}", database),
        _ => {
            let user = if !username.is_empty() { format!("{}@", username) } else { String::new() };
            format!("{}://{}{}:{}/{}", db_type, user, host, port, database)
        }
    }
}

// ── URL parsing ───────────────────────────────────────────────────────────────

async fn parse_connection_url(args: &Value) -> HandlerResult {
    let url_str: String = get(args, "url")?;
    let parsed = url::Url::parse(&url_str).map_err(err)?;
    let (db_type, default_port) = detect_db_type(&url_str)?;
    Ok(json!({
        "db_type": db_type,
        "host": parsed.host_str().unwrap_or("localhost"),
        "port": parsed.port().unwrap_or(default_port),
        "database": parsed.path().trim_start_matches('/'),
        "username": parsed.username(),
        "password": parsed.password(),
    }))
}

async fn test_connection_with_ssh(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let url: String = get(args, "url")?;
    let ssh_profile_id: String = get(args, "sshProfileId")?;

    let profile = state
        .config
        .lock()
        .await
        .get_ssh_profile(&ssh_profile_id)
        .map_err(err)?
        .ok_or_else(|| err(format!("SSH profile '{}' not found", ssh_profile_id)))?;

    let parsed_url = url::Url::parse(&url).map_err(err)?;
    let target_host = parsed_url.host_str().unwrap_or("localhost").to_string();
    let (_, default_port) = detect_db_type(&url)?;
    let target_port = parsed_url.port().unwrap_or(default_port);

    let auth_config = build_auth_config_for_profile(&profile, &state.credentials)?;
    let ssh_config = SshHostConfig::new(&profile.host, profile.port, auth_config.clone());

    let tunnel_id = format!("test-{}", uuid::Uuid::new_v4());
    let tunnel = SshTunnel::create(tunnel_id, &ssh_config, auth_config, target_host, target_port, vec![])
        .await
        .map_err(err)?;

    SshTunnel::start_forwarding(&tunnel).await.map_err(err)?;
    let local_port = tunnel.local_port;

    let mut test_url = parsed_url.clone();
    let _ = test_url.set_host(Some("127.0.0.1"));
    let _ = test_url.set_port(Some(local_port));

    let result = DbManager::test_connection(&test_url.to_string()).await;
    SshTunnel::close(tunnel).await.ok();
    let msg = result.map_err(err)?;
    Ok(json!(msg))
}

fn build_auth_config_for_profile(
    profile: &SshProfile,
    credentials: &CredentialStore,
) -> Result<SshAuthConfig, (StatusCode, String)> {
    use sqlator_core::models::SshAuthMethod;
    match profile.auth_method {
        SshAuthMethod::Key => {
            let key_path = profile.key_path.as_deref().unwrap_or_default();
            let passphrase = credentials.get_credential(&profile.id, "passphrase").map_err(err)?;
            if let Some(pp) = passphrase {
                Ok(SshAuthConfig::with_key_and_passphrase(&profile.username, key_path, pp))
            } else {
                Ok(SshAuthConfig::with_key(&profile.username, key_path))
            }
        }
        SshAuthMethod::Password => {
            let pw = credentials.get_credential(&profile.id, "password").map_err(err)?.unwrap_or_default();
            Ok(SshAuthConfig::with_password(&profile.username, pw))
        }
        SshAuthMethod::Agent => Ok(SshAuthConfig::with_agent(&profile.username)),
    }
}

// ── Schema metadata ───────────────────────────────────────────────────────────

/// Outcome of trying to identify a single editable source table.
enum TableExtract {
    /// One table reference found.
    Found(String, Option<String>),
    /// Parser succeeded but the query is a join, CTE, subquery, etc.
    NotSingleTable,
    /// Parser failed and the regex fallback could not determine a table.
    Undetermined,
}

fn extract_single_table(sql: &str) -> TableExtract {
    use sqlparser::ast::{SetExpr, Statement, TableFactor};
    use sqlparser::dialect::GenericDialect;
    use sqlparser::parser::Parser;

    let stmts = match Parser::parse_sql(&GenericDialect {}, sql) {
        Ok(s) => s,
        Err(_) => {
            return match extract_table_regex(sql) {
                Some((t, s)) => TableExtract::Found(t, s),
                None => TableExtract::Undetermined,
            };
        }
    };

    let Some(stmt) = stmts.into_iter().next() else {
        return TableExtract::NotSingleTable;
    };
    let query = match stmt {
        Statement::Query(q) => q,
        _ => return TableExtract::NotSingleTable,
    };

    if query.with.is_some() {
        return TableExtract::NotSingleTable;
    }

    let body = match *query.body {
        SetExpr::Select(sel) => sel,
        _ => return TableExtract::NotSingleTable,
    };

    if body.from.len() != 1 {
        return TableExtract::NotSingleTable;
    }
    let twj = &body.from[0];
    if !twj.joins.is_empty() {
        return TableExtract::NotSingleTable;
    }

    match &twj.relation {
        TableFactor::Table { name, .. } => {
            let idents: Vec<String> = name.0.iter().map(|i| i.value.clone()).collect();
            match idents.len() {
                1 => TableExtract::Found(idents[0].clone(), None),
                2 => TableExtract::Found(idents[1].clone(), Some(idents[0].clone())),
                _ => TableExtract::NotSingleTable,
            }
        }
        _ => TableExtract::NotSingleTable,
    }
}

/// Regex/heuristic fallback when sqlparser cannot parse the statement.
///
/// Delimits the FROM region first, then rejects commas/`JOIN` only inside that
/// region — so commas in the SELECT list are fine, but `FROM t1 , t2` is not.
/// Indexes against the original string (not an uppercased copy) so non-ASCII
/// before `FROM` cannot shift offsets.
fn extract_table_regex(sql: &str) -> Option<(String, Option<String>)> {
    let from_idx = sql
        .as_bytes()
        .windows(6)
        .position(|w| w.eq_ignore_ascii_case(b" from "))?;
    let after_from = sql[from_idx + 6..].trim_start();
    let from_region = delimit_from_region(after_from);

    // Multi-table FROM: comma or JOIN inside the region only
    if from_region.contains(',') {
        return None;
    }
    if from_region
        .as_bytes()
        .windows(6)
        .any(|w| w.eq_ignore_ascii_case(b" join "))
    {
        return None;
    }

    let table_token: String = from_region
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ';' && *c != '\\')
        .collect();
    if table_token.is_empty() {
        return None;
    }

    split_schema_table(&table_token)
}

/// Slice of `after_from` up to the next clause keyword or `;`.
fn delimit_from_region(after_from: &str) -> &str {
    const KEYWORDS: &[&str] = &[
        "where", "group", "having", "order", "limit", "offset", "fetch", "window", "union",
        "intersect", "except", "for", "into",
    ];

    let bytes = after_from.as_bytes();
    let mut end = after_from.len();
    if let Some(semi) = after_from.find(';') {
        end = semi;
    }

    let mut i = 0;
    while i < end {
        while i < end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end {
            break;
        }
        let token_start = i;
        while i < end && !bytes[i].is_ascii_whitespace() && bytes[i] != b';' {
            i += 1;
        }
        let token = &after_from[token_start..i];
        if KEYWORDS.iter().any(|kw| token.eq_ignore_ascii_case(kw)) {
            end = token_start;
            break;
        }
    }

    after_from[..end].trim()
}

fn unquote_ident(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']')
        .to_string()
}

/// Split `schema.table` on the first dot not inside quotes/brackets.
fn split_schema_table(token: &str) -> Option<(String, Option<String>)> {
    let mut in_quotes: Option<char> = None;
    let mut dot_pos = None;
    for (i, c) in token.char_indices() {
        match (in_quotes, c) {
            (None, '"' | '`') => in_quotes = Some(c),
            (None, '[') => in_quotes = Some(']'),
            (Some(q), c) if c == q => in_quotes = None,
            (None, '.') => {
                dot_pos = Some(i);
                break;
            }
            _ => {}
        }
    }

    match dot_pos {
        Some(i) => Some((
            unquote_ident(&token[i + 1..]),
            Some(unquote_ident(&token[..i])),
        )),
        None => Some((unquote_ident(token), None)),
    }
}

fn non_editable_meta(reason: &str) -> TableMeta {
    TableMeta {
        table_name: String::new(),
        schema: None,
        columns: vec![],
        primary_key: sqlator_core::PrimaryKeyMeta {
            columns: vec![],
            exists: false,
        },
        is_editable: false,
        editability_reason: Some(reason.into()),
    }
}

async fn fetch_schema_metadata(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let connection_id: String = get(args, "connectionId")?;
    let sql: String = get(args, "sql")?;

    let is_select = {
        let t = sql.trim().to_uppercase();
        t.starts_with("SELECT") || t.starts_with("WITH")
    };
    if !is_select { return Ok(json!(null)); }

    let (table_name, schema_name) = match extract_single_table(&sql) {
        TableExtract::Found(t, s) => (t, s),
        TableExtract::NotSingleTable => {
            return Ok(json!(non_editable_meta(
                "Cannot edit: query joins multiple tables or uses a subquery",
            )));
        }
        TableExtract::Undetermined => {
            return Ok(json!(non_editable_meta(
                "Cannot determine a single source table for this query",
            )));
        }
    };


    let cache_key = format!("{connection_id}:{schema_name:?}:{table_name}");
    if let Some(cached) = state.schema_cache.get(&cache_key) {
        let (meta, expires_at) = cached.clone();
        if std::time::Instant::now() < expires_at {
            return Ok(json!(meta));
        }
        drop(cached);
        state.schema_cache.remove(&cache_key);
    }

    let meta = state
        .db
        .fetch_schema_metadata(&connection_id, &table_name, schema_name.as_deref())
        .await
        .map_err(err)?;

    let expires = std::time::Instant::now() + std::time::Duration::from_secs(300);
    state.schema_cache.insert(cache_key, (meta.clone(), expires));
    Ok(json!(meta))
}

async fn get_schemas(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let connection_id: String = get(args, "connectionId")?;
    let schemas: Vec<SchemaInfo> = state.db.get_schemas(&connection_id).await.map_err(err)?;
    Ok(json!(schemas))
}

async fn get_tables(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let connection_id: String = get(args, "connectionId")?;
    let schema: Option<String> = get_opt(args, "schema")?;
    let tables: Vec<TableInfo> = state
        .db
        .get_tables(&connection_id, schema.as_deref())
        .await
        .map_err(err)?;
    Ok(json!(tables))
}

async fn get_columns(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let connection_id: String = get(args, "connectionId")?;
    let table_name: String = get(args, "tableName")?;
    let schema: Option<String> = get_opt(args, "schema")?;
    let cols: Vec<SchemaColumnInfo> = state
        .db
        .get_columns(&connection_id, &table_name, schema.as_deref())
        .await
        .map_err(err)?;
    Ok(json!(cols))
}

async fn query_table(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let params: TableQueryParams = get(args, "params")?;
    let connection_id = params.connection_id.clone();
    let result: TableQueryResult = state.db.query_table(&connection_id, &params).await.map_err(err)?;
    Ok(json!(result))
}

async fn execute_batch(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let connection_id: String = get(args, "connectionId")?;
    let batch: SqlBatch = get(args, "batch")?;
    let result: BatchResult = state.db.execute_batch(&connection_id, &batch).await.map_err(err)?;
    Ok(json!(result))
}

// ── Docker Container Discovery ────────────────────────────────────────────────

async fn discover_container(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let ssh_profile_id: String = get(args, "sshProfileId")?;
    let container_name: String = get(args, "containerName")?;

    let profile = state.config.lock().await
        .get_ssh_profile(&ssh_profile_id).map_err(err)?
        .ok_or_else(|| err(format!("SSH profile '{}' not found", ssh_profile_id)))?;

    let auth_config = build_auth_config_for_profile(&profile, &state.credentials)?;
    let ssh_config = SshHostConfig::new(&profile.host, profile.port, auth_config.clone());
    let jump_hosts = build_jump_hosts_for_profile(&profile)?;

    let info = ContainerInspector::inspect(
        &ssh_config,
        auth_config,
        jump_hosts,
        &container_name,
    )
    .await
    .map_err(|e| err(format!("{}", e)))?;

    Ok(json!({
        "ip_address": info.ip_address,
        "status": match info.status {
            sqlator_core::ContainerStatus::Running => "running",
            sqlator_core::ContainerStatus::Stopped => "stopped",
            sqlator_core::ContainerStatus::NotFound => "not_found",
        },
        "ports": info.ports,
        "database_type_hint": info.database_type_hint,
    }))
}

async fn list_running_containers(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let ssh_profile_id: String = get(args, "sshProfileId")?;

    let profile = state.config.lock().await
        .get_ssh_profile(&ssh_profile_id).map_err(err)?
        .ok_or_else(|| err(format!("SSH profile '{}' not found", ssh_profile_id)))?;

    let auth_config = build_auth_config_for_profile(&profile, &state.credentials)?;
    let ssh_config = SshHostConfig::new(&profile.host, profile.port, auth_config.clone());
    let jump_hosts = build_jump_hosts_for_profile(&profile)?;

    let containers = ContainerInspector::list_running(
        &ssh_config,
        auth_config,
        jump_hosts,
    )
    .await
    .map_err(|e| err(format!("{}", e)))?;

    Ok(json!(containers))
}

async fn test_docker_connection(state: &Arc<AppState>, args: &Value) -> HandlerResult {
    let ssh_profile_id: String = get(args, "sshProfileId")?;
    let container_name: String = get(args, "containerName")?;
    let container_port: u16 = get(args, "containerPort")?;
    let url: String = get(args, "url")?;

    let profile = state.config.lock().await
        .get_ssh_profile(&ssh_profile_id).map_err(err)?
        .ok_or_else(|| err(format!("SSH profile '{}' not found", ssh_profile_id)))?;

    let auth_config = build_auth_config_for_profile(&profile, &state.credentials)?;
    let ssh_config = SshHostConfig::new(&profile.host, profile.port, auth_config.clone());
    let jump_hosts = build_jump_hosts_for_profile(&profile)?;

    let container_info = ContainerInspector::inspect(
        &ssh_config,
        auth_config.clone(),
        jump_hosts.clone(),
        &container_name,
    )
    .await
    .map_err(|e| err(format!("Docker inspect failed: {}", e)))?;

    if container_info.status != sqlator_core::ContainerStatus::Running {
        return Err(err(format!("Container '{}' is not running", container_name)));
    }

    let tunnel_id = format!("test-docker-{}", uuid::Uuid::new_v4());
    let tunnel = SshTunnel::create(
        tunnel_id,
        &ssh_config,
        auth_config,
        container_info.ip_address.clone(),
        container_port,
        jump_hosts,
    )
    .await
    .map_err(err)?;

    SshTunnel::start_forwarding(&tunnel).await.map_err(err)?;
    let local_port = tunnel.local_port;

    let parsed_url = url::Url::parse(&url).map_err(err)?;
    let mut test_url = parsed_url;
    let _ = test_url.set_host(Some("127.0.0.1"));
    let _ = test_url.set_port(Some(local_port));

    let result = DbManager::test_connection(&test_url.to_string()).await;
    SshTunnel::close(tunnel).await.ok();
    let msg = result.map_err(err)?;
    Ok(json!(msg))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn resolve_connection_type(config: &ConnectionConfig) -> ConnectionType {
    match config.connection_type {
        Some(ref ct) => ct.clone(),
        None => {
            if config.container_name.is_some() && config.ssh_profile_id.is_some() {
                ConnectionType::DockerContainer
            } else if config.ssh_profile_id.is_some() {
                ConnectionType::SshTunnel
            } else {
                ConnectionType::Direct
            }
        }
    }
}

fn build_jump_hosts_for_profile(
    profile: &SshProfile,
) -> Result<Vec<(SshHostConfig, SshAuthConfig)>, (StatusCode, String)> {
    use sqlator_core::models::SshAuthMethod;
    profile
        .proxy_jump
        .iter()
        .map(|jump| {
            let auth = match jump.auth_method {
                SshAuthMethod::Key => {
                    let key_path = jump.key_path.as_deref().unwrap_or_default();
                    SshAuthConfig::with_key(&jump.username, key_path)
                }
                SshAuthMethod::Agent => SshAuthConfig::with_agent(&jump.username),
                SshAuthMethod::Password => {
                    return Err(err(format!(
                        "Jump host '{}' uses password auth, which is not supported for stored jump hosts",
                        jump.host
                    )))
                }
            };
            let config = SshHostConfig::new(&jump.host, jump.port, auth.clone());
            Ok((config, auth))
        })
        .collect()
}

#[cfg(test)]
mod table_extract_tests {
    use super::*;

    #[test]
    fn regex_simple_table() {
        assert_eq!(
            extract_table_regex("SELECT a FROM t"),
            Some(("t".into(), None))
        );
    }

    #[test]
    fn regex_comma_in_select_list() {
        assert_eq!(
            extract_table_regex("SELECT a, b FROM t"),
            Some(("t".into(), None))
        );
    }

    #[test]
    fn regex_comma_in_function_call() {
        assert_eq!(
            extract_table_regex("SELECT f(a, b) FROM t"),
            Some(("t".into(), None))
        );
    }

    #[test]
    fn regex_comma_in_in_list() {
        assert_eq!(
            extract_table_regex("SELECT a FROM t WHERE x IN (1,2)"),
            Some(("t".into(), None))
        );
    }

    #[test]
    fn regex_implicit_comma_join_rejected() {
        assert_eq!(extract_table_regex("SELECT a, b FROM t1, t2"), None);
        assert_eq!(extract_table_regex("SELECT a, b FROM t1 , t2"), None);
    }

    #[test]
    fn regex_join_rejected() {
        assert_eq!(
            extract_table_regex("SELECT a FROM t1 JOIN t2 ON t1.id = t2.id"),
            None
        );
    }

    #[test]
    fn regex_mixed_quoting() {
        assert_eq!(
            extract_table_regex("SELECT a FROM `\"t\"`"),
            Some(("t".into(), None))
        );
    }

    #[test]
    fn regex_schema_qualified() {
        assert_eq!(
            extract_table_regex("SELECT a FROM s.t"),
            Some(("t".into(), Some("s".into())))
        );
    }

    #[test]
    fn regex_mssql_brackets() {
        assert_eq!(
            extract_table_regex("SELECT a FROM [dbo].[t]"),
            Some(("t".into(), Some("dbo".into())))
        );
    }

    #[test]
    fn regex_non_ascii_before_from_does_not_shift_offset() {
        // Dotless-i uppercases to a different byte length; must still find `t`.
        assert_eq!(
            extract_table_regex("SELECT 'ııı' FROM t"),
            Some(("t".into(), None))
        );
    }

    #[test]
    fn regex_quoted_ident_containing_dot() {
        assert_eq!(
            extract_table_regex(r#"SELECT a FROM "my.schema".t"#),
            Some(("t".into(), Some("my.schema".into())))
        );
    }

    #[test]
    fn regex_mysql_g_terminator() {
        // Confirmed sqlparser failure; fallback must still extract the table.
        assert_eq!(
            extract_table_regex("SELECT a FROM t\\G"),
            Some(("t".into(), None))
        );
        match extract_single_table("SELECT a FROM t\\G") {
            TableExtract::Found(t, s) => {
                assert_eq!(t, "t");
                assert_eq!(s, None);
            }
            other => panic!("expected Found, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn undetermined_reason_for_fallback_failure() {
        match extract_single_table("SELECT a FROM t1 , t2\\G") {
            TableExtract::Undetermined | TableExtract::NotSingleTable => {}
            TableExtract::Found(t, _) => panic!("expected non-editable, got {t}"),
        }
    }
}

#[cfg(test)]
mod group_a_tests {
    use super::*;
    use sqlator_core::config::ConfigManager;
    use sqlator_core::credentials::{CredentialStore, StorageMode};
    use sqlator_core::db::DbManager;
    use sqlator_core::models::{ConnectionGroup, SshAuthMethod, SshProfile};
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    // ── unique_name ───────────────────────────────────────────────────────────

    #[test]
    fn unique_name_never_returns_base_even_when_free() {
        // Characterization: impl always starts at "{base} (1)", never returns base itself.
        let existing = HashSet::new();
        assert_eq!(unique_name("alpha", &existing), "alpha (1)");
    }

    #[test]
    fn unique_name_skips_taken_suffixes() {
        let existing: HashSet<String> = ["alpha (1)".into(), "alpha (2)".into()].into();
        assert_eq!(unique_name("alpha", &existing), "alpha (3)");
    }

    #[test]
    fn unique_name_ignores_whether_base_itself_is_taken() {
        let existing: HashSet<String> = ["alpha".into()].into();
        assert_eq!(unique_name("alpha", &existing), "alpha (1)");
    }

    // ── build_url_no_password ─────────────────────────────────────────────────

    #[test]
    fn build_url_no_password_all_engines() {
        assert_eq!(
            build_url_no_password("postgres", "h", 5432, "db", "u"),
            "postgres://u@h:5432/db"
        );
        assert_eq!(
            build_url_no_password("mysql", "h", 3306, "db", "u"),
            "mysql://u@h:3306/db"
        );
        assert_eq!(
            build_url_no_password("mariadb", "h", 3306, "db", "u"),
            "mariadb://u@h:3306/db"
        );
        assert_eq!(
            build_url_no_password("mssql", "h", 1433, "db", "u"),
            "mssql://u@h:1433/db"
        );
        assert_eq!(
            build_url_no_password("oracle", "h", 1521, "db", "u"),
            "oracle://u@h:1521/db"
        );
        assert_eq!(
            build_url_no_password("clickhouse", "h", 8123, "db", "u"),
            "clickhouse://u@h:8123/db"
        );
        assert_eq!(
            build_url_no_password("sqlite", "ignored", 0, "/tmp/x.db", "u"),
            "sqlite:///tmp/x.db"
        );
    }

    #[test]
    fn build_url_no_password_empty_username_omits_at() {
        assert_eq!(
            build_url_no_password("postgres", "h", 5432, "db", ""),
            "postgres://h:5432/db"
        );
    }

    // ── resolve_connection_type ───────────────────────────────────────────────

    fn conn_cfg(
        connection_type: Option<ConnectionType>,
        ssh_profile_id: Option<&str>,
        container_name: Option<&str>,
    ) -> ConnectionConfig {
        ConnectionConfig {
            name: "n".into(),
            color_id: "c".into(),
            url: "postgres://h/db".into(),
            ssh_profile_id: ssh_profile_id.map(str::to_string),
            group_id: None,
            connection_type,
            container_name: container_name.map(str::to_string),
            container_port: None,
        }
    }

    #[test]
    fn resolve_connection_type_explicit_wins() {
        let cfg = conn_cfg(Some(ConnectionType::Direct), Some("ssh"), Some("ctr"));
        assert_eq!(resolve_connection_type(&cfg), ConnectionType::Direct);

        let cfg = conn_cfg(Some(ConnectionType::LocalDockerContainer), None, None);
        assert_eq!(
            resolve_connection_type(&cfg),
            ConnectionType::LocalDockerContainer
        );
    }

    #[test]
    fn resolve_connection_type_inference_paths() {
        let cfg = conn_cfg(None, Some("ssh"), Some("ctr"));
        assert_eq!(resolve_connection_type(&cfg), ConnectionType::DockerContainer);

        let cfg = conn_cfg(None, Some("ssh"), None);
        assert_eq!(resolve_connection_type(&cfg), ConnectionType::SshTunnel);

        let cfg = conn_cfg(None, None, None);
        assert_eq!(resolve_connection_type(&cfg), ConnectionType::Direct);

        // container alone does NOT infer LocalDockerContainer — falls through to Direct
        let cfg = conn_cfg(None, None, Some("ctr"));
        assert_eq!(resolve_connection_type(&cfg), ConnectionType::Direct);
    }

    // ── parse_auth_method ─────────────────────────────────────────────────────

    #[test]
    fn parse_auth_method_key_password_agent() {
        assert!(matches!(parse_auth_method("key"), Ok(SshAuthMethod::Key)));
        assert!(matches!(
            parse_auth_method("password"),
            Ok(SshAuthMethod::Password)
        ));
        assert!(matches!(parse_auth_method("agent"), Ok(SshAuthMethod::Agent)));
    }

    #[test]
    fn parse_auth_method_unknown_error_string() {
        // Ledger row 8: web uses err("unknown auth method: {}") — lowercase, via StatusCode.
        let (status, msg) = parse_auth_method("token").unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(msg, "unknown auth method: token");
    }

    // ── extract_single_table (sqlparser path) ─────────────────────────────────

    #[test]
    fn extract_single_table_simple_select() {
        match extract_single_table("SELECT a FROM t") {
            TableExtract::Found(t, s) => {
                assert_eq!(t, "t");
                assert_eq!(s, None);
            }
            other => panic!("expected Found, got discriminant {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn extract_single_table_schema_qualified() {
        match extract_single_table("SELECT a FROM public.users") {
            TableExtract::Found(t, s) => {
                assert_eq!(t, "users");
                assert_eq!(s.as_deref(), Some("public"));
            }
            other => panic!("expected Found, got discriminant {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn extract_single_table_join_is_not_single() {
        assert!(matches!(
            extract_single_table("SELECT a FROM t1 JOIN t2 ON t1.id = t2.id"),
            TableExtract::NotSingleTable
        ));
    }

    #[test]
    fn extract_single_table_subquery_is_not_single() {
        assert!(matches!(
            extract_single_table("SELECT a FROM (SELECT 1 AS a) AS sub"),
            TableExtract::NotSingleTable
        ));
    }

    #[test]
    fn extract_single_table_cte_is_not_single() {
        assert!(matches!(
            extract_single_table("WITH c AS (SELECT 1 AS a) SELECT a FROM c"),
            TableExtract::NotSingleTable
        ));
    }

    #[test]
    fn extract_single_table_non_select_is_not_single() {
        assert!(matches!(
            extract_single_table("INSERT INTO t (a) VALUES (1)"),
            TableExtract::NotSingleTable
        ));
        assert!(matches!(
            extract_single_table("UPDATE t SET a = 1"),
            TableExtract::NotSingleTable
        ));
    }

    // ── import / export fixtures ──────────────────────────────────────────────

    struct TestFixture {
        state: Arc<AppState>,
        cleanup_dir: PathBuf,
    }

    impl Drop for TestFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.cleanup_dir);
        }
    }

    fn make_fixture() -> TestFixture {
        let id = uuid::Uuid::new_v4();
        let app_name = format!("sqlator-char-a-web-{id}");
        let config = ConfigManager::new(&app_name).expect("ConfigManager");
        let cleanup_dir = dirs::config_dir()
            .expect("config dir")
            .join(&app_name);
        let vault_path = cleanup_dir.join("vault.enc");
        let state = Arc::new(AppState {
            config: Mutex::new(config),
            db: DbManager::new(),
            tunnels: dashmap::DashMap::new(),
            credentials: Arc::new(CredentialStore::new(vault_path, StorageMode::Vault)),
            schema_cache: dashmap::DashMap::new(),
            single_db: None,
        });
        TestFixture { state, cleanup_dir }
    }

    fn empty_export_json(groups: &str, profiles: &str, connections: &str) -> String {
        format!(
            r#"{{
              "version": "1.0",
              "exported_at": "2026-01-01T00:00:00Z",
              "groups": {groups},
              "ssh_profiles": {profiles},
              "connections": {connections}
            }}"#
        )
    }

    fn import_args(json: &str, mode: &str) -> Value {
        json!({ "json": json, "duplicateMode": mode })
    }

    #[tokio::test]
    async fn import_nested_groups_remaps_parent_ids() {
        let fx = make_fixture();
        let payload = empty_export_json(
            r#"[
              {"name": "child", "color": null, "parent_group_name": "parent", "order": 1},
              {"name": "parent", "color": "fff", "parent_group_name": null, "order": 0},
              {"name": "grandchild", "color": null, "parent_group_name": "child", "order": 2}
            ]"#,
            "[]",
            "[]",
        );
        let result = import_connections(&fx.state, &import_args(&payload, "skip"))
            .await
            .expect("import");
        assert_eq!(result["groups_added"], 3);

        let groups = fx.state.config.lock().await.get_groups().unwrap();
        assert_eq!(groups.len(), 3);
        let parent = groups.iter().find(|g| g.name == "parent").unwrap();
        let child = groups.iter().find(|g| g.name == "child").unwrap();
        let grandchild = groups.iter().find(|g| g.name == "grandchild").unwrap();
        assert!(parent.parent_group_id.is_none());
        assert_eq!(child.parent_group_id.as_deref(), Some(parent.id.as_str()));
        assert_eq!(grandchild.parent_group_id.as_deref(), Some(child.id.as_str()));
        assert_ne!(parent.id, "parent");
    }

    #[tokio::test]
    async fn import_parent_appearing_later_in_file() {
        let fx = make_fixture();
        let payload = empty_export_json(
            r#"[
              {"name": "child", "color": null, "parent_group_name": "parent", "order": 1},
              {"name": "parent", "color": null, "parent_group_name": null, "order": 0}
            ]"#,
            "[]",
            "[]",
        );
        let result = import_connections(&fx.state, &import_args(&payload, "skip"))
            .await
            .expect("import");
        assert_eq!(result["groups_added"], 2);
        let groups = fx.state.config.lock().await.get_groups().unwrap();
        let parent = groups.iter().find(|g| g.name == "parent").unwrap();
        let child = groups.iter().find(|g| g.name == "child").unwrap();
        assert_eq!(child.parent_group_id.as_deref(), Some(parent.id.as_str()));
    }

    #[tokio::test]
    async fn import_cyclic_parent_silently_drops_cycle() {
        let fx = make_fixture();
        let payload = empty_export_json(
            r#"[
              {"name": "a", "color": null, "parent_group_name": "b", "order": 0},
              {"name": "b", "color": null, "parent_group_name": "a", "order": 1}
            ]"#,
            "[]",
            "[]",
        );
        let result = import_connections(&fx.state, &import_args(&payload, "skip"))
            .await
            .expect("import");
        assert_eq!(result["groups_added"], 0);
        assert!(fx.state.config.lock().await.get_groups().unwrap().is_empty());
    }

    #[tokio::test]
    async fn import_group_name_collision_always_skips_even_under_rename() {
        let fx = make_fixture();
        let existing_id = uuid::Uuid::new_v4().to_string();
        fx.state
            .config
            .lock()
            .await
            .save_group(ConnectionGroup {
                id: existing_id.clone(),
                name: "Shared".into(),
                color: Some("#111".into()),
                parent_group_id: None,
                order: 0,
                collapsed: false,
            })
            .unwrap();

        let payload = empty_export_json(
            r#"[{"name": "Shared", "color": "222", "parent_group_name": null, "order": 5}]"#,
            "[]",
            "[]",
        );
        let result = import_connections(&fx.state, &import_args(&payload, "rename"))
            .await
            .expect("import rename");
        assert_eq!(result["groups_added"], 0);
        let groups = fx.state.config.lock().await.get_groups().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, existing_id);
        assert_eq!(groups[0].color.as_deref(), Some("#111"));

        let result = import_connections(&fx.state, &import_args(&payload, "skip"))
            .await
            .expect("import skip");
        assert_eq!(result["groups_added"], 0);
    }

    #[tokio::test]
    async fn import_connection_name_collision_skip_and_rename() {
        let fx = make_fixture();
        fx.state
            .config
            .lock()
            .await
            .save_connection(SavedConnection {
                id: uuid::Uuid::new_v4().to_string(),
                name: "Prod".into(),
                color_id: "blue".into(),
                db_type: "postgres".into(),
                host: "localhost".into(),
                port: 5432,
                database: "db".into(),
                username: "u".into(),
                url: "postgres://u@localhost:5432/db".into(),
                ssh_profile_id: None,
                group_id: None,
                connection_type: ConnectionType::Direct,
                container_name: None,
                container_port: None,
            })
            .unwrap();

        let payload = empty_export_json(
            "[]",
            "[]",
            r#"[
              {
                "name": "Prod",
                "color_id": "red",
                "db_type": "postgres",
                "host": "h",
                "port": 5432,
                "database": "other",
                "username": "u",
                "ssh_profile_name": null,
                "group_name": null
              }
            ]"#,
        );

        let skip = import_connections(&fx.state, &import_args(&payload, "skip"))
            .await
            .expect("skip");
        assert_eq!(skip["connections_added"], 0);
        assert_eq!(skip["connections_skipped"], 1);
        assert_eq!(
            fx.state.config.lock().await.get_connections().unwrap().len(),
            1
        );

        let rename = import_connections(&fx.state, &import_args(&payload, "rename"))
            .await
            .expect("rename");
        assert_eq!(rename["connections_added"], 1);
        assert_eq!(rename["connections_skipped"], 0);
        let names: HashSet<_> = fx
            .state
            .config
            .lock()
            .await
            .get_connections()
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert!(names.contains("Prod"));
        assert!(names.contains("Prod (1)"));
    }

    /// Ledger row 4: web import hard-codes `local_port_binding` / `keepalive_interval` to None.
    /// This round-trip would fail today; keep it ignored until phase 1 adopts Tauri's winner.
    #[tokio::test]
    #[ignore = "ledger row 4: web import drops local_port_binding / keepalive_interval"]
    async fn export_import_round_trip_preserves_ssh_tunnel_settings() {
        let fx = make_fixture();
        let profile_id = uuid::Uuid::new_v4().to_string();
        {
            let config = fx.state.config.lock().await;
            config
                .save_ssh_profile(SshProfile {
                    id: profile_id.clone(),
                    name: "bastion".into(),
                    host: "ssh.example".into(),
                    port: 22,
                    username: "deploy".into(),
                    auth_method: SshAuthMethod::Key,
                    key_path: Some("/tmp/id_rsa".into()),
                    proxy_jump: vec![],
                    local_port_binding: Some(15432),
                    keepalive_interval: Some(30),
                })
                .unwrap();
            config
                .save_group(ConnectionGroup {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: "Work".into(),
                    color: None,
                    parent_group_id: None,
                    order: 0,
                    collapsed: false,
                })
                .unwrap();
            let group_id = config
                .get_groups()
                .unwrap()
                .into_iter()
                .find(|g| g.name == "Work")
                .map(|g| g.id);
            config
                .save_connection(SavedConnection {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: "App DB".into(),
                    color_id: "green".into(),
                    db_type: "postgres".into(),
                    host: "db.internal".into(),
                    port: 5432,
                    database: "app".into(),
                    username: "app".into(),
                    url: "postgres://app@db.internal:5432/app".into(),
                    ssh_profile_id: Some(profile_id),
                    group_id,
                    connection_type: ConnectionType::SshTunnel,
                    container_name: None,
                    container_port: None,
                })
                .unwrap();
        }

        let exported = {
            let config = fx.state.config.lock().await;
            build_export(&config).expect("export")
        };
        assert!(
            !exported.to_lowercase().contains("password"),
            "export must not contain password material: {exported}"
        );

        let fx2 = make_fixture();
        let result = import_connections(&fx2.state, &import_args(&exported, "skip"))
            .await
            .expect("import");
        assert_eq!(result["groups_added"], 1);
        assert_eq!(result["profiles_added"], 1);
        assert_eq!(result["connections_added"], 1);

        let profiles = fx2.state.config.lock().await.get_ssh_profiles().unwrap();
        assert_eq!(profiles.len(), 1);
        // These assertions fail on web today (ledger row 4) — that is why the test is ignored.
        assert_eq!(profiles[0].local_port_binding, Some(15432));
        assert_eq!(profiles[0].keepalive_interval, Some(30));
        assert_eq!(profiles[0].name, "bastion");
    }
}

#[cfg(test)]
mod group_b_tests {
    use super::*;
    use sqlator_core::config::ConfigManager;
    use sqlator_core::credentials::{CredentialStore, StorageMode};
    use sqlator_core::db::DbManager;
    use sqlator_core::models::{PrimaryKeyMeta, TableMeta};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::Mutex;

    // ── Schema cache key format (same Debug-on-Option formula as Tauri) ───────

    fn schema_cache_key(
        connection_id: &str,
        schema_name: &Option<String>,
        table_name: &str,
    ) -> String {
        format!("{connection_id}:{schema_name:?}:{table_name}")
    }

    fn sample_meta(name: &str) -> TableMeta {
        TableMeta {
            table_name: name.into(),
            schema: None,
            columns: vec![],
            primary_key: PrimaryKeyMeta {
                columns: vec![],
                exists: false,
            },
            is_editable: true,
            editability_reason: None,
        }
    }

    #[test]
    fn schema_cache_key_uses_debug_on_option() {
        assert_eq!(
            schema_cache_key("c1", &Some("public".into()), "users"),
            r#"c1:Some("public"):users"#
        );
        assert_eq!(schema_cache_key("c1", &None, "users"), "c1:None:users");
        assert_eq!(
            schema_cache_key("c1", &Some("".into()), "users"),
            r#"c1:Some(""):users"#
        );
        assert_ne!(
            schema_cache_key("c1", &None, "t"),
            schema_cache_key("c1", &Some("".into()), "t")
        );
    }

    #[test]
    fn schema_cache_ttl_constant_is_300_seconds() {
        const SCHEMA_CACHE_TTL_SECS: u64 = 300;
        let expires = Instant::now() + Duration::from_secs(SCHEMA_CACHE_TTL_SECS);
        let remaining = expires.saturating_duration_since(Instant::now());
        assert!(remaining.as_secs() >= 299 && remaining.as_secs() <= 300);
    }

    #[test]
    fn schema_cache_hit_miss_and_expiry_via_dashmap() {
        let fx = make_fixture();
        let key_hit = schema_cache_key("c1", &Some("public".into()), "users");
        let key_other = schema_cache_key("c1", &None, "users");

        fx.state.schema_cache.insert(
            key_hit.clone(),
            (sample_meta("users"), Instant::now() + Duration::from_secs(300)),
        );

        {
            let cached = fx.state.schema_cache.get(&key_hit).expect("hit");
            let (m, expires_at) = cached.clone();
            assert!(Instant::now() < expires_at);
            assert_eq!(m.table_name, "users");
        }
        assert!(fx.state.schema_cache.get(&key_other).is_none());

        let expired_key = schema_cache_key("c1", &Some("public".into()), "old");
        fx.state.schema_cache.insert(
            expired_key.clone(),
            (sample_meta("old"), Instant::now() - Duration::from_secs(1)),
        );
        if let Some(cached) = fx.state.schema_cache.get(&expired_key) {
            let (_meta, expires_at) = cached.clone();
            assert!(Instant::now() >= expires_at);
            drop(cached);
            fx.state.schema_cache.remove(&expired_key);
        }
        assert!(fx.state.schema_cache.get(&expired_key).is_none());
    }

    // ── Tunnel registry ───────────────────────────────────────────────────────
    // Web connect_database does not manage tunnels (ledger row 1). Ephemeral
    // test_connection_with_ssh still closes its tunnel (handlers.rs close after test).
    // Unit-test DashMap replace pattern; live SSH teardown → Group C / #[ignore].

    #[test]
    fn tunnel_registry_replace_same_id_does_not_leak_entries() {
        let tunnels: dashmap::DashMap<String, u16> = dashmap::DashMap::new();
        tunnels.insert("conn-1".into(), 10_001);
        if let Some((_, old)) = tunnels.remove("conn-1") {
            assert_eq!(old, 10_001);
        }
        tunnels.insert("conn-1".into(), 10_002);
        assert_eq!(tunnels.len(), 1);
        assert_eq!(*tunnels.get("conn-1").unwrap(), 10_002);
    }

    #[test]
    fn appstate_tunnels_map_starts_empty() {
        let fx = make_fixture();
        assert!(fx.state.tunnels.is_empty());
    }

    #[tokio::test]
    #[ignore = "needs live SSH; overlaps Group C — ephemeral teardown on test_connection_with_ssh failure"]
    async fn tunnel_ephemeral_teardown_on_test_connection_failure_live_ssh() {
        panic!("not implemented without live SSH");
    }

    struct TestFixture {
        state: Arc<AppState>,
        cleanup_dir: PathBuf,
    }

    impl Drop for TestFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.cleanup_dir);
        }
    }

    fn make_fixture() -> TestFixture {
        let id = uuid::Uuid::new_v4();
        let app_name = format!("sqlator-char-b-web-{id}");
        let config = ConfigManager::new(&app_name).expect("ConfigManager");
        let cleanup_dir = dirs::config_dir()
            .expect("config dir")
            .join(&app_name);
        let vault_path = cleanup_dir.join("vault.enc");
        let state = Arc::new(AppState {
            config: Mutex::new(config),
            db: DbManager::new(),
            tunnels: dashmap::DashMap::new(),
            credentials: Arc::new(CredentialStore::new(vault_path, StorageMode::Vault)),
            schema_cache: dashmap::DashMap::new(),
            single_db: None,
        });
        TestFixture { state, cleanup_dir }
    }
}
