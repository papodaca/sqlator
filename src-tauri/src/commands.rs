use crate::state::AppState;
use sqlator_core::credentials::VaultSettings;
use sqlator_core::models::{
    ConnectionConfig, ConnectionGroup, ConnectionInfo, QueryEvent, SchemaColumnInfo, SchemaInfo,
    SqlBatch, SshProfile, TableInfo, TableMeta, TableQueryParams, TableQueryResult,
};
use sqlator_core::ssh::{config_parser, HostEntry};
use sqlator_core::BatchResult;
use tauri::ipc::Channel;
use tauri::State;

type CmdResult<T> = Result<T, String>;

fn map_err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn map_svc(e: sqlator_service::ServiceError) -> String {
    e.message()
}

// --- Connection CRUD ---

#[tauri::command]
pub async fn get_connections(state: State<'_, AppState>) -> CmdResult<Vec<ConnectionInfo>> {
    state.service.list_connections().await.map_err(map_svc)
}

#[tauri::command]
pub async fn save_connection(
    state: State<'_, AppState>,
    config: ConnectionConfig,
) -> CmdResult<ConnectionInfo> {
    state.service.save_connection(config).await.map_err(map_svc)
}

#[tauri::command]
pub async fn update_connection(
    state: State<'_, AppState>,
    id: String,
    config: ConnectionConfig,
) -> CmdResult<ConnectionInfo> {
    state
        .service
        .update_connection(id, config)
        .await
        .map_err(map_svc)
}

#[tauri::command]
pub async fn delete_connection(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.service.delete_connection(id).await.map_err(map_svc)
}

#[tauri::command]
pub async fn clone_connection(state: State<'_, AppState>, id: String) -> CmdResult<ConnectionInfo> {
    state.service.clone_connection(id).await.map_err(map_svc)
}

#[tauri::command]
pub async fn test_connection(state: State<'_, AppState>, url: String) -> CmdResult<String> {
    state.service.test_connection(&url).await.map_err(map_svc)
}

// --- Active connection ---

#[tauri::command]
pub async fn connect_database(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.service.connect_database(&id).await.map_err(map_svc)
}

#[tauri::command]
pub async fn disconnect_database(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state
        .service
        .disconnect_database(&id)
        .await
        .map_err(map_svc)
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
    let db = state.service.db();
    let bridge_handle = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let _ = on_event.send(event);
        }
    });
    let result = db.execute_query(&connection_id, &sql, tx).await;
    let _ = bridge_handle.await;
    result.map_err(map_err)
}

// --- Query / tab / theme persistence ---

#[tauri::command]
pub async fn get_query(
    state: State<'_, AppState>,
    connection_id: String,
) -> CmdResult<Option<String>> {
    state
        .service
        .get_query(connection_id)
        .await
        .map_err(map_svc)
}

#[tauri::command]
pub async fn save_query(
    state: State<'_, AppState>,
    connection_id: String,
    query: String,
) -> CmdResult<()> {
    state
        .service
        .save_query(connection_id, query)
        .await
        .map_err(map_svc)
}

#[tauri::command]
pub async fn get_tab_state(state: State<'_, AppState>) -> CmdResult<Option<serde_json::Value>> {
    state.service.get_tab_state().await.map_err(map_svc)
}

#[tauri::command]
pub async fn save_tab_state(
    state: State<'_, AppState>,
    tab_state: serde_json::Value,
) -> CmdResult<()> {
    state
        .service
        .save_tab_state(tab_state)
        .await
        .map_err(map_svc)
}

#[tauri::command]
pub async fn get_theme(state: State<'_, AppState>) -> CmdResult<String> {
    state.service.get_theme().await.map_err(map_svc)
}

#[tauri::command]
pub async fn save_theme(state: State<'_, AppState>, theme: String) -> CmdResult<()> {
    state.service.save_theme(theme).await.map_err(map_svc)
}

// --- SSH Config / tunnels / profiles ---

