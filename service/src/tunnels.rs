//! Profile+target tunnel registry with connection-id refcounting.
//!
//! See `docs/plans/gtk/2026-07-26-001-design-ssh-tunnel-registry-keying.md`.

use sqlator_core::ssh::TunnelHandle;
use std::collections::HashSet;

/// Registry key: share only when profile **and** remote target match.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TunnelKey {
    pub profile_id: String,
    pub target_host: String,
    pub target_port: u16,
}

impl TunnelKey {
    pub fn new(
        profile_id: impl Into<String>,
        target_host: impl Into<String>,
        target_port: u16,
    ) -> Self {
        Self {
            profile_id: profile_id.into(),
            target_host: target_host.into(),
            target_port,
        }
    }
}

/// Who holds a claim on a managed tunnel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelClaim {
    /// A saved DB connection is using this forward.
    Connection(String),
    /// Standalone `create_ssh_tunnel` (no connection id).
    Standalone,
}

/// A live tunnel plus its refcount / standalone claim.
pub struct ManagedTunnel {
    pub handle: TunnelHandle,
    users: HashSet<String>,
    standalone: bool,
}

impl ManagedTunnel {
    pub fn new(handle: TunnelHandle) -> Self {
        Self {
            handle,
            users: HashSet::new(),
            standalone: false,
        }
    }

    pub fn add_claim(&mut self, claim: TunnelClaim) {
        match claim {
            TunnelClaim::Connection(id) => {
                self.users.insert(id);
            }
            TunnelClaim::Standalone => {
                self.standalone = true;
            }
        }
    }

    /// Remove a claim. Returns `true` when the tunnel has no remaining users
    /// and should be closed and removed from the registry.
    pub fn remove_claim(&mut self, claim: &TunnelClaim) -> bool {
        match claim {
            TunnelClaim::Connection(id) => {
                self.users.remove(id);
            }
            TunnelClaim::Standalone => {
                self.standalone = false;
            }
        }
        self.is_idle()
    }

    pub fn is_idle(&self) -> bool {
        self.users.is_empty() && !self.standalone
    }

    pub fn users(&self) -> &HashSet<String> {
        &self.users
    }

    pub fn has_standalone(&self) -> bool {
        self.standalone
    }

    pub fn has_connection(&self, connection_id: &str) -> bool {
        self.users.contains(connection_id)
    }
}

/// Info DTO returned to frontends for active tunnels.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SshTunnelInfo {
    pub profile_id: String,
    pub local_port: u16,
    pub target_host: String,
    pub target_port: u16,
}

impl From<&TunnelHandle> for SshTunnelInfo {
    fn from(t: &TunnelHandle) -> Self {
        Self {
            profile_id: t.profile_id.clone(),
            local_port: t.local_port,
            target_host: t.target_host.clone(),
            target_port: t.target_port,
        }
    }
}

/// Parse host/port from a database URL (shared by connect + test paths).
pub fn target_from_db_url(url: &str) -> Result<(String, u16), crate::error::ServiceError> {
    let parsed = url::Url::parse(url).map_err(|e| {
        crate::error::ServiceError::app("INVALID_URL", format!("Invalid database URL: {e}"))
    })?;
    let host = parsed.host_str().unwrap_or("localhost").to_string();
    let default_port = match parsed.scheme() {
        "postgres" | "postgresql" => 5432u16,
        "mysql" | "mariadb" => 3306,
        _ => 0,
    };
    let port = parsed.port().unwrap_or(default_port);
    Ok((host, port))
}

/// Rewrite a DB URL to point at a local tunnel endpoint.
pub fn rewrite_url_via_localhost(
    url: &str,
    local_port: u16,
) -> Result<String, crate::error::ServiceError> {
    let parsed = url::Url::parse(url).map_err(|e| {
        crate::error::ServiceError::app("INVALID_URL", format!("Invalid database URL: {e}"))
    })?;
    let mut rewritten = parsed;
    let _ = rewritten.set_host(Some("127.0.0.1"));
    let _ = rewritten.set_port(Some(local_port));
    Ok(rewritten.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claim/refcount logic without a live SSH session (mirrors [`ManagedTunnel`]).
    struct ClaimProbe {
        users: HashSet<String>,
        standalone: bool,
    }

    impl ClaimProbe {
        fn new() -> Self {
            Self {
                users: HashSet::new(),
                standalone: false,
            }
        }

        fn add_claim(&mut self, claim: TunnelClaim) {
            match claim {
                TunnelClaim::Connection(id) => {
                    self.users.insert(id);
                }
                TunnelClaim::Standalone => self.standalone = true,
            }
        }

        fn remove_claim(&mut self, claim: &TunnelClaim) -> bool {
            match claim {
                TunnelClaim::Connection(id) => {
                    self.users.remove(id);
                }
                TunnelClaim::Standalone => self.standalone = false,
            }
            self.users.is_empty() && !self.standalone
        }
    }

    #[test]
    fn tunnel_key_distinguishes_targets_on_same_profile() {
        let a = TunnelKey::new("bastion", "pg.internal", 5432);
        let b = TunnelKey::new("bastion", "mysql.internal", 3306);
        let a2 = TunnelKey::new("bastion", "pg.internal", 5432);
        assert_ne!(a, b);
        assert_eq!(a, a2);
    }

    #[test]
    fn refcount_teardown_after_last_connection() {
        let mut t = ClaimProbe::new();
        t.add_claim(TunnelClaim::Connection("c1".into()));
        t.add_claim(TunnelClaim::Connection("c2".into()));
        assert!(!t.remove_claim(&TunnelClaim::Connection("c1".into())));
        assert!(t.remove_claim(&TunnelClaim::Connection("c2".into())));
    }

    #[test]
    fn standalone_keeps_tunnel_alive_without_connections() {
        let mut t = ClaimProbe::new();
        t.add_claim(TunnelClaim::Standalone);
        t.add_claim(TunnelClaim::Connection("c1".into()));
        assert!(!t.remove_claim(&TunnelClaim::Connection("c1".into())));
        assert!(t.remove_claim(&TunnelClaim::Standalone));
    }

    #[test]
    fn target_from_db_url_defaults_postgres_port() {
        let (host, port) = target_from_db_url("postgres://u@db.example/app").unwrap();
        assert_eq!(host, "db.example");
        assert_eq!(port, 5432);
    }

    #[test]
    fn rewrite_url_via_localhost_sets_port() {
        let out = rewrite_url_via_localhost("postgres://u:p@db.example:5432/app", 15432).unwrap();
        let parsed = url::Url::parse(&out).unwrap();
        assert_eq!(parsed.host_str(), Some("127.0.0.1"));
        assert_eq!(parsed.port(), Some(15432));
    }
}
