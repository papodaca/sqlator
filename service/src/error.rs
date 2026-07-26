//! Typed errors for the shared application layer.
//!
//! Frontends flatten once at the adapter boundary:
//! - Tauri → `String`
//! - web → `(StatusCode, String)`
//! - GTK → dialog vs toast by [`ServiceError::code`]

use sqlator_core::docker::DockerError;
use sqlator_core::error::CoreError;
use sqlator_core::ssh::SshError;
use thiserror::Error;

/// Application-layer error with a stable machine-readable `code`.
#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("{0}")]
    Core(#[from] CoreError),

    #[error("{0}")]
    Ssh(#[from] SshError),

    #[error("{0}")]
    Docker(#[from] DockerError),

    /// Policy / orchestration failures that do not originate in core.
    #[error("[{code}] {message}")]
    App { message: String, code: String },
}

impl ServiceError {
    /// Stable code for UI branching (`VAULT_LOCKED`, `NO_CONNECTION`, …).
    pub fn code(&self) -> &str {
        match self {
            ServiceError::Core(e) => e.code.as_str(),
            ServiceError::Ssh(e) => ssh_code(e),
            ServiceError::Docker(e) => docker_code(e),
            ServiceError::App { code, .. } => code.as_str(),
        }
    }

    pub fn message(&self) -> String {
        self.to_string()
    }

    pub fn app(code: impl Into<String>, message: impl Into<String>) -> Self {
        ServiceError::App {
            code: code.into(),
            message: message.into(),
        }
    }
}

fn ssh_code(e: &SshError) -> &'static str {
    match e {
        SshError::ConnectionFailed(_) => "SSH_CONNECTION_FAILED",
        SshError::AuthFailed(_) => "SSH_AUTH_FAILED",
        SshError::KeyLoadFailed(_) => "SSH_KEY_LOAD_FAILED",
        SshError::PortBindFailed(_) => "SSH_PORT_BIND_FAILED",
        SshError::PortForwardFailed(_) => "SSH_PORT_FORWARD_FAILED",
        SshError::TunnelNotFound(_) => "SSH_TUNNEL_NOT_FOUND",
        SshError::JumpHostFailed(_) => "SSH_JUMP_HOST_FAILED",
        SshError::AgentError(_) => "SSH_AGENT_ERROR",
        SshError::HostKeyVerification { .. } => "SSH_HOST_KEY_VERIFICATION",
        SshError::UnknownHostKey { .. } => "SSH_UNKNOWN_HOST_KEY",
        SshError::ConfigError(_) => "SSH_CONFIG_ERROR",
        SshError::Io(_) => "SSH_IO_ERROR",
        SshError::Other(_) => "SSH_ERROR",
    }
}

fn docker_code(e: &DockerError) -> &'static str {
    match e {
        DockerError::ContainerNotFound(_) => "DOCKER_CONTAINER_NOT_FOUND",
        DockerError::ContainerStopped(_) => "DOCKER_CONTAINER_STOPPED",
        DockerError::PermissionDenied => "DOCKER_PERMISSION_DENIED",
        DockerError::DaemonUnreachable => "DOCKER_DAEMON_UNREACHABLE",
        DockerError::NetworkIsolated => "DOCKER_NETWORK_ISOLATED",
        DockerError::InvalidContainerName => "DOCKER_INVALID_CONTAINER_NAME",
        DockerError::ParseError(_) => "DOCKER_PARSE_ERROR",
        DockerError::SshError(inner) => ssh_code(inner),
        DockerError::Timeout => "DOCKER_TIMEOUT",
        DockerError::Other(_) => "DOCKER_ERROR",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_preserves_code() {
        let err = ServiceError::from(CoreError {
            message: "Not connected".into(),
            code: "NO_CONNECTION".into(),
        });
        assert_eq!(err.code(), "NO_CONNECTION");
    }

    #[test]
    fn app_error_exposes_code() {
        let err = ServiceError::app("MULTI_DB_REQUIRED", "single-db mode forbids this");
        assert_eq!(err.code(), "MULTI_DB_REQUIRED");
    }
}