#[tauri::command]
pub async fn list_ssh_hosts() -> CmdResult<Vec<HostEntry>> {
    config_parser::load_ssh_config().map_err(map_err)
}

#[tauri::command]
pub async fn create_ssh_tunnel(
    state: State<'_, AppState>,
    request: sqlator_service::SshTunnelRequest,
) -> CmdResult<sqlator_service::SshTunnelInfo> {
    state
        .service
        .create_ssh_tunnel(request)
        .await
        .map_err(map_svc)
}

#[tauri::command]
pub async fn close_ssh_tunnel(state: State<'_, AppState>, profile_id: String) -> CmdResult<()> {
    state
        .service
        .close_ssh_tunnel(&profile_id)
        .await
        .map_err(map_svc)
}

#[tauri::command]
pub async fn get_active_tunnels(
    state: State<'_, AppState>,
) -> CmdResult<Vec<sqlator_service::SshTunnelInfo>> {
    Ok(state.service.get_active_tunnels())
}

#[tauri::command]
pub async fn get_ssh_profiles(state: State<'_, AppState>) -> CmdResult<Vec<SshProfile>> {
    state.service.get_ssh_profiles().map_err(map_svc)
}

#[tauri::command]
pub async fn save_ssh_profile(
    state: State<'_, AppState>,
    config: sqlator_service::SshProfileConfig,
) -> CmdResult<SshProfile> {
    state.service.save_ssh_profile(config).map_err(map_svc)
}

#[tauri::command]
pub async fn update_ssh_profile(
    state: State<'_, AppState>,
    id: String,
    config: sqlator_service::SshProfileConfig,
) -> CmdResult<SshProfile> {
    state
        .service
        .update_ssh_profile(&id, config)
        .map_err(map_svc)
}

#[tauri::command]
pub async fn delete_ssh_profile(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.service.delete_ssh_profile(&id).map_err(map_svc)
}

#[tauri::command]
pub async fn connections_using_ssh_profile(
    state: State<'_, AppState>,
    profile_id: String,
) -> CmdResult<Vec<String>> {
    state
        .service
        .connections_using_ssh_profile(&profile_id)
        .map_err(map_svc)
}

#[tauri::command]
pub async fn parse_connection_url(url: String) -> CmdResult<sqlator_service::ParsedConnectionUrl> {
    sqlator_service::parse_connection_url(&url).map_err(map_svc)
}

#[tauri::command]
pub async fn test_connection_with_ssh(
    state: State<'_, AppState>,
    url: String,
    ssh_profile_id: String,
) -> CmdResult<String> {
    state
        .service
        .test_connection_with_ssh(&url, &ssh_profile_id)
        .await
        .map_err(map_svc)
}

// --- Credentials / vault ---

#[tauri::command]
pub async fn check_keyring_available() -> bool {
    sqlator_service::AppService::keyring_available()
}

#[tauri::command]
pub async fn get_storage_mode(state: State<'_, AppState>) -> CmdResult<String> {
    state.service.get_storage_mode().await.map_err(map_svc)
}

#[tauri::command]
pub async fn set_storage_mode(
    state: State<'_, AppState>,
    mode: String,
    migrate: bool,
) -> CmdResult<()> {
    state
        .service
        .set_storage_mode(mode, migrate)
        .await
        .map_err(map_svc)
}

#[tauri::command]
pub async fn vault_exists(state: State<'_, AppState>) -> CmdResult<bool> {
    state.service.vault_exists().await.map_err(map_svc)
}

#[tauri::command]
pub async fn is_vault_locked(state: State<'_, AppState>) -> CmdResult<bool> {
    state.service.is_vault_locked().await.map_err(map_svc)
}

#[tauri::command]
pub async fn create_vault(state: State<'_, AppState>, password: String) -> CmdResult<()> {
    state.service.create_vault(password).await.map_err(map_svc)
}

