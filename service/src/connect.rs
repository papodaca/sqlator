//! Connect / disconnect orchestration across direct, SSH, and Docker paths.
//!
//! One authenticated SSH session per profile; local forwards keyed by
//! [`crate::tunnels::TunnelKey`] `(profile, host, port)` with refcounting.
//! Ephemeral `test_*` tunnels are never inserted into the registry.

use crate::connections::default_port_for_db_type;
use crate::error::ServiceError;
use crate::service::AppService;
use crate::ssh::{build_auth_config_for_profile, build_jump_hosts_for_profile};
use crate::tunnels::{
    rewrite_url_via_localhost, target_from_db_url, ManagedForward, ManagedSession, TunnelClaim,
    TunnelKey,
};
use sqlator_core::docker::inspector::ContainerInspector;
use sqlator_core::docker::{ContainerStatus, LocalDockerAccess};
use sqlator_core::models::{ConnectionType, SavedConnection};
use sqlator_core::ssh::{SshHostConfig, SshSession, SshTunnel};
use tracing::{debug, info, warn};

impl AppService {
    /// Connect a saved connection (direct / SSH tunnel / remote Docker / local Docker).
    pub async fn connect_database(&self, connection_id: &str) -> Result<(), ServiceError> {
        // Single-db mode: pool is pre-connected at startup.
        if let Some(info) = self.single_db_info() {
            if connection_id == info.connection_id {
                return Ok(());
            }
            return Err(ServiceError::app(
                "SINGLE_DB_ONLY",
                "cannot connect to other databases in single-db mode",
            ));
        }

        let conn = self.find_saved_connection(connection_id)?;

        let url = match conn.connection_type {
            ConnectionType::DockerContainer => {
                self.connect_docker_container(connection_id, &conn).await?
            }
            ConnectionType::SshTunnel | ConnectionType::Direct if conn.ssh_profile_id.is_some() => {
                self.connect_ssh_tunnel(connection_id, &conn).await?
            }
            ConnectionType::LocalDockerContainer => {
                self.connect_local_docker(connection_id, &conn).await?
            }
            _ => {
                debug!(
                    "connect_database '{}': no SSH profile, direct connection",
                    connection_id
                );
                conn.url.clone()
            }
        };

        self.db.connect(connection_id, &url).await?;
        Ok(())
    }

    /// Disconnect pool and release this connection's tunnel claim (refcount teardown).
    pub async fn disconnect_database(&self, connection_id: &str) -> Result<(), ServiceError> {
        self.db.disconnect(connection_id).await;
        self.release_connection_tunnel(connection_id).await?;
        Ok(())
    }

    /// Public lookup for terminal / VTE: connection → shared tunnel local port.
    pub fn tunnel_local_port_for_connection(&self, connection_id: &str) -> Option<u16> {
        let key = self.connection_tunnels.get(connection_id)?;
        self.tunnels
            .get(key.value())
            .map(|entry| entry.forward.local_port)
    }

    /// Test a DB URL through an ephemeral SSH tunnel (always closed afterwards).
    pub async fn test_connection_with_ssh(
        &self,
        url: &str,
        ssh_profile_id: &str,
    ) -> Result<String, ServiceError> {
        let profile = self
            .config
            .get_ssh_profile(ssh_profile_id)?
            .ok_or_else(|| {
                ServiceError::app(
                    "SSH_PROFILE_NOT_FOUND",
                    format!("SSH profile '{ssh_profile_id}' not found"),
                )
            })?;

        let (target_host, target_port) = target_from_db_url(url)?;
        let auth_config = build_auth_config_for_profile(&profile, &self.credentials)?;
        let ssh_config = SshHostConfig::new(&profile.host, profile.port, &auth_config);
        let jump_hosts = build_jump_hosts_for_profile(&profile)?;
        let tunnel_id = format!("test-{}", uuid::Uuid::new_v4());

        let tunnel = SshTunnel::create(
            tunnel_id,
            &ssh_config,
            &auth_config,
            target_host,
            target_port,
            &jump_hosts,
        )
        .await?;

        SshTunnel::start_forwarding(&tunnel).await?;
        let local_port = tunnel.local_port;
        let test_url = rewrite_url_via_localhost(url, local_port)?;

        let result = sqlator_core::db::DbManager::test_connection(&test_url).await;
        SshTunnel::close(tunnel).await.ok();
        Ok(result?)
    }

