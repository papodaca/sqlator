use thiserror::Error;

#[derive(Debug, Error)]
pub enum SshError {
    #[error("Failed to connect to SSH host: {0}")]
    ConnectionFailed(String),

    #[error("SSH authentication failed: {0}")]
    AuthFailed(String),

    #[error("Failed to load SSH key: {0}")]
    KeyLoadFailed(String),

    #[error("Failed to bind local port: {0}")]
    PortBindFailed(String),

    #[error("Failed to establish port forward: {0}")]
    PortForwardFailed(String),

    #[error("SSH tunnel not found: {0}")]
    TunnelNotFound(String),

    #[error("Jump host connection failed: {0}")]
    JumpHostFailed(String),

    #[error("SSH agent error: {0}")]
    AgentError(String),

    #[error("Host key verification failed for {host}")]
    HostKeyVerification { host: String },

    #[error("Unknown host key for {host}. Fingerprint: {fingerprint}")]
    UnknownHostKey { host: String, fingerprint: String },

    #[error("SSH configuration error: {0}")]
    ConfigError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub type SshResult<T> = Result<T, SshError>;
