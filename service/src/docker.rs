//! Docker discovery and DTO mapping (local and remote).

use crate::error::ServiceError;
use crate::service::AppService;
use crate::ssh::{build_auth_config_for_profile, build_jump_hosts_for_profile};
use serde::{Deserialize, Serialize};
use sqlator_core::docker::inspector::ContainerInspector;
use sqlator_core::docker::{ContainerStatus, LocalDockerAccess};
use sqlator_core::ssh::SshHostConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerContainerInfo {
    pub ip_address: String,
    pub status: String,
    pub ports: Vec<ContainerPortInfo>,
    pub database_type_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerPortInfo {
    pub container_port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerSummaryInfo {
    pub name: String,
    pub image: String,
    pub status: String,
    pub database_type_hint: Option<String>,
}

fn status_str(status: ContainerStatus) -> String {
    match status {
        ContainerStatus::Running => "running".to_string(),
        ContainerStatus::Stopped => "stopped".to_string(),
        ContainerStatus::NotFound => "not_found".to_string(),
    }
}

impl AppService {
    pub async fn discover_container(
        &self,
        ssh_profile_id: &str,
        container_name: &str,
    ) -> Result<DockerContainerInfo, ServiceError> {
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

        let info =
            ContainerInspector::inspect(&ssh_config, auth_config, jump_hosts, container_name)
                .await?;

        Ok(DockerContainerInfo {
            ip_address: info.ip_address,
            status: status_str(info.status),
            ports: info
                .ports
                .into_iter()
                .map(|p| ContainerPortInfo {
                    container_port: p.container_port,
                    protocol: p.protocol,
                })
                .collect(),
            database_type_hint: info.database_type_hint,
        })
    }

    pub async fn list_running_containers(
        &self,
        ssh_profile_id: &str,
    ) -> Result<Vec<ContainerSummaryInfo>, ServiceError> {
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

        let containers =
            ContainerInspector::list_running(&ssh_config, auth_config, jump_hosts).await?;

        Ok(containers
            .into_iter()
            .map(|c| ContainerSummaryInfo {
                name: c.name,
                image: c.image,
                status: c.status,
                database_type_hint: c.database_type_hint,
            })
            .collect())
    }

    pub async fn discover_local_container(
        &self,
        container_name: &str,
    ) -> Result<DockerContainerInfo, ServiceError> {
        let local_docker = LocalDockerAccess::new()?;
        let info = local_docker.inspect(container_name).await?;
        Ok(DockerContainerInfo {
            ip_address: info.ip_address,
            status: status_str(info.status),
            ports: info
                .ports
                .into_iter()
                .map(|p| ContainerPortInfo {
                    container_port: p.container_port,
                    protocol: p.protocol,
                })
                .collect(),
            database_type_hint: info.database_type_hint,
        })
    }

    pub async fn list_local_containers(&self) -> Result<Vec<ContainerSummaryInfo>, ServiceError> {
        let local_docker = LocalDockerAccess::new()?;
        let containers = local_docker.list_running().await?;
        Ok(containers
            .into_iter()
            .map(|c| ContainerSummaryInfo {
                name: c.name,
                image: c.image,
                status: c.status,
                database_type_hint: c.database_type_hint,
            })
            .collect())
    }
}