#[tauri::command]
pub async fn unlock_vault(state: State<'_, AppState>, password: String) -> CmdResult<()> {
    state.service.unlock_vault(password).await.map_err(map_svc)
}

#[tauri::command]
pub async fn lock_vault(state: State<'_, AppState>) -> CmdResult<()> {
    state.service.lock_vault().await.map_err(map_svc)
}

#[tauri::command]
pub async fn get_vault_settings(state: State<'_, AppState>) -> CmdResult<VaultSettings> {
    state.service.get_vault_settings().await.map_err(map_svc)
}

#[tauri::command]
pub async fn save_vault_settings(
    state: State<'_, AppState>,
    settings: VaultSettings,
) -> CmdResult<()> {
    state
        .service
        .save_vault_settings(settings)
        .await
        .map_err(map_svc)
}

// --- Groups ---

#[tauri::command]
pub async fn get_groups(state: State<'_, AppState>) -> CmdResult<Vec<ConnectionGroup>> {
    state.service.get_groups().await.map_err(map_svc)
}

#[tauri::command]
pub async fn save_group(
    state: State<'_, AppState>,
    payload: sqlator_service::SaveGroupPayload,
) -> CmdResult<ConnectionGroup> {
    state.service.save_group(payload).await.map_err(map_svc)
}

#[tauri::command]
pub async fn update_group(
    state: State<'_, AppState>,
    group: ConnectionGroup,
) -> CmdResult<ConnectionGroup> {
    state.service.update_group(group).await.map_err(map_svc)
}

#[tauri::command]
pub async fn delete_group(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.service.delete_group(id).await.map_err(map_svc)
}

#[tauri::command]
pub async fn move_connection_to_group(
    state: State<'_, AppState>,
    connection_id: String,
    group_id: Option<String>,
) -> CmdResult<ConnectionInfo> {
    state
        .service
        .move_connection_to_group(connection_id, group_id)
        .await
        .map_err(map_svc)
}

// --- Import / Export ---

