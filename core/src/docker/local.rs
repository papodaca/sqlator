use crate::docker::inspector::{ContainerInfo, ContainerStatus, ContainerSummary, DockerError};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct LocalDockerAccess {
    socket_path: PathBuf,
}

impl LocalDockerAccess {
    pub fn new() -> Result<Self, DockerError> {
        let socket = PathBuf::from("/var/run/docker.sock");
        if socket.exists() {
            Ok(Self { socket_path: socket })
        } else {
            Err(DockerError::Other("Docker socket not found at /var/run/docker.sock".to_string()))
        }
    }

    pub fn with_socket_path(path: PathBuf) -> Result<Self, DockerError> {
        if path.exists() {
            Ok(Self { socket_path: path })
        } else {
            Err(DockerError::Other(format!(
                "Docker socket not found at {}",
                path.display()
            )))
        }
    }

    pub async fn inspect(&self, container_name: &str) -> Result<ContainerInfo, DockerError> {
        let output = tokio::process::Command::new("docker")
            .args([
                "inspect",
                "--format",
                "{{.State.Running}}|{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}|{{.Config.Image}}|{{.Config.Labels}}|{{range $p, $conf := .NetworkSettings.Ports}}{{if $conf}}{{(index $conf 0).HostPort}}/{{$p}} {{end}}{{end}}",
                container_name,
            ])
            .env("DOCKER_HOST", format!("unix://{}", self.socket_path.display()))
            .output()
            .await
            .map_err(|e| DockerError::Other(format!("Failed to execute docker: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("No such object") || stderr.contains("No such container") {
                return Err(DockerError::ContainerNotFound(container_name.to_string()));
            }
            if stderr.contains("permission denied") {
                return Err(DockerError::PermissionDenied);
            }
            if stderr.contains("Cannot connect to the Docker daemon") {
                return Err(DockerError::DaemonUnreachable);
            }
            return Err(DockerError::Other(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_local_inspect(container_name, &stdout)
    }

    pub async fn list_running(&self) -> Result<Vec<ContainerSummary>, DockerError> {
        let output = tokio::process::Command::new("docker")
            .args(["ps", "--format", "{{.Names}}|{{.Image}}|{{.Status}}"])
            .env("DOCKER_HOST", format!("unix://{}", self.socket_path.display()))
            .output()
            .await
            .map_err(|e| DockerError::Other(format!("Failed to execute docker: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("permission denied") {
                return Err(DockerError::PermissionDenied);
            }
            return Err(DockerError::Other(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut containers = Vec::new();
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.splitn(3, '|').collect();
            if parts.len() >= 3 {
                let name = parts[0].to_string();
                let image = parts[1].to_string();
                let status = parts[2].to_string();
                let database_type_hint = super::inspector::detect_db_type_from_image(&image);
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
}

fn parse_local_inspect(container_name: &str, output: &str) -> Result<ContainerInfo, DockerError> {
    let output = output.trim();
    if output.is_empty() {
        return Err(DockerError::ContainerNotFound(container_name.to_string()));
    }

    let parts: Vec<&str> = output.splitn(5, '|').collect();
    if parts.len() < 2 {
        return Err(DockerError::ParseError(format!(
            "Unexpected docker inspect output: {}", output
        )));
    }

    let is_running = parts[0].trim() == "true";
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

    let labels = super::inspector::parse_labels(labels_str);
    let database_type_hint = super::inspector::detect_db_type(image, &labels);

    Ok(ContainerInfo {
        ip_address,
        status: ContainerStatus::Running,
        ports: vec![],
        labels,
        database_type_hint,
    })
}