    /// Test via remote Docker inspect + ephemeral tunnel (always closed afterwards).
    pub async fn test_docker_connection(
        &self,
        ssh_profile_id: &str,
        container_name: &str,
        container_port: Option<u16>,
        url: &str,
        db_type: &str,
    ) -> Result<String, ServiceError> {
        let profile = self
            .config
            .get_ssh_profile(ssh_profile_id)?
            .ok_or_else(|| {
                ServiceError::app(
                    "SSH_PROFILE_NOT_FOUND",
                    format!("SSH profile '{ssh_profile_id}' not found"),
                )
            })?;

        let auth_config = build_auth_config_for_profile(&profile, &self.credentials)?;
        let ssh_config = SshHostConfig::new(&profile.host, profile.port, &auth_config);
        let jump_hosts = build_jump_hosts_for_profile(&profile)?;

        let container_info =
            ContainerInspector::inspect(&ssh_config, &auth_config, &jump_hosts, container_name)
                .await?;

        if container_info.status != ContainerStatus::Running {
            return Err(ServiceError::app(
                "DOCKER_CONTAINER_STOPPED",
                format!("Container '{container_name}' is not running"),
            ));
        }

        let port = container_port.unwrap_or_else(|| default_port_for_db_type(db_type));
        let tunnel_id = format!("test-docker-{}", uuid::Uuid::new_v4());
        let tunnel = SshTunnel::create(
            tunnel_id,
            &ssh_config,
            &auth_config,
            container_info.ip_address.clone(),
            port,
            &jump_hosts,
        )
        .await?;

        SshTunnel::start_forwarding(&tunnel).await?;
        let test_url = rewrite_url_via_localhost(url, tunnel.local_port)?;
        let result = sqlator_core::db::DbManager::test_connection(&test_url).await;
        SshTunnel::close(tunnel).await.ok();
        Ok(result?)
    }

    /// Test a local Docker container (no SSH tunnel).
    pub async fn test_local_docker_connection(
        &self,
        container_name: &str,
        container_port: Option<u16>,
        url: &str,
        db_type: &str,
    ) -> Result<String, ServiceError> {
        let local_docker = LocalDockerAccess::new()?;
        let container_info = local_docker.inspect(container_name).await?;
        if container_info.status != ContainerStatus::Running {
            return Err(ServiceError::app(
                "DOCKER_CONTAINER_STOPPED",
                format!("Container '{container_name}' is not running"),
            ));
        }
        let port = container_port.unwrap_or_else(|| default_port_for_db_type(db_type));
        let parsed = url::Url::parse(url)
            .map_err(|e| ServiceError::app("INVALID_URL", format!("Invalid database URL: {e}")))?;
        let mut docker_url = parsed;
        let _ = docker_url.set_host(Some(&container_info.ip_address));
        let _ = docker_url.set_port(Some(port));
        Ok(sqlator_core::db::DbManager::test_connection(docker_url.as_str()).await?)
    }

    /// Test a database URL directly (no SSH / Docker).
    pub async fn test_connection(&self, url: &str) -> Result<String, ServiceError> {
        Ok(sqlator_core::db::DbManager::test_connection(url).await?)
    }

    async fn connect_ssh_tunnel(
        &self,
        connection_id: &str,
        conn: &SavedConnection,
    ) -> Result<String, ServiceError> {
        let ssh_profile_id = conn.ssh_profile_id.as_ref().ok_or_else(|| {
            ServiceError::app(
                "SSH_PROFILE_REQUIRED",
                "SSH tunnel connection requires an SSH profile",
            )
        })?;

        info!(
            "connect_database '{}': SSH tunnel via profile '{}'",
            connection_id, ssh_profile_id
        );

        let (target_host, target_port) = target_from_db_url(&conn.url)?;
        let local_port = self
            .acquire_tunnel(connection_id, ssh_profile_id, &target_host, target_port)
            .await?;

        rewrite_url_via_localhost(&conn.url, local_port)
    }

