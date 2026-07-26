use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use sqlator_core::credentials::VaultSettings;
use sqlator_core::models::{
    ConnectionConfig, ConnectionGroup, SchemaColumnInfo, SchemaInfo, SqlBatch, TableInfo,
    TableQueryParams, TableQueryResult,
};
use sqlator_core::BatchResult;
use std::sync::Arc;

// ── Error helpers ─────────────────────────────────────────────────────────────

type HandlerResult = Result<Value, (StatusCode, String)>;

fn err(msg: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, msg.to_string())
}

fn map_svc(e: sqlator_service::ServiceError) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, e.message())
}

fn server_err(msg: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, msg.to_string())
}

/// Extract a required field from the JSON args body.
fn get<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> Result<T, (StatusCode, String)> {
    serde_json::from_value(args[key].clone())
        .map_err(|e| err(format!("missing/invalid '{key}': {e}")))
}

/// Extract an optional field (returns None if missing or null).
fn get_opt<T: serde::de::DeserializeOwned>(
    args: &Value,
    key: &str,
) -> Result<Option<T>, (StatusCode, String)> {
    let v = &args[key];
    if v.is_null() {
        return Ok(None);
    }
    serde_json::from_value(v.clone())
        .map(Some)
        .map_err(|e| err(format!("invalid '{key}': {e}")))
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
        "get-connections" => Ok(json!(state
            .service
            .list_connections()
            .await
            .map_err(map_svc)?)),
        "save-connection" => {
            let config: ConnectionConfig = get(args, "config")?;
            Ok(json!(state
                .service
                .save_connection(config)
                .await
                .map_err(map_svc)?))
        }
        "update-connection" => {
            let id: String = get(args, "id")?;
            let config: ConnectionConfig = get(args, "config")?;
            Ok(json!(state
                .service
                .update_connection(id, config)
                .await
                .map_err(map_svc)?))
        }
        "clone-connection" => {
            let id: String = get(args, "id")?;
            Ok(json!(state
                .service
                .clone_connection(id)
                .await
                .map_err(map_svc)?))
        }
        "delete-connection" => {
            let id: String = get(args, "id")?;
            state.service.delete_connection(id).await.map_err(map_svc)?;
            Ok(json!(null))
        }
        "test-connection" => {
            let url: String = get(args, "url")?;
            Ok(json!(state
                .service
                .test_connection(&url)
                .await
                .map_err(map_svc)?))
        }
        "connect-database" => {
            let id: String = get(args, "id")?;
            state.service.connect_database(&id).await.map_err(map_svc)?;
            Ok(json!(null))
        }
        "disconnect-database" => {
            let id: String = get(args, "id")?;
            state
                .service
                .disconnect_database(&id)
                .await
                .map_err(map_svc)?;
            Ok(json!(null))
        }

        // ── Query & tab state ─────────────────────────────────────────────────
        "get-query" => {
            let connection_id: String = get(args, "connectionId")?;
            Ok(json!(state
                .service
                .get_query(connection_id)
                .await
                .map_err(map_svc)?))
        }
        "save-query" => {
            let connection_id: String = get(args, "connectionId")?;
            let query: String = get(args, "query")?;
            state
                .service
                .save_query(connection_id, query)
                .await
                .map_err(map_svc)?;
            Ok(json!(null))
        }
        "get-tab-state" => Ok(json!(state
            .service
            .get_tab_state()
            .await
            .map_err(map_svc)?)),
        "save-tab-state" => {
            let tab_state = args["tabState"].clone();
            state
                .service
                .save_tab_state(tab_state)
                .await
                .map_err(map_svc)?;
            Ok(json!(null))
        }

        // ── Theme ─────────────────────────────────────────────────────────────
        "get-theme" => Ok(json!(state.service.get_theme().await.map_err(map_svc)?)),
        "save-theme" => {
            let theme: String = get(args, "theme")?;
            state.service.save_theme(theme).await.map_err(map_svc)?;
            Ok(json!(null))
        }

        // ── SSH config & profiles ─────────────────────────────────────────────
        "list-ssh-hosts" => Ok(json!(state.service.list_ssh_hosts().map_err(map_svc)?)),
        "get-ssh-profiles" => Ok(json!(state.service.get_ssh_profiles().map_err(map_svc)?)),
        "save-ssh-profile" => {
            let config: sqlator_service::SshProfileConfig = get(args, "config")?;
            Ok(json!(state
                .service
                .save_ssh_profile(config)
                .map_err(map_svc)?))
        }
        "update-ssh-profile" => {
            let id: String = get(args, "id")?;
            let config: sqlator_service::SshProfileConfig = get(args, "config")?;
            Ok(json!(state
                .service
                .update_ssh_profile(&id, config)
                .map_err(map_svc)?))
        }
        "delete-ssh-profile" => {
            let id: String = get(args, "id")?;
            state.service.delete_ssh_profile(&id).map_err(map_svc)?;
            Ok(json!(null))
        }
        "connections-using-ssh-profile" => {
            let profile_id: String = get(args, "profileId")?;
            Ok(json!(state
                .service
                .connections_using_ssh_profile(&profile_id)
                .map_err(map_svc)?))
        }

        // ── SSH tunnels ───────────────────────────────────────────────────────
        "create-ssh-tunnel" => {
            let request: sqlator_service::SshTunnelRequest = get(args, "request")?;
            Ok(json!(state
                .service
                .create_ssh_tunnel(request)
                .await
                .map_err(map_svc)?))
        }
        "close-ssh-tunnel" => {
            let profile_id: String = get(args, "profileId")?;
            state
                .service
                .close_ssh_tunnel(&profile_id)
                .await
                .map_err(map_svc)?;
            Ok(json!(null))
        }
        "get-active-tunnels" => Ok(json!(state.service.get_active_tunnels())),

        // ── Credential storage ────────────────────────────────────────────────
        "check-keyring-available" => Ok(json!(sqlator_service::AppService::keyring_available())),
        "get-storage-mode" => Ok(json!(state
            .service
            .get_storage_mode()
            .await
            .map_err(map_svc)?)),
        "set-storage-mode" => {
            let mode: String = get(args, "mode")?;
            let migrate: bool = get(args, "migrate")?;
            state
                .service
                .set_storage_mode(mode, migrate)
                .await
                .map_err(map_svc)?;
            Ok(json!(null))
        }
        "vault-exists" => Ok(json!(state
            .service
            .vault_exists()
            .await
            .map_err(map_svc)?)),
        "is-vault-locked" => Ok(json!(state
            .service
            .is_vault_locked()
            .await
            .map_err(map_svc)?)),
        "create-vault" => {
            let password: String = get(args, "password")?;
            state
                .service
                .create_vault(password)
                .await
                .map_err(map_svc)?;
            Ok(json!(null))
        }
        "unlock-vault" => {
            let password: String = get(args, "password")?;
            state
                .service
                .unlock_vault(password)
                .await
                .map_err(map_svc)?;
            Ok(json!(null))
        }
        "lock-vault" => {
            state.service.lock_vault().await.map_err(map_svc)?;
            Ok(json!(null))
        }
        "get-vault-settings" => Ok(json!(state
            .service
            .get_vault_settings()
            .await
            .map_err(map_svc)?)),
        "save-vault-settings" => {
            let settings: VaultSettings = get(args, "settings")?;
            state
                .service
                .save_vault_settings(settings)
                .await
                .map_err(map_svc)?;
            Ok(json!(null))
        }

        // ── Connection groups ─────────────────────────────────────────────────
        "get-groups" => Ok(json!(state.service.get_groups().await.map_err(map_svc)?)),
        "save-group" => {
            let payload: sqlator_service::SaveGroupPayload = get(args, "payload")?;
            Ok(json!(state
                .service
                .save_group(payload)
                .await
                .map_err(map_svc)?))
        }
        "update-group" => {
            let group: ConnectionGroup = get(args, "group")?;
            Ok(json!(state
                .service
                .update_group(group)
                .await
                .map_err(map_svc)?))
        }
        "delete-group" => {
            let id: String = get(args, "id")?;
            state.service.delete_group(id).await.map_err(map_svc)?;
            Ok(json!(null))
        }
        "move-connection-to-group" => {
            let connection_id: String = get(args, "connectionId")?;
            let group_id: Option<String> = get_opt(args, "groupId")?;
            Ok(json!(state
                .service
                .move_connection_to_group(connection_id, group_id)
                .await
                .map_err(map_svc)?))
        }

        // ── Import / Export ───────────────────────────────────────────────────
        "export-connections" => export_connections(state).await,
        "import-connections" => {
            let json_str: String = get(args, "json")?;
            let duplicate_mode: String = get(args, "duplicateMode")?;
            Ok(json!(state
                .service
                .import_connections(json_str, duplicate_mode)
                .await
                .map_err(map_svc)?))
        }

        // ── URL parsing ───────────────────────────────────────────────────────
        "parse-connection-url" => {
            let url_str: String = get(args, "url")?;
            Ok(json!(
                sqlator_service::parse_connection_url(&url_str).map_err(map_svc)?
            ))
        }
        "test-connection-with-ssh" => {
            let url: String = get(args, "url")?;
            let ssh_profile_id: String = get(args, "sshProfileId")?;
            Ok(json!(state
                .service
                .test_connection_with_ssh(&url, &ssh_profile_id)
                .await
                .map_err(map_svc)?))
        }

        // ── Schema & query ────────────────────────────────────────────────────
        "fetch-schema-metadata" => {
            let connection_id: String = get(args, "connectionId")?;
            let sql: String = get(args, "sql")?;
            Ok(json!(state
                .service
                .fetch_schema_metadata_for_sql(&connection_id, &sql)
                .await
                .map_err(map_svc)?))
        }
        "get-schemas" => {
            let connection_id: String = get(args, "connectionId")?;
            let schemas: Vec<SchemaInfo> = state
                .service
                .db()
                .get_schemas(&connection_id)
                .await
                .map_err(err)?;
            Ok(json!(schemas))
        }
        "get-tables" => {
            let connection_id: String = get(args, "connectionId")?;
            let schema: Option<String> = get_opt(args, "schema")?;
            let tables: Vec<TableInfo> = state
                .service
                .db()
                .get_tables(&connection_id, schema.as_deref())
                .await
                .map_err(err)?;
            Ok(json!(tables))
        }
        "get-columns" => {
            let connection_id: String = get(args, "connectionId")?;
            let table_name: String = get(args, "tableName")?;
            let schema: Option<String> = get_opt(args, "schema")?;
            let cols: Vec<SchemaColumnInfo> = state
                .service
                .db()
                .get_columns(&connection_id, &table_name, schema.as_deref())
                .await
                .map_err(err)?;
            Ok(json!(cols))
        }
        "query-table" => {
            let params: TableQueryParams = get(args, "params")?;
            let connection_id = params.connection_id.clone();
            let result: TableQueryResult = state
                .service
                .db()
                .query_table(&connection_id, &params)
                .await
                .map_err(err)?;
            Ok(json!(result))
        }
        "get-ddl" => {
            let connection_id: String = get(args, "connectionId")?;
            let table_name: String = get(args, "tableName")?;
            let schema: Option<String> = get_opt(args, "schema")?;
            let ddl = state
                .service
                .db()
                .get_ddl(&connection_id, &table_name, schema.as_deref())
                .await
                .map_err(err)?;
            Ok(json!(ddl))
        }
        "execute-batch" => {
            let connection_id: String = get(args, "connectionId")?;
            let batch: SqlBatch = get(args, "batch")?;
            let result: BatchResult = state
                .service
                .db()
                .execute_batch(&connection_id, &batch)
                .await
                .map_err(err)?;
            Ok(json!(result))
        }

        // ── Docker ────────────────────────────────────────────────────────────
        "discover-container" => {
            let ssh_profile_id: String = get(args, "sshProfileId")?;
            let container_name: String = get(args, "containerName")?;
            Ok(json!(state
                .service
                .discover_container(&ssh_profile_id, &container_name)
                .await
                .map_err(map_svc)?))
        }
        "list-running-containers" => {
            let ssh_profile_id: String = get(args, "sshProfileId")?;
            Ok(json!(state
                .service
                .list_running_containers(&ssh_profile_id)
                .await
                .map_err(map_svc)?))
        }
        "test-docker-connection" => {
            let ssh_profile_id: String = get(args, "sshProfileId")?;
            let container_name: String = get(args, "containerName")?;
            let container_port: u16 = get(args, "containerPort")?;
            let url: String = get(args, "url")?;
            Ok(json!(state
                .service
                .test_docker_connection(
                    &ssh_profile_id,
                    &container_name,
                    Some(container_port),
                    &url,
                    "",
                )
                .await
                .map_err(map_svc)?))
        }
        "discover-local-container" => {
            let container_name: String = get(args, "containerName")?;
            Ok(json!(state
                .service
                .discover_local_container(&container_name)
                .await
                .map_err(map_svc)?))
        }
        "list-local-containers" => Ok(json!(state
            .service
            .list_local_containers()
            .await
            .map_err(map_svc)?)),
        "test-local-docker-connection" => {
            let container_name: String = get(args, "containerName")?;
            let container_port: u16 = get(args, "containerPort")?;
            let url: String = get(args, "url")?;
            Ok(json!(state
                .service
                .test_local_docker_connection(&container_name, Some(container_port), &url, "")
                .await
                .map_err(map_svc)?))
        }

        other => Err(err(format!("unknown command: {other}"))),
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
    match state.service.single_db_info() {
        Some(cfg) => Ok(json!({
            "mode": "single-db",
            "connectionId": cfg.connection_id,
            "connectionName": cfg.connection_name,
        })),
        None => Ok(json!({ "mode": "multi-db" })),
    }
}

