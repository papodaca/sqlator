//! Connect / disconnect orchestration across direct, SSH, and Docker paths.
//!
//! Tunnel sharing uses [`crate::tunnels::TunnelKey`] `(profile, host, port)` with
//! refcounting. Ephemeral `test_*` tunnels are never inserted into the registry.

use crate::connections::default_port_for_db_type;
use crate::error::ServiceError;
use crate::service::AppService;
use crate::ssh::{build_auth_config_for_profile, build_jump_hosts_for_profile};
use crate::tunnels::{
    rewrite_url_via_localhost, target_from_db_url, ManagedTunnel, TunnelClaim, TunnelKey,
};
use sqlator_core::docker::inspector::ContainerInspector;
use sqlator_core::docker::{ContainerStatus, LocalDockerAccess};
use sqlator_core::models::{ConnectionType, SavedConnection};
use sqlator_core::ssh::{SshHostConfig, SshTunnel};
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
            .map(|entry| entry.handle.local_port)
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
        let ssh_config = SshHostConfig::new(&profile.host, profile.port, auth_config.clone());
        let jump_hosts = build_jump_hosts_for_profile(&profile)?;
        let tunnel_id = format!("test-{}", uuid::Uuid::new_v4());

        let tunnel = SshTunnel::create(
            tunnel_id,
            &ssh_config,
            auth_config,
            target_host,
            target_port,
            jump_hosts,
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
        let ssh_config = SshHostConfig::new(&profile.host, profile.port, auth_config.clone());
        let jump_hosts = build_jump_hosts_for_profile(&profile)?;

        let container_info = ContainerInspector::inspect(
            &ssh_config,
            auth_config.clone(),
            jump_hosts.clone(),
            container_name,
        )
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
            auth_config,
            container_info.ip_address.clone(),
            port,
            jump_hosts,
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
        let ssh_config = SshHostConfig::new(&profile.host, profile.port, auth_config.clone());
        let jump_hosts = build_jump_hosts_for_profile(&profile)?;

        let container_info =
            ContainerInspector::inspect(&ssh_config, auth_config, jump_hosts, container_name)
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

    /// Acquire or share a tunnel for `(profile, target)`; claim `connection_id`.
    ///
    /// If this connection already held a different key (stale reconnect), that
    /// claim is released first.
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
                // Already claimed this exact tunnel — reuse.
                let port = self
                    .tunnels
                    .get(&key)
                    .map(|e| e.handle.local_port)
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
            let port = entry.handle.local_port;
            drop(entry);
            self.connection_tunnels
                .insert(connection_id.to_string(), key);
            return Ok(port);
        }

        let profile = self.config.get_ssh_profile(profile_id)?.ok_or_else(|| {
            ServiceError::app(
                "SSH_PROFILE_NOT_FOUND",
                format!("SSH profile '{profile_id}' not found"),
            )
        })?;
        let auth_config = build_auth_config_for_profile(&profile, &self.credentials)?;
        let ssh_config = SshHostConfig::new(&profile.host, profile.port, auth_config.clone());
        let jump_hosts = build_jump_hosts_for_profile(&profile)?;

        let tunnel = SshTunnel::create(
            profile_id.to_string(),
            &ssh_config,
            auth_config,
            target_host.to_string(),
            target_port,
            jump_hosts,
        )
        .await?;
        SshTunnel::start_forwarding(&tunnel).await?;

        // Another task may have inserted the same key while we awaited SSH setup.
        // Prefer the winner; close the unused handle so we do not leak a listener.
        let local_port = match self.tunnels.entry(key.clone()) {
            dashmap::mapref::entry::Entry::Occupied(mut occ) => {
                occ.get_mut()
                    .add_claim(TunnelClaim::Connection(connection_id.to_string()));
                let port = occ.get().handle.local_port;
                drop(occ);
                SshTunnel::close(tunnel).await.ok();
                port
            }
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                let mut managed = ManagedTunnel::new(tunnel);
                managed.add_claim(TunnelClaim::Connection(connection_id.to_string()));
                let port = managed.handle.local_port;
                vacant.insert(managed);
                port
            }
        };
        self.connection_tunnels
            .insert(connection_id.to_string(), key);

        Ok(local_port)
    }

    async fn release_connection_tunnel(&self, connection_id: &str) -> Result<(), ServiceError> {
        let Some((_, key)) = self.connection_tunnels.remove(connection_id) else {
            return Ok(());
        };

        let should_close = {
            let mut entry = match self.tunnels.get_mut(&key) {
                Some(e) => e,
                None => return Ok(()),
            };
            entry.remove_claim(&TunnelClaim::Connection(connection_id.to_string()))
        };

        if should_close {
            if let Some((_, managed)) = self.tunnels.remove(&key) {
                SshTunnel::close(managed.handle).await.ok();
            }
        }
        Ok(())
    }
}