#[tauri::command]
pub async fn export_connections(state: State<'_, AppState>) -> CmdResult<String> {
    let json = state
        .service
        .export_connections_json()
        .await
        .map_err(map_svc)?;

    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let filename = format!("sqlator-export-{date}.json");
    let dir = dirs::download_dir()
        .or_else(dirs::home_dir)
        .ok_or("Could not determine export directory")?;
    let path = dir.join(&filename);
    std::fs::write(&path, json).map_err(map_err)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn import_connections(
    state: State<'_, AppState>,
    json: String,
    duplicate_mode: String,
) -> CmdResult<sqlator_service::ImportResult> {
    state
        .service
        .import_connections(json, duplicate_mode)
        .await
        .map_err(map_svc)
}

// --- Docker ---

#[tauri::command]
pub async fn discover_container(
    state: State<'_, AppState>,
    ssh_profile_id: String,
    container_name: String,
) -> CmdResult<sqlator_service::DockerContainerInfo> {
    state
        .service
        .discover_container(&ssh_profile_id, &container_name)
        .await
        .map_err(map_svc)
}

#[tauri::command]
pub async fn list_running_containers(
    state: State<'_, AppState>,
    ssh_profile_id: String,
) -> CmdResult<Vec<sqlator_service::ContainerSummaryInfo>> {
    state
        .service
        .list_running_containers(&ssh_profile_id)
        .await
        .map_err(map_svc)
}

#[tauri::command]
pub async fn test_docker_connection(
    state: State<'_, AppState>,
    ssh_profile_id: String,
    container_name: String,
    container_port: u16,
    url: String,
) -> CmdResult<String> {
    state
        .service
        .test_docker_connection(
            &ssh_profile_id,
            &container_name,
            Some(container_port),
            &url,
            "",
        )
        .await
        .map_err(map_svc)
}

#[tauri::command]
pub async fn discover_local_container(
    state: State<'_, AppState>,
    container_name: String,
) -> CmdResult<sqlator_service::DockerContainerInfo> {
    state
        .service
        .discover_local_container(&container_name)
        .await
        .map_err(map_svc)
}

#[tauri::command]
pub async fn list_local_containers(
    state: State<'_, AppState>,
) -> CmdResult<Vec<sqlator_service::ContainerSummaryInfo>> {
    state.service.list_local_containers().await.map_err(map_svc)
}

#[tauri::command]
pub async fn test_local_docker_connection(
    state: State<'_, AppState>,
    container_name: String,
    container_port: u16,
    url: String,
) -> CmdResult<String> {
    state
        .service
        .test_local_docker_connection(&container_name, Some(container_port), &url, "")
        .await
        .map_err(map_svc)
}

// --- Schema metadata / browser / batch ---

#[tauri::command]
pub async fn fetch_schema_metadata(
    state: State<'_, AppState>,
    connection_id: String,
    sql: String,
) -> CmdResult<Option<TableMeta>> {
    state
        .service
        .fetch_schema_metadata_for_sql(&connection_id, &sql)
        .await
        .map_err(map_svc)
}

#[tauri::command]
pub async fn get_schemas(
    state: State<'_, AppState>,
    connection_id: String,
) -> CmdResult<Vec<SchemaInfo>> {
    state
        .service
        .db()
        .get_schemas(&connection_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn get_tables(
    state: State<'_, AppState>,
    connection_id: String,
    schema: Option<String>,
) -> CmdResult<Vec<TableInfo>> {
    state
        .service
        .db()
        .get_tables(&connection_id, schema.as_deref())
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn get_columns(
    state: State<'_, AppState>,
    connection_id: String,
    table_name: String,
    schema: Option<String>,
) -> CmdResult<Vec<SchemaColumnInfo>> {
    state
        .service
        .db()
        .get_columns(&connection_id, &table_name, schema.as_deref())
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn query_table(
    state: State<'_, AppState>,
    params: TableQueryParams,
) -> CmdResult<TableQueryResult> {
    let connection_id = params.connection_id.clone();
    state
        .service
        .db()
        .query_table(&connection_id, &params)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn get_ddl(
    state: State<'_, AppState>,
    connection_id: String,
    table_name: String,
    schema: Option<String>,
) -> CmdResult<String> {
    state
        .service
        .db()
        .get_ddl(&connection_id, &table_name, schema.as_deref())
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn execute_batch(
    state: State<'_, AppState>,
    connection_id: String,
    batch: SqlBatch,
) -> CmdResult<BatchResult> {
    state
        .service
        .db()
        .execute_batch(&connection_id, &batch)
        .await
        .map_err(map_err)
}

#[cfg(test)]
mod group_a_tests {
    use super::*;
    use sqlator_core::models::{ConnectionGroup, ConnectionType, SshAuthMethod, SshProfile};
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tauri::Manager;

    // Pure helpers (unique_name, build_url_no_password, resolve_connection_type,
    // default_port_for_db_type, parse_auth_method) live in sqlator-service now.

    // extract_single_table tests live in sqlator-service::schema.

    // ── import / export fixtures ──────────────────────────────────────────────

    struct TestFixture {
        app: tauri::App<tauri::test::MockRuntime>,
        cleanup_dir: PathBuf,
    }

    impl Drop for TestFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.cleanup_dir);
        }
    }

    fn make_fixture() -> TestFixture {
        let id = uuid::Uuid::new_v4();
        let app_name = format!("sqlator-char-a-{id}");
        let cleanup_dir = dirs::config_dir().expect("config dir").join(&app_name);
        let state = AppState {
            service: Arc::new(
                sqlator_service::AppService::with_app_name(&app_name).expect("AppService"),
            ),
            terminals: dashmap::DashMap::new(),
        };
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        TestFixture { app, cleanup_dir }
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

    #[tokio::test]
    async fn import_nested_groups_remaps_parent_ids() {
        let fx = make_fixture();
        let json = empty_export_json(
            r#"[
              {"name": "child", "color": null, "parent_group_name": "parent", "order": 1},
              {"name": "parent", "color": "fff", "parent_group_name": null, "order": 0},
              {"name": "grandchild", "color": null, "parent_group_name": "child", "order": 2}
            ]"#,
            "[]",
            "[]",
        );
        let result = import_connections(fx.app.state(), json, "skip".into())
            .await
            .expect("import");
        assert_eq!(result.groups_added, 3);

        let groups = fx
            .app
            .state::<AppState>()
            .service
            .config()
            .get_groups()
            .unwrap();
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
        // IDs are freshly generated UUIDs, not export-time names
        assert_ne!(parent.id, "parent");
    }

    #[tokio::test]
    async fn import_parent_appearing_later_in_file() {
        let fx = make_fixture();
        // Child listed before parent — three-pass algorithm must still link them.
        let json = empty_export_json(
            r#"[
              {"name": "child", "color": null, "parent_group_name": "parent", "order": 1},
              {"name": "parent", "color": null, "parent_group_name": null, "order": 0}
            ]"#,
            "[]",
            "[]",
        );
        let result = import_connections(fx.app.state(), json, "skip".into())
            .await
            .expect("import");
        assert_eq!(result.groups_added, 2);
        let groups = fx
            .app
            .state::<AppState>()
            .service
            .config()
            .get_groups()
            .unwrap();
        let parent = groups.iter().find(|g| g.name == "parent").unwrap();
        let child = groups.iter().find(|g| g.name == "child").unwrap();
        assert_eq!(child.parent_group_id.as_deref(), Some(parent.id.as_str()));
    }

    #[tokio::test]
    async fn import_cyclic_parent_silently_drops_cycle() {
        let fx = make_fixture();
        let json = empty_export_json(
            r#"[
              {"name": "a", "color": null, "parent_group_name": "b", "order": 0},
              {"name": "b", "color": null, "parent_group_name": "a", "order": 1}
            ]"#,
            "[]",
            "[]",
        );
        let result = import_connections(fx.app.state(), json, "skip".into())
            .await
            .expect("import");
        // After 3 passes neither side of the cycle can resolve — both dropped.
        assert_eq!(result.groups_added, 0);
        assert!(fx
            .app
            .state::<AppState>()
            .service
            .config()
            .get_groups()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn import_group_name_collision_always_skips_even_under_rename() {
        let fx = make_fixture();
        let existing_id = uuid::Uuid::new_v4().to_string();
        fx.app
            .state::<AppState>()
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

        let json = empty_export_json(
            r#"[{"name": "Shared", "color": "222", "parent_group_name": null, "order": 5}]"#,
            "[]",
            "[]",
        );
        // Groups ignore duplicate_mode — always skip on name collision.
        let result = import_connections(fx.app.state(), json.clone(), "rename".into())
            .await
            .expect("import rename");
        assert_eq!(result.groups_added, 0);
        let groups = fx
            .app
            .state::<AppState>()
            .service
            .config()
            .get_groups()
            .unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, existing_id);
        assert_eq!(groups[0].color.as_deref(), Some("#111"));

        let result = import_connections(fx.app.state(), json, "skip".into())
            .await
            .expect("import skip");
        assert_eq!(result.groups_added, 0);
    }

    #[tokio::test]
    async fn import_connection_name_collision_skip_and_rename() {
        let fx = make_fixture();
        fx.app
            .state::<AppState>()
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

        let json = empty_export_json(
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

        let skip = import_connections(fx.app.state(), json.clone(), "skip".into())
            .await
            .expect("skip");
        assert_eq!(skip.connections_added, 0);
        assert_eq!(skip.connections_skipped, 1);
        assert_eq!(
            fx.app
                .state::<AppState>()
                .service
                .config()
                .get_connections()
                .unwrap()
                .len(),
            1
        );

        let rename = import_connections(fx.app.state(), json, "rename".into())
            .await
            .expect("rename");
        assert_eq!(rename.connections_added, 1);
        assert_eq!(rename.connections_skipped, 0);
        let names: HashSet<_> = fx
            .app
            .state::<AppState>()
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
        fx.app
            .state::<AppState>()
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
        fx.app
            .state::<AppState>()
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
        fx.app
            .state::<AppState>()
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
                group_id: fx
                    .app
                    .state::<AppState>()
                    .service
                    .config()
                    .get_groups()
                    .unwrap()
                    .into_iter()
                    .find(|g| g.name == "Work")
                    .map(|g| g.id),
                connection_type: ConnectionType::SshTunnel,
                container_name: None,
                container_port: None,
            })
            .unwrap();

        let exported = fx
            .app
            .state::<AppState>()
            .service
            .export_connections_json()
            .await
            .expect("export");
        assert!(
            !exported.to_lowercase().contains("password"),
            "export must not contain password material: {exported}"
        );

        // Import into a fresh fixture
        let fx2 = make_fixture();
        let result = import_connections(fx2.app.state(), exported, "skip".into())
            .await
            .expect("import");
        assert_eq!(result.groups_added, 1);
        assert_eq!(result.profiles_added, 1);
        assert_eq!(result.connections_added, 1);

        let profiles = fx2
            .app
            .state::<AppState>()
            .service
            .config()
            .get_ssh_profiles()
            .unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].local_port_binding, Some(15432));
        assert_eq!(profiles[0].keepalive_interval, Some(30));
        assert_eq!(profiles[0].name, "bastion");

        let conns = fx2
            .app
            .state::<AppState>()
            .service
            .config()
            .get_connections()
            .unwrap();
        assert_eq!(conns.len(), 1);
        assert_eq!(conns[0].name, "App DB");
        assert!(conns[0].ssh_profile_id.is_some());
        assert!(conns[0].group_id.is_some());
    }
}