    async fn connect_docker_container(
        &self,
        connection_id: &str,
        conn: &SavedConnection,
    ) -> Result<String, ServiceError> {
        let ssh_profile_id = conn.ssh_profile_id.as_ref().ok_or_else(|| {
            ServiceError::app(
                "SSH_PROFILE_REQUIRED",
                "DockerContainer connection requires an SSH profile",
            )
        })?;
        let container_name = conn.container_name.as_ref().ok_or_else(|| {
            ServiceError::app(
                "CONTAINER_NAME_REQUIRED",
                "DockerContainer connection requires a container name",
            )
        })?;

        info!(
            "connect_database '{}': Docker container, tunnel via SSH profile '{}'",
            connection_id, ssh_profile_id
        );

        let profile = self
            .config
            .get_ssh_profile(ssh_profile_id)?
            .ok_or_else(|| {
                ServiceError::app(
                    "SSH_PROFILE_NOT_FOUND",
                    format!("SSH profile '{ssh_profile_id}' not found"),
                )
            })?;

        let auth_config = build_auth_config_for_profile(&profile, &self.credentials)?;
        let ssh_config = SshHostConfig::new(&profile.host, profile.port, &auth_config);
        let jump_hosts = build_jump_hosts_for_profile(&profile)?;

        let container_info =
            ContainerInspector::inspect(&ssh_config, &auth_config, &jump_hosts, container_name)
                .await?;

        if container_info.status != ContainerStatus::Running {
            return Err(ServiceError::app(
                "DOCKER_CONTAINER_STOPPED",
                format!("Container '{container_name}' is not running"),
            ));
        }

        let container_port = conn
            .container_port
            .unwrap_or_else(|| default_port_for_db_type(&conn.db_type));

        let local_port = self
            .acquire_tunnel(
                connection_id,
                ssh_profile_id,
                &container_info.ip_address,
                container_port,
            )
            .await?;

        info!(
            "connect_database: Docker tunnel ready on localhost:{} -> {}:{}",
            local_port, container_info.ip_address, container_port
        );

        rewrite_url_via_localhost(&conn.url, local_port)
    }

    async fn connect_local_docker(
        &self,
        _connection_id: &str,
        conn: &SavedConnection,
    ) -> Result<String, ServiceError> {
        let container_name = conn.container_name.as_ref().ok_or_else(|| {
            ServiceError::app(
                "CONTAINER_NAME_REQUIRED",
                "LocalDockerContainer connection requires a container name",
            )
        })?;

        let local_docker = LocalDockerAccess::new()?;
        let container_info = local_docker.inspect(container_name).await?;
        if container_info.status != ContainerStatus::Running {
            return Err(ServiceError::app(
                "DOCKER_CONTAINER_STOPPED",
                format!("Container '{container_name}' is not running"),
            ));
        }

        let container_port = conn
            .container_port
            .unwrap_or_else(|| default_port_for_db_type(&conn.db_type));

        let parsed = url::Url::parse(&conn.url)
            .map_err(|e| ServiceError::app("INVALID_URL", format!("Invalid database URL: {e}")))?;
        let mut docker_url = parsed;
        let _ = docker_url.set_host(Some(&container_info.ip_address));
        let _ = docker_url.set_port(Some(container_port));
        Ok(docker_url.to_string())
    }

    /// Acquire or share a forward for `(profile, target)`; claim `connection_id`.
    ///
    /// Reuses the authenticated session for `profile_id` when present. If this
    /// connection already held a different key (stale reconnect), that claim is
    /// released first.
    async fn acquire_tunnel(
        &self,
        connection_id: &str,
        profile_id: &str,
        target_host: &str,
        target_port: u16,
    ) -> Result<u16, ServiceError> {
        let key = TunnelKey::new(profile_id, target_host, target_port);

        // Stale reconnect: drop previous claim for this connection if any.
        if let Some(old) = self.connection_tunnels.get(connection_id) {
            if old.value() != &key {
                drop(old);
                warn!(
                    "connect_database: releasing stale tunnel claim for connection '{}'",
                    connection_id
                );
                self.release_connection_tunnel(connection_id).await?;
            } else {
                // Already claimed this exact forward — reuse.
                let port = self
                    .tunnels
                    .get(&key)
                    .map(|e| e.forward.local_port)
                    .ok_or_else(|| {
                        ServiceError::app(
                            "TUNNEL_INVARIANT",
                            "connection_tunnels entry without tunnels map entry",
                        )
                    })?;
                return Ok(port);
            }
        }

        if let Some(mut entry) = self.tunnels.get_mut(&key) {
            entry.add_claim(TunnelClaim::Connection(connection_id.to_string()));
            let port = entry.forward.local_port;
            drop(entry);
            self.connection_tunnels
                .insert(connection_id.to_string(), key);
            return Ok(port);
        }

        let local_port = self
            .ensure_forward(&key, TunnelClaim::Connection(connection_id.to_string()))
            .await?;
        self.connection_tunnels
            .insert(connection_id.to_string(), key);
        Ok(local_port)
    }

