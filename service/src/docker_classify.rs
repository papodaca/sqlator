//! Docker / connection-test error classification with remediation hints.
//!
//! Ported from `DockerConnectionWizard.svelte` so every frontend can show the
//! same actionable guidance. Prefer matching on [`ServiceError::code`] when the
//! failure originated in core Docker / SSH; fall back to message substrings for
//! driver-level TLS/auth/reachability strings.

use crate::error::ServiceError;
use serde::{Deserialize, Serialize};

/// Stable kind shared with the Svelte wire format (`snake_case`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerErrorKind {
    PermissionDenied,
    DaemonUnreachable,
    NetworkIsolated,
    NotFound,
    Stopped,
    Timeout,
    InvalidName,
    Ssh,
    Other,
}

/// Classified failure for UI display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifiedDockerError {
    pub kind: DockerErrorKind,
    /// Remediation hint; empty means none.
    pub hint: String,
    /// `true` when retry without user action may help.
    pub transient: bool,
}

impl ClassifiedDockerError {
    fn new(kind: DockerErrorKind, transient: bool, hint: impl Into<String>) -> Self {
        Self {
            kind,
            hint: hint.into(),
            transient,
        }
    }
}

/// Classify a discovery / list / inspect failure from its message text.
pub fn classify_docker_error(msg: &str) -> ClassifiedDockerError {
    let m = msg.to_lowercase();
    if m.contains("permission denied")
        || m.contains("docker access")
        || m.contains("got permission denied")
    {
        return ClassifiedDockerError::new(
            DockerErrorKind::PermissionDenied,
            false,
            "Add the SSH user to the 'docker' group on the server:\nsudo usermod -aG docker <username>  && newgrp docker",
        );
    }
    if m.contains("docker daemon")
        || m.contains("cannot connect to the docker")
        || m.contains("daemon not responding")
    {
        return ClassifiedDockerError::new(
            DockerErrorKind::DaemonUnreachable,
            true,
            "Check that Docker is running on the server:\nsudo systemctl status docker",
        );
    }
    if (m.contains("flatpak") || m.contains("sandbox"))
        && m.contains("local docker")
        && (m.contains("unavailable") || m.contains("disabled"))
    {
        return ClassifiedDockerError::new(
            DockerErrorKind::Other,
            false,
            "Local Docker is disabled in Flatpak. Use remote Docker over SSH instead.",
        );
    }
    if m.contains("isolated network") || m.contains("docker network") {
        return ClassifiedDockerError::new(
            DockerErrorKind::NetworkIsolated,
            false,
            "The container has no routable IP. Connect it to a bridge network:\ndocker network connect bridge <container>",
        );
    }
    if m.contains("not running") || m.contains("is stopped") {
        return ClassifiedDockerError::new(
            DockerErrorKind::Stopped,
            false,
            "Start the container, then retry discovery:\ndocker start <container>",
        );
    }
    if m.contains("not found") || m.contains("no such object") || m.contains("check the name") {
        return ClassifiedDockerError::new(
            DockerErrorKind::NotFound,
            false,
            "List all containers to verify the name:\ndocker ps -a --format '{{.Names}}'",
        );
    }
    if m.contains("timed out") || m.contains("timeout") {
        return ClassifiedDockerError::new(
            DockerErrorKind::Timeout,
            true,
            "The SSH command timed out. Check server connectivity and try again.",
        );
    }
    if m.contains("injection") || m.contains("invalid container name") {
        return ClassifiedDockerError::new(
            DockerErrorKind::InvalidName,
            false,
            "Container names may only contain letters, numbers, hyphens, underscores, and dots.",
        );
    }
    if m.contains("ssh")
        || m.contains("auth")
        || m.contains("handshake")
        || m.contains("connection refused")
    {
        return ClassifiedDockerError::new(
            DockerErrorKind::Ssh,
            true,
            "Check the SSH profile settings and ensure the server is reachable.",
        );
    }
    ClassifiedDockerError::new(DockerErrorKind::Other, false, "")
}

/// Classify a connection-test failure (TLS / SSH / auth / reachability).
pub fn classify_test_error(msg: &str) -> ClassifiedDockerError {
    let m = msg.to_lowercase();
    // TLS/SSL before generic SSH — HandshakeFailure is a DB TLS alert.
    if m.contains("handshakefailure")
        || m.contains("fatal alert")
        || m.contains("tls")
        || m.contains("ssl")
    {
        return ClassifiedDockerError::new(
            DockerErrorKind::Other,
            false,
            "TLS/SSL negotiation with the database failed. If the database doesn't require TLS, try appending ?sslmode=disable to the connection URL.",
        );
    }
    if m.contains("ssh")
        || m.contains("auth failed")
        || m.contains("authentication rejected")
        || m.contains("connection refused to ssh")
    {
        return ClassifiedDockerError::new(
            DockerErrorKind::Ssh,
            true,
            "SSH tunnel failed to establish. Verify the SSH profile credentials.",
        );
    }
    if m.contains("connection refused")
        || m.contains("no route to host")
        || m.contains("timed out")
        || m.contains("timeout")
    {
        return ClassifiedDockerError::new(
            DockerErrorKind::Timeout,
            true,
            "Could not reach the database port inside the container. Confirm the port and that the database is accepting connections.",
        );
    }
    if m.contains("password")
        || m.contains("authentication")
        || m.contains("login failed")
        || m.contains("access denied")
    {
        return ClassifiedDockerError::new(
            DockerErrorKind::PermissionDenied,
            false,
            "Check the database username and password.",
        );
    }
    ClassifiedDockerError::new(DockerErrorKind::Other, false, "")
}