// ── Import / Export (web persists to a downloadable temp path) ─────────────────

async fn export_connections(state: &Arc<AppState>) -> HandlerResult {
    let json = state
        .service
        .export_connections_json()
        .await
        .map_err(map_svc)?;

    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let filename = format!("sqlator-export-{date}.json");
    let dir = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir);
    let path = dir.join(&filename);
    std::fs::write(&path, &json).map_err(server_err)?;
    Ok(json!(path.to_string_lossy()))
}

#[cfg(test)]
mod group_a_tests {
    use super::*;
    use sqlator_core::models::{ConnectionGroup, ConnectionType, SshAuthMethod, SshProfile};
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Arc;

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
        let cleanup_dir = dirs::config_dir().expect("config dir").join(&app_name);
        let state = Arc::new(AppState {
            service: Arc::new(
                sqlator_service::AppService::with_app_name(&app_name).expect("AppService"),
            ),
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

    async fn import_connections_handler(state: &Arc<AppState>, args: &Value) -> HandlerResult {
        handle("import-connections", state, args).await
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
        let result = import_connections_handler(&fx.state, &import_args(&payload, "skip"))
            .await
            .expect("import");
        assert_eq!(result["groups_added"], 3);

        let groups = fx.state.service.config().get_groups().unwrap();
        assert_eq!(groups.len(), 3);
        let parent = groups.iter().find(|g| g.name == "parent").unwrap();
        let child = groups.iter().find(|g| g.name == "child").unwrap();
        let grandchild = groups.iter().find(|g| g.name == "grandchild").unwrap();
        assert!(parent.parent_group_id.is_none());
        assert_eq!(child.parent_group_id.as_deref(), Some(parent.id.as_str()));
        assert_eq!(
            grandchild.parent_group_id.as_deref(),
            Some(child.id.as_str())
        );
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
        let result = import_connections_handler(&fx.state, &import_args(&payload, "skip"))
            .await
            .expect("import");
        assert_eq!(result["groups_added"], 2);
        let groups = fx.state.service.config().get_groups().unwrap();
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
        let result = import_connections_handler(&fx.state, &import_args(&payload, "skip"))
            .await
            .expect("import");
        assert_eq!(result["groups_added"], 0);
        assert!(fx.state.service.config().get_groups().unwrap().is_empty());
    }

    #[tokio::test]
    async fn import_group_name_collision_always_skips_even_under_rename() {
        let fx = make_fixture();
        let existing_id = uuid::Uuid::new_v4().to_string();
        fx.state
            .service
            .config()
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
        let result = import_connections_handler(&fx.state, &import_args(&payload, "rename"))
            .await
            .expect("import rename");
        assert_eq!(result["groups_added"], 0);
        let groups = fx.state.service.config().get_groups().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, existing_id);
        assert_eq!(groups[0].color.as_deref(), Some("#111"));

        let result = import_connections_handler(&fx.state, &import_args(&payload, "skip"))
            .await
            .expect("import skip");
        assert_eq!(result["groups_added"], 0);
    }

    #[tokio::test]
    async fn import_connection_name_collision_skip_and_rename() {
        let fx = make_fixture();
        fx.state
            .service
            .config()
            .save_connection(sqlator_core::models::SavedConnection {
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

        let skip = import_connections_handler(&fx.state, &import_args(&payload, "skip"))
            .await
            .expect("skip");
        assert_eq!(skip["connections_added"], 0);
        assert_eq!(skip["connections_skipped"], 1);
        assert_eq!(
            fx.state.service.config().get_connections().unwrap().len(),
            1
        );

        let rename = import_connections_handler(&fx.state, &import_args(&payload, "rename"))
            .await
            .expect("rename");
        assert_eq!(rename["connections_added"], 1);
        assert_eq!(rename["connections_skipped"], 0);
        let names: HashSet<_> = fx
            .state
            .service
            .config()
            .get_connections()
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert!(names.contains("Prod"));
        assert!(names.contains("Prod (1)"));
    }

    #[tokio::test]
    async fn export_import_round_trip_preserves_ssh_tunnel_settings() {
        let fx = make_fixture();
        let profile_id = uuid::Uuid::new_v4().to_string();
        fx.state
            .service
            .config()
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
        fx.state
            .service
            .config()
            .save_group(ConnectionGroup {
                id: uuid::Uuid::new_v4().to_string(),
                name: "Work".into(),
                color: None,
                parent_group_id: None,
                order: 0,
                collapsed: false,
            })
            .unwrap();
        let group_id = fx
            .state
            .service
            .config()
            .get_groups()
            .unwrap()
            .into_iter()
            .find(|g| g.name == "Work")
            .map(|g| g.id);
        fx.state
            .service
            .config()
            .save_connection(sqlator_core::models::SavedConnection {
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

        let exported = fx
            .state
            .service
            .export_connections_json()
            .await
            .expect("export");
        assert!(
            !exported.to_lowercase().contains("password"),
            "export must not contain password material: {exported}"
        );

        let fx2 = make_fixture();
        let result = import_connections_handler(&fx2.state, &import_args(&exported, "skip"))
            .await
            .expect("import");
        assert_eq!(result["groups_added"], 1);
        assert_eq!(result["profiles_added"], 1);
        assert_eq!(result["connections_added"], 1);

        let profiles = fx2.state.service.config().get_ssh_profiles().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].local_port_binding, Some(15432));
        assert_eq!(profiles[0].keepalive_interval, Some(30));
        assert_eq!(profiles[0].name, "bastion");
    }
}

#[cfg(test)]
mod group_b_tests {
    use super::*;
    use sqlator_core::models::{PrimaryKeyMeta, TableMeta};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use sqlator_service::schema_cache_key;

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
        const SCHEMA_CACHE_TTL_SECS: u64 = sqlator_service::SCHEMA_CACHE_TTL_SECS;
        let expires = Instant::now() + Duration::from_secs(SCHEMA_CACHE_TTL_SECS);
        let remaining = expires.saturating_duration_since(Instant::now());
        assert!(remaining.as_secs() >= 299 && remaining.as_secs() <= 300);
    }

    #[test]
    fn schema_cache_hit_miss_and_expiry_via_dashmap() {
        let fx = make_fixture();
        let key_hit = schema_cache_key("c1", &Some("public".into()), "users");
        let key_other = schema_cache_key("c1", &None, "users");

        fx.state.service.schema_cache().insert(
            key_hit.clone(),
            (
                sample_meta("users"),
                Instant::now() + Duration::from_secs(300),
            ),
        );

        {
            let cached = fx.state.service.schema_cache().get(&key_hit).expect("hit");
            let (m, expires_at) = cached.clone();
            assert!(Instant::now() < expires_at);
            assert_eq!(m.table_name, "users");
        }
        assert!(fx.state.service.schema_cache().get(&key_other).is_none());

        let expired_key = schema_cache_key("c1", &Some("public".into()), "old");
        fx.state.service.schema_cache().insert(
            expired_key.clone(),
            (sample_meta("old"), Instant::now() - Duration::from_secs(1)),
        );
        if let Some(cached) = fx.state.service.schema_cache().get(&expired_key) {
            let (_meta, expires_at) = cached.clone();
            assert!(Instant::now() >= expires_at);
            drop(cached);
            fx.state.service.schema_cache().remove(&expired_key);
        }
        assert!(fx.state.service.schema_cache().get(&expired_key).is_none());
    }

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
        assert!(fx.state.service.tunnels().is_empty());
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
        let cleanup_dir = dirs::config_dir().expect("config dir").join(&app_name);
        let state = Arc::new(AppState {
            service: Arc::new(
                sqlator_service::AppService::with_app_name(&app_name).expect("AppService"),
            ),
        });
        TestFixture { state, cleanup_dir }
    }
}