    /// Ensure a shared session exists for `profile_id`, then a forward for `key`.
    ///
    /// Returns the forward's local port. On races, prefers the winning registry
    /// entry and closes the loser's forward (session only if we created a spare).
    pub(crate) async fn ensure_forward(
        &self,
        key: &TunnelKey,
        claim: TunnelClaim,
    ) -> Result<u16, ServiceError> {
        if let Some(mut entry) = self.tunnels.get_mut(key) {
            entry.add_claim(claim);
            return Ok(entry.forward.local_port);
        }

        let session_view = self.ensure_session(&key.profile_id).await?;
        let forward = match SshTunnel::open_forward(key.target_host.clone(), key.target_port) {
            Ok(f) => f,
            Err(e) => {
                self.reap_idle_session(&key.profile_id).await;
                return Err(e.into());
            }
        };
        if let Err(e) = SshTunnel::start_local_forward(&session_view, &forward).await {
            SshTunnel::close_forward(forward);
            self.reap_idle_session(&key.profile_id).await;
            return Err(e.into());
        }

        let local_port = match self.tunnels.entry(key.clone()) {
            dashmap::mapref::entry::Entry::Occupied(mut occ) => {
                occ.get_mut().add_claim(claim);
                let port = occ.get().forward.local_port;
                drop(occ);
                SshTunnel::close_forward(forward);
                port
            }
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                let port = forward.local_port;
                let mut managed = ManagedForward::new(forward);
                managed.add_claim(claim);
                vacant.insert(managed);
                if let Some(mut session) = self.sessions.get_mut(&key.profile_id) {
                    session.add_forward(key.clone());
                }
                port
            }
        };