/// Prefer stable [`ServiceError::code`] for discovery errors; fall back to message matching.
pub fn classify_docker_service_error(err: &ServiceError) -> ClassifiedDockerError {
    match err.code() {
        "DOCKER_PERMISSION_DENIED" => ClassifiedDockerError::new(
            DockerErrorKind::PermissionDenied,
            false,
            "Add the SSH user to the 'docker' group on the server:\nsudo usermod -aG docker <username>  && newgrp docker",
        ),
        "DOCKER_DAEMON_UNREACHABLE" => ClassifiedDockerError::new(
            DockerErrorKind::DaemonUnreachable,
            true,
            "Check that Docker is running on the server:\nsudo systemctl status docker",
        ),
        "DOCKER_NETWORK_ISOLATED" => ClassifiedDockerError::new(
            DockerErrorKind::NetworkIsolated,
            false,
            "The container has no routable IP. Connect it to a bridge network:\ndocker network connect bridge <container>",
        ),
        "DOCKER_CONTAINER_STOPPED" => ClassifiedDockerError::new(
            DockerErrorKind::Stopped,
            false,
            "Start the container, then retry discovery:\ndocker start <container>",
        ),
        "DOCKER_CONTAINER_NOT_FOUND" => ClassifiedDockerError::new(
            DockerErrorKind::NotFound,
            false,
            "List all containers to verify the name:\ndocker ps -a --format '{{.Names}}'",
        ),
        "DOCKER_TIMEOUT" => ClassifiedDockerError::new(
            DockerErrorKind::Timeout,
            true,
            "The SSH command timed out. Check server connectivity and try again.",
        ),
        "DOCKER_INVALID_CONTAINER_NAME" => ClassifiedDockerError::new(
            DockerErrorKind::InvalidName,
            false,
            "Container names may only contain letters, numbers, hyphens, underscores, and dots.",
        ),
        code if code.starts_with("SSH_") => ClassifiedDockerError::new(
            DockerErrorKind::Ssh,
            true,
            "Check the SSH profile settings and ensure the server is reachable.",
        ),
        _ => classify_docker_error(&err.message()),
    }
}

/// Prefer codes when present; otherwise use the test-error message classifier.
pub fn classify_test_service_error(err: &ServiceError) -> ClassifiedDockerError {
    match err.code() {
        code if code.starts_with("SSH_") => ClassifiedDockerError::new(
            DockerErrorKind::Ssh,
            true,
            "SSH tunnel failed to establish. Verify the SSH profile credentials.",
        ),
        "DOCKER_TIMEOUT" => classify_test_error(&err.message()),
        _ => classify_test_error(&err.message()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlator_core::docker::DockerError;

    #[test]
    fn discovery_permission_denied() {
        let c = classify_docker_error(
            "Got permission denied while trying to connect to the Docker daemon",
        );
        assert_eq!(c.kind, DockerErrorKind::PermissionDenied);
        assert!(!c.transient);
        assert!(c.hint.contains("usermod"));
    }

    #[test]
    fn discovery_daemon_unreachable_is_transient() {
        let c = classify_docker_error(
            "Cannot connect to the Docker daemon at unix:///var/run/docker.sock",
        );
        assert_eq!(c.kind, DockerErrorKind::DaemonUnreachable);
        assert!(c.transient);
    }

    #[test]
    fn discovery_from_service_code() {
        let err = ServiceError::from(DockerError::NetworkIsolated);
        let c = classify_docker_service_error(&err);
        assert_eq!(c.kind, DockerErrorKind::NetworkIsolated);
        assert!(c.hint.contains("network connect"));
    }

    #[test]
    fn discovery_flatpak_local_docker_disabled() {
        let c = classify_docker_error(
            "Local Docker is unavailable inside Flatpak sandbox. Use remote Docker over SSH.",
        );
        assert_eq!(c.kind, DockerErrorKind::Other);
        assert!(c.hint.contains("remote Docker over SSH"));
        assert!(!c.transient);
    }

    #[test]
    fn test_tls_before_ssh() {
        let c = classify_test_error("tls handshake failed: HandshakeFailure fatal alert");
        assert_eq!(c.kind, DockerErrorKind::Other);
        assert!(c.hint.contains("sslmode=disable"));
    }

    #[test]
    fn test_auth_failure() {
        let c = classify_test_error("password authentication failed for user \"postgres\"");
        assert_eq!(c.kind, DockerErrorKind::PermissionDenied);
        assert!(!c.transient);
    }

    #[test]
    fn test_reachability_transient() {
        let c = classify_test_error("connection refused");
        assert_eq!(c.kind, DockerErrorKind::Timeout);
        assert!(c.transient);
    }
}