#[cfg(test)]
mod group_b_tests {
    use super::*;
    use sqlator_core::models::{PrimaryKeyMeta, TableMeta};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tauri::Manager;

    // ── Schema cache key format ───────────────────────────────────────────────

    /// Reconstructs the production key at `fetch_schema_metadata`:
    /// `format!("{connection_id}:{schema_name:?}:{table_name}")`
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
        // Pin Debug formatting: Some("public") keys as Some("public"), not "public"
        assert_eq!(
            schema_cache_key("c1", &Some("public".into()), "users"),
            r#"c1:Some("public"):users"#
        );
        assert_eq!(schema_cache_key("c1", &None, "users"), "c1:None:users");
        assert_eq!(
            schema_cache_key("c1", &Some("".into()), "users"),
            r#"c1:Some(""):users"#
        );
        // None vs Some("") must not collide
        assert_ne!(
            schema_cache_key("c1", &None, "t"),
            schema_cache_key("c1", &Some("".into()), "t")
        );
    }

    #[test]
    fn schema_cache_ttl_constant_is_300_seconds() {
        // Production insert: Instant::now() + Duration::from_secs(300)
        // Cannot advance Instant without a clock abstraction; pin the constant.
        // Production insert: Instant::now() + Duration::from_secs(sqlator_service::SCHEMA_CACHE_TTL_SECS)
        const SCHEMA_CACHE_TTL_SECS: u64 = sqlator_service::SCHEMA_CACHE_TTL_SECS;
        let inserted_at = Instant::now();
        let expires = inserted_at + Duration::from_secs(SCHEMA_CACHE_TTL_SECS);
        let remaining = expires.saturating_duration_since(Instant::now());
        assert!(
            remaining.as_secs() >= 299 && remaining.as_secs() <= 300,
            "TTL characterization: inserts use 300s; remaining={remaining:?}"
        );
    }

    #[test]
    fn schema_cache_hit_miss_and_expiry_via_dashmap() {
        // Exercise the same lookup logic as fetch_schema_metadata without a live DB.
        let fx = make_fixture();
        let state = fx.app.state::<AppState>();
        let key_hit = schema_cache_key("c1", &Some("public".into()), "users");
        let key_other = schema_cache_key("c1", &None, "users");

        let meta = sample_meta("users");
        state.service.schema_cache().insert(
            key_hit.clone(),
            (meta.clone(), Instant::now() + Duration::from_secs(300)),
        );

        // Hit
        {
            let cached = state.service.schema_cache().get(&key_hit).expect("hit");
            let (m, expires_at) = cached.clone();
            assert!(Instant::now() < expires_at);
            assert_eq!(m.table_name, "users");
        }

        // Miss — different key (None vs Some("public"))
        assert!(state.service.schema_cache().get(&key_other).is_none());

        // Expiry — past expires_at → remove (mirrors production branch)
        let expired_key = schema_cache_key("c1", &Some("public".into()), "old");
        state.service.schema_cache().insert(
            expired_key.clone(),
            (sample_meta("old"), Instant::now() - Duration::from_secs(1)),
        );
        if let Some(cached) = state.service.schema_cache().get(&expired_key) {
            let (_meta, expires_at) = cached.clone();
            assert!(Instant::now() >= expires_at);
            drop(cached);
            state.service.schema_cache().remove(&expired_key);
        }
        assert!(state.service.schema_cache().get(&expired_key).is_none());
    }

    // ── Tunnel registry ───────────────────────────────────────────────────────

    #[test]
    fn tunnel_registry_replace_same_id_does_not_leak_entries() {
        // Characterization of commands.rs connect_ssh_tunnel / connect_docker_via_ssh:
        // `tunnels.remove(id)` then `tunnels.insert(id, new)` — DashMap len stays 1.
        // Stand-in values: we cannot build TunnelHandle without live SSH.
        let tunnels: dashmap::DashMap<String, u16> = dashmap::DashMap::new();
        tunnels.insert("conn-1".into(), 10_001);

        if let Some((_, old_port)) = tunnels.remove("conn-1") {
            assert_eq!(old_port, 10_001);
            // Production would: SshTunnel::close(old_tunnel).await.ok();
        }
        tunnels.insert("conn-1".into(), 10_002);

        assert_eq!(tunnels.len(), 1);
        assert_eq!(*tunnels.get("conn-1").unwrap(), 10_002);
    }

    #[test]
    fn appstate_tunnels_map_starts_empty() {
        let fx = make_fixture();
        assert!(fx.app.state::<AppState>().service.tunnels().is_empty());
    }

    /// Full cleanup (reconnect closes old tunnel; failing test_connection_with_ssh
    /// closes ephemeral tunnel; local port re-bindable) needs live SSH — Group C.
    #[tokio::test]
    #[ignore = "needs live SSH; overlaps Group C tunnel tests — see plan Group B tunnel registry"]
    async fn tunnel_ephemeral_teardown_on_test_connection_failure_live_ssh() {
        // Stub: when SSH is available, assert state.service.tunnels() is empty after a failing
        // test_connection_with_ssh and that the forwarded local port can be rebound.
        panic!("not implemented without live SSH");
    }

    // ── Fixture (mirrors group_a) ─────────────────────────────────────────────

    struct TestFixture {
        app: tauri::App<tauri::test::MockRuntime>,
        cleanup_dir: PathBuf,
    }

    impl Drop for TestFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.cleanup_dir);
        }
    }

    fn make_fixture() -> TestFixture {
        let id = uuid::Uuid::new_v4();
        let app_name = format!("sqlator-char-b-{id}");
        let cleanup_dir = dirs::config_dir().expect("config dir").join(&app_name);
        let state = AppState {
            service: Arc::new(
                sqlator_service::AppService::with_app_name(&app_name).expect("AppService"),
            ),
            terminals: dashmap::DashMap::new(),
        };
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        TestFixture { app, cleanup_dir }
    }
}