        Ok(local_port)
    }

    /// Drop a session that still has zero forwards (failed forward setup / races).
    pub(crate) async fn reap_idle_session(&self, profile_id: &str) {
        let idle = self
            .sessions
            .get(profile_id)
            .map(|s| s.is_idle())
            .unwrap_or(false);
        if !idle {
            return;
        }
        if let Some((_, managed)) = self.sessions.remove(profile_id) {
            if managed.is_idle() {
                SshTunnel::close_session(managed.session).await.ok();
            } else {
                // Another task attached a forward between the check and remove.
                self.sessions.insert(profile_id.to_string(), managed);
            }
        }
    }

    /// Return a session view for opening forwards (clones the shared russh handle).
    pub(crate) async fn ensure_session(
        &self,
        profile_id: &str,
    ) -> Result<SshSession, ServiceError> {
        if let Some(existing) = self.sessions.get(profile_id) {
            return Ok(existing.session.clone());
        }

        let profile = self.config.get_ssh_profile(profile_id)?.ok_or_else(|| {
            ServiceError::app(
                "SSH_PROFILE_NOT_FOUND",
                format!("SSH profile '{profile_id}' not found"),
            )
        })?;
        let auth_config = build_auth_config_for_profile(&profile, &self.credentials)?;
        let ssh_config = SshHostConfig::new(&profile.host, profile.port, &auth_config);
        let jump_hosts = build_jump_hosts_for_profile(&profile)?;

        let new_session = SshTunnel::connect_session(
            profile_id.to_string(),
            &ssh_config,
            &auth_config,
            &jump_hosts,
        )
        .await?;

        match self.sessions.entry(profile_id.to_string()) {
            dashmap::mapref::entry::Entry::Occupied(occ) => {
                let view = occ.get().session.clone();
                drop(occ);
                SshTunnel::close_session(new_session).await.ok();
                Ok(view)
            }
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                let view = new_session.clone();
                vacant.insert(ManagedSession::new(new_session));
                Ok(view)
            }
        }
    }

    pub(crate) async fn release_forward_claim(
        &self,
        key: &TunnelKey,
        claim: TunnelClaim,
    ) -> Result<(), ServiceError> {
        let should_close_forward = {
            let mut entry = match self.tunnels.get_mut(key) {
                Some(e) => e,
                None => return Ok(()),
            };
            entry.remove_claim(&claim)
        };

        if !should_close_forward {
            return Ok(());
        }

        if let Some((_, managed)) = self.tunnels.remove(key) {
            SshTunnel::close_forward(managed.forward);
        }

        let should_close_session = {
            let mut session = match self.sessions.get_mut(&key.profile_id) {
                Some(s) => s,
                None => return Ok(()),
            };
            session.remove_forward(key)
        };

        if should_close_session {
            if let Some((_, managed)) = self.sessions.remove(&key.profile_id) {
                // Another task may have attached a forward between remove_forward
                // and remove — put it back instead of tearing down a live session.
                if managed.is_idle() {
                    SshTunnel::close_session(managed.session).await.ok();
                } else {
                    self.sessions.insert(key.profile_id.clone(), managed);
                }
            }
        }

        Ok(())
    }

    async fn release_connection_tunnel(&self, connection_id: &str) -> Result<(), ServiceError> {
        let Some((_, key)) = self.connection_tunnels.remove(connection_id) else {
            return Ok(());
        };
        self.release_forward_claim(&key, TunnelClaim::Connection(connection_id.to_string()))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::SshProfileConfig;
    use crate::tunnels::ManagedForward;
    use sqlator_core::models::{ConnectionConfig, ConnectionType};
    use std::path::PathBuf;

    struct Fixture {
        service: AppService,
        cleanup_dir: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.cleanup_dir);
        }
    }

    fn fixture(label: &str) -> Fixture {
        let id = uuid::Uuid::new_v4();
        let app_name = format!("sqlator-qa2-{label}-{id}");
        let cleanup_dir = dirs::config_dir().expect("config dir").join(&app_name);
        let service = AppService::with_app_name(&app_name).expect("AppService");
        Fixture {
            service,
            cleanup_dir,
        }
    }

    fn integration_enabled() -> bool {
        matches!(
            std::env::var("SQLATOR_INTEGRATION").ok().as_deref(),
            Some("1") | Some("true")
        )
    }

    #[tokio::test]
    async fn release_forward_claim_closes_idle_forward_without_session() {
        let fx = fixture("release");
        let key = TunnelKey::new("bastion", "pg.internal", 5432);
        let forward = SshTunnel::open_forward("pg.internal".into(), 5432).expect("open_forward");
        let cancel = forward.cancel_token.clone();
        let mut managed = ManagedForward::new(forward);
        managed.add_claim(TunnelClaim::Connection("c1".into()));
        managed.add_claim(TunnelClaim::Connection("c2".into()));
        fx.service.tunnels.insert(key.clone(), managed);

        fx.service
            .release_forward_claim(&key, TunnelClaim::Connection("c1".into()))
            .await
            .unwrap();
        assert!(fx.service.tunnels.contains_key(&key));
        assert!(!cancel.is_cancelled());

        fx.service
            .release_forward_claim(&key, TunnelClaim::Connection("c2".into()))
            .await
            .unwrap();
        assert!(!fx.service.tunnels.contains_key(&key));
        assert!(cancel.is_cancelled());
        assert!(fx.service.sessions.is_empty());
    }

    #[tokio::test]
    async fn ephemeral_test_ssh_failure_never_inserts_registry_entries() {
        let fx = fixture("ephemeral");
        let profile = fx
            .service
            .save_ssh_profile(SshProfileConfig {
                name: "unreachable".into(),
                host: "127.0.0.1".into(),
                port: 1, // closed port — connect fails without touching registry
                username: "sqlator".into(),
                auth_method: "password".into(),
                key_path: None,
                password: Some("sqlator".into()),
                key_passphrase: None,
                proxy_jump: vec![],
                local_port_binding: None,
                keepalive_interval: None,
            })
            .expect("save profile");

        let err = fx
            .service
            .test_connection_with_ssh(
                "postgres://sqlator:sqlator@postgres:5432/sqlator",
                &profile.id,
            )
            .await;
        assert!(err.is_err(), "expected SSH failure, got {err:?}");
        assert!(
            fx.service.tunnels.is_empty(),
            "ephemeral test_* must not leave forwards in the registry"
        );
        assert!(
            fx.service.sessions.is_empty(),
            "ephemeral test_* must not leave sessions in the registry"
        );
    }

    /// Live compose: same SSH profile, two DB targets → one session, two forwards.
    #[tokio::test]
    async fn shared_session_two_targets_refcount_teardown_live_ssh() {
        if !integration_enabled() {
            eprintln!("skipping: set SQLATOR_INTEGRATION=1");
            return;
        }

        let ssh_ok = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::net::TcpStream::connect(("127.0.0.1", 2222)),
        )
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);
        if !ssh_ok {
            eprintln!("skipping: OpenSSH not on localhost:2222");
            return;
        }

        let fx = fixture("shared-live");
        // Headless/CI keyring is unreliable; force an unlocked vault for profile secrets.
        fx.service
            .credentials()
            .set_mode(sqlator_core::credentials::StorageMode::Vault);
        fx.service
            .credentials()
            .vault
            .create("qa2-test-vault")
            .expect("create vault");

        let profile = fx
            .service
            .save_ssh_profile(SshProfileConfig {
                name: "compose-bastion".into(),
                host: "127.0.0.1".into(),
                port: 2222,
                username: "sqlator".into(),
                auth_method: "password".into(),
                key_path: None,
                password: Some("sqlator".into()),
                key_passphrase: None,
                proxy_jump: vec![],
                local_port_binding: None,
                keepalive_interval: None,
            })
            .expect("save ssh profile");

        let stored = fx
            .service
            .credentials()
            .get_credential(&profile.id, "password")
            .expect("get password")
            .expect("password must be stored for live SSH");
        assert_eq!(stored, "sqlator");

        let pg = fx
            .service
            .save_connection(ConnectionConfig {
                name: "pg-via-bastion".into(),
                color_id: "blue".into(),
                // Hostnames as seen from the openssh container on the compose network.
                url: "postgresql://sqlator:sqlator@postgres:5432/sqlator".into(),
                ssh_profile_id: Some(profile.id.clone()),
                group_id: None,
                connection_type: Some(ConnectionType::SshTunnel),
                container_name: None,
                container_port: None,
            })
            .await
            .expect("save pg");

        let mysql = fx
            .service
            .save_connection(ConnectionConfig {
                name: "mysql-via-bastion".into(),
                color_id: "green".into(),
                url: "mysql://sqlator:sqlator@mysql:3306/sqlator".into(),
                ssh_profile_id: Some(profile.id.clone()),
                group_id: None,
                connection_type: Some(ConnectionType::SshTunnel),
                container_name: None,
                container_port: None,
            })
            .await
            .expect("save mysql");

        fx.service
            .connect_database(&pg.id)
            .await
            .expect("connect postgres via shared session");
        fx.service
            .connect_database(&mysql.id)
            .await
            .expect("connect mysql via shared session");

        assert_eq!(
            fx.service.sessions.len(),
            1,
            "same profile must share one SSH session"
        );
        assert_eq!(
            fx.service.tunnels.len(),
            2,
            "different targets must get distinct forwards"
        );

        fx.service
            .disconnect_database(&pg.id)
            .await
            .expect("disconnect pg");
        assert_eq!(fx.service.tunnels.len(), 1);
        assert_eq!(
            fx.service.sessions.len(),
            1,
            "session stays while another forward remains"
        );
        assert!(fx
            .service
            .tunnel_local_port_for_connection(&mysql.id)
            .is_some());

        fx.service
            .disconnect_database(&mysql.id)
            .await
            .expect("disconnect mysql");
        assert!(fx.service.tunnels.is_empty());
        assert!(
            fx.service.sessions.is_empty(),
            "last forward drop must tear down the session"
        );
    }
}
