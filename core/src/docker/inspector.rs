use crate::ssh::command::{SshCommand, SshCommandResult};
use crate::ssh::{SshAuthConfig, SshHostConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub ip_address: String,
    pub status: ContainerStatus,
    pub ports: Vec<ContainerPort>,
    pub labels: HashMap<String, String>,
    pub database_type_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ContainerStatus {
    Running,
    Stopped,
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerPort {
    pub container_port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerSummary {
    pub name: String,
    pub image: String,
    pub status: String,
    pub database_type_hint: Option<String>,
}

#[derive(Debug, Error)]
pub enum DockerError {
    #[error("Container '{0}' not found. Check the name and try again.")]
    ContainerNotFound(String),

    #[error("Container '{0}' is not running. Start it and try again.")]
    ContainerStopped(String),

    #[error("Permission denied. User may not have Docker access on the server.")]
    PermissionDenied,

    #[error("Docker daemon not responding on server.")]
    DaemonUnreachable,

    #[error("Container is on an isolated network. Check Docker network configuration.")]
    NetworkIsolated,

    #[error("Command injection detected in container name")]
    InvalidContainerName,

    #[error("Failed to parse Docker output: {0}")]
    ParseError(String),

    #[error("SSH error: {0}")]
    SshError(#[from] crate::ssh::SshError),

    #[error("Command timed out")]
    Timeout,

    #[error("{0}")]
    Other(String),
}

fn validate_container_name(name: &str) -> Result<(), DockerError> {
    if name.is_empty() {
        return Err(DockerError::InvalidContainerName);
    }
    let forbidden = [';', '&', '|', '`', '$', '(', ')', '{', '}', '<', '>', '\n', '\r', '\\', '\''];
    if name.chars().any(|c| forbidden.contains(&c)) {
        return Err(DockerError::InvalidContainerName);
    }
    Ok(())
}

pub struct ContainerInspector;

impl ContainerInspector {
    pub async fn inspect(
        ssh_config: &SshHostConfig,
        auth_config: SshAuthConfig,
        jump_hosts: Vec<(SshHostConfig, SshAuthConfig)>,
        container_name: &str,
    ) -> Result<ContainerInfo, DockerError> {
        validate_container_name(container_name)?;

        let command = format!(
            "docker inspect --format '{{{{.State.Running}}}}|{{{{range .NetworkSettings.Networks}}}}{{{{.IPAddress}}}}{{{{end}}}}|{{{{.Config.Image}}}}|{{{{.Config.Labels}}}}|{{{{range $p, $conf := .NetworkSettings.Ports}}}}{{{{if $conf}}}}{{{{(index $conf 0).HostPort}}}}/{{{{$p}}}} {{{{end}}}}{{{{end}}}}' {}",
            shell_escape(container_name)
        );

        info!("ContainerInspector: inspecting '{}' over SSH", container_name);
        debug!("ContainerInspector: command: {}", command);

        let result = if jump_hosts.is_empty() {
            SshCommand::exec(
                ssh_config,
                auth_config,
                &command,
                std::time::Duration::from_secs(30),
            )
            .await?
        } else {
            SshCommand::exec_via_jump(
                ssh_config,
                auth_config,
                &jump_hosts,
                &command,
                std::time::Duration::from_secs(30),
            )
            .await?
        };

        Self::parse_inspect_result(container_name, result)
    }

    pub async fn list_running(
        ssh_config: &SshHostConfig,
        auth_config: SshAuthConfig,
        jump_hosts: Vec<(SshHostConfig, SshAuthConfig)>,
    ) -> Result<Vec<ContainerSummary>, DockerError> {
        let command = r#"docker ps --format '{{.Names}}|{{.Image}}|{{.Status}}'"#;

        info!("ContainerInspector: listing running containers over SSH");

        let result = if jump_hosts.is_empty() {
            SshCommand::exec(
                ssh_config,
                auth_config,
                command,
                std::time::Duration::from_secs(30),
            )
            .await?
        } else {
            SshCommand::exec_via_jump(
                ssh_config,
                auth_config,
                &jump_hosts,
                command,
                std::time::Duration::from_secs(30),
            )
            .await?
        };

        if let Some(code) = result.exit_code {
            if code != 0 {
                if result.stderr.contains("permission denied") || result.stderr.contains("Got permission denied") {
                    return Err(DockerError::PermissionDenied);
                }
                if result.stderr.contains("Cannot connect to the Docker daemon") {
                    return Err(DockerError::DaemonUnreachable);
                }
                return Err(DockerError::Other(result.stderr.trim().to_string()));
            }
        }

        let mut containers = Vec::new();
        for line in result.stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.splitn(3, '|').collect();
            if parts.len() >= 3 {
                let name = parts[0].to_string();
                let image = parts[1].to_string();
                let status = parts[2].to_string();
                let database_type_hint = detect_db_type_from_image(&image);
                containers.push(ContainerSummary {
                    name,
                    image,
                    status,
                    database_type_hint,
                });
            }
        }

        Ok(containers)
    }

    fn parse_inspect_result(
        container_name: &str,
        result: SshCommandResult,
    ) -> Result<ContainerInfo, DockerError> {
        if let Some(code) = result.exit_code {
            if code != 0 {
                let stderr = result.stderr.trim().to_string();
                if stderr.contains("No such object") || stderr.contains("No such container") {
                    return Err(DockerError::ContainerNotFound(container_name.to_string()));
                }
                if stderr.contains("permission denied") || stderr.contains("Got permission denied") {
                    return Err(DockerError::PermissionDenied);
                }
                if stderr.contains("Cannot connect to the Docker daemon") {
                    return Err(DockerError::DaemonUnreachable);
                }
                return Err(DockerError::Other(stderr));
            }
        }

        let output = result.stdout.trim().to_string();
        if output.is_empty() {
            return Err(DockerError::ContainerNotFound(container_name.to_string()));
        }

        let parts: Vec<&str> = output.splitn(5, '|').collect();
        if parts.len() < 2 {
            return Err(DockerError::ParseError(format!(
                "Unexpected docker inspect output: {}", output
            )));
        }

        let running_str = parts[0].trim();
        let is_running = running_str == "true";

        if !is_running {
            return Ok(ContainerInfo {
                ip_address: String::new(),
                status: ContainerStatus::Stopped,
                ports: vec![],
                labels: HashMap::new(),
                database_type_hint: None,
            });
        }

        let ip_address = parts[1].trim().to_string();
        let image = parts.get(2).map(|s| s.trim()).unwrap_or("");
        let labels_str = parts.get(3).map(|s| s.trim()).unwrap_or("");
        let ports_str = parts.get(4).map(|s| s.trim()).unwrap_or("");

        let labels = parse_labels(labels_str);
        let ports = parse_ports(ports_str);
        let database_type_hint = detect_db_type(&image, &labels);

        if ip_address.is_empty() {
            warn!("Container '{}' is running but has no IP address (may be on an isolated network)", container_name);
            return Err(DockerError::NetworkIsolated);
        }

        info!(
            "ContainerInspector: found '{}' at IP {} (db_type_hint={:?})",
            container_name, ip_address, database_type_hint
        );

        Ok(ContainerInfo {
            ip_address,
            status: ContainerStatus::Running,
            ports,
            labels,
            database_type_hint,
        })
    }
}

fn shell_escape(s: &str) -> String {
    let mut escaped = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
            escaped.push(c);
        } else {
            escaped.push('\\');
            escaped.push(c);
        }
    }
    escaped
}

pub fn parse_labels(labels_str: &str) -> HashMap<String, String> {
    let mut labels = HashMap::new();
    if labels_str.starts_with("map[") && labels_str.ends_with(']') {
        let inner = &labels_str[4..labels_str.len() - 1];
        let mut key = String::new();
        let mut value = String::new();
        let mut in_value = false;
        let mut depth = 0;

        for c in inner.chars() {
            match c {
                ':' if depth == 0 && !in_value => {
                    in_value = true;
                }
                ' ' if depth == 0 && in_value => {
                    if !key.is_empty() {
                        labels.insert(key.clone(), value.clone());
                    }
                    key.clear();
                    value.clear();
                    in_value = false;
                }
                '[' => {
                    depth += 1;
                    if in_value {
                        value.push(c);
                    } else {
                        key.push(c);
                    }
                }
                ']' => {
                    depth -= 1;
                    if in_value {
                        value.push(c);
                    } else {
                        key.push(c);
                    }
                }
                _ => {
                    if in_value {
                        value.push(c);
                    } else {
                        key.push(c);
                    }
                }
            }
        }
        if !key.is_empty() {
            labels.insert(key, value);
        }
    }
    labels
}

fn parse_ports(ports_str: &str) -> Vec<ContainerPort> {
    let mut ports = Vec::new();
    for part in ports_str.split_whitespace() {
        if let Some((_host_port, container_spec)) = part.split_once('/') {
            if let Ok(container_port) = container_spec.parse::<u16>() {
                ports.push(ContainerPort {
                    container_port,
                    protocol: "tcp".to_string(),
                });
            }
        }
    }
    ports
}

pub fn detect_db_type(image: &str, labels: &HashMap<String, String>) -> Option<String> {
    if let Some(db_label) = labels.get("com.docker.compose.service") {
        let hint = detect_db_type_from_image(db_label);
        if hint.is_some() {
            return hint;
        }
    }
    detect_db_type_from_image(image)
}

pub fn detect_db_type_from_image(image: &str) -> Option<String> {
    let lower = image.to_lowercase();
    if lower.contains("postgres") || lower.contains("postgresql") {
        Some("postgres".to_string())
    } else if lower.contains("mysql") {
        Some("mysql".to_string())
    } else if lower.contains("mariadb") {
        Some("mysql".to_string())
    } else if lower.contains("mssql") || lower.contains("sqlserver") {
        Some("mssql".to_string())
    } else if lower.contains("oracle") {
        Some("oracle".to_string())
    } else if lower.contains("clickhouse") {
        Some("clickhouse".to_string())
    } else {
        None
    }
}